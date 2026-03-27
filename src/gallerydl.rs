use crate::browser::Browser;
use crate::error::IgdlError;
use crate::media::build_media_filename;
use crate::paths::{managed_gallerydl_binary_path_from, managed_gallerydl_venv_dir_from};
use crate::ytdlp::download_release_asset;
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const GALLERYDL_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(30);
const GALLERYDL_DOWNLOAD_FILENAME_TEMPLATE: &str = "{post_shortcode}_{num:>02}.{extension}";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExtractedMediaItem {
    pub url: String,
    pub extension: String,
    pub description: Option<String>,
    pub shortcode: String,
    pub index: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct MediaDownloadRequest<'a> {
    pub binary: &'a Path,
    pub browser: Browser,
    pub url: &'a str,
    pub items: &'a [ExtractedMediaItem],
    pub output_dir: &'a Path,
    pub ytdlp_binary: Option<&'a Path>,
    pub verbose: bool,
}

#[derive(Clone, Copy)]
struct MediaDownloadExecution<'a> {
    request: MediaDownloadRequest<'a>,
    temp_dir: &'a Path,
}

pub fn resolve_gallerydl_binary(home: &Path) -> Result<PathBuf, IgdlError> {
    if let Ok(path) = which::which("gallery-dl") {
        return Ok(path);
    }

    let managed = managed_gallerydl_binary_path_from(home);
    if managed.is_file() {
        return Ok(managed);
    }

    Err(IgdlError::GalleryDlBootstrap(
        "gallery-dl not found on PATH or managed cache".to_owned(),
    ))
}

pub fn bootstrap_managed_gallerydl(home: &Path) -> Result<PathBuf, IgdlError> {
    let managed = managed_gallerydl_binary_path_from(home);
    if managed.is_file() {
        return Ok(managed);
    }

    match std::env::consts::OS {
        "windows" => bootstrap_managed_gallerydl_windows(home),
        _ => bootstrap_managed_gallerydl_venv(home),
    }
}

pub fn build_media_extraction_command(binary: &Path, browser: Browser, url: &str) -> Command {
    build_media_extraction_command_with_ytdlp(binary, browser, url, None)
}

pub fn build_media_extraction_command_with_ytdlp(
    binary: &Path,
    browser: Browser,
    url: &str,
    ytdlp_binary: Option<&Path>,
) -> Command {
    let mut cmd = Command::new(binary);
    cmd.arg("--cookies-from-browser")
        .arg(browser.as_ytdlp_arg())
        .arg("-j")
        .arg(url);
    configure_command_ytdlp_path(&mut cmd, ytdlp_binary);
    cmd
}

pub fn build_media_download_command(
    binary: &Path,
    browser: Browser,
    url: &str,
    output_dir: &Path,
) -> Command {
    build_media_download_command_with_ytdlp(binary, browser, url, output_dir, None)
}

pub fn build_media_download_command_with_ytdlp(
    binary: &Path,
    browser: Browser,
    url: &str,
    output_dir: &Path,
    ytdlp_binary: Option<&Path>,
) -> Command {
    let mut cmd = Command::new(binary);
    cmd.arg("--cookies-from-browser")
        .arg(browser.as_ytdlp_arg())
        .arg("-D")
        .arg(output_dir)
        .arg("-f")
        .arg(GALLERYDL_DOWNLOAD_FILENAME_TEMPLATE)
        .arg(url);
    configure_command_ytdlp_path(&mut cmd, ytdlp_binary);
    cmd
}

pub fn extract_media_items(
    binary: &Path,
    browser: Browser,
    url: &str,
    ytdlp_binary: Option<&Path>,
) -> Result<Vec<ExtractedMediaItem>, String> {
    let output = build_media_extraction_command_with_ytdlp(binary, browser, url, ytdlp_binary)
        .output()
        .map_err(|err| format!("failed to run gallery-dl: {err}"))?;

    if !output.status.success() {
        return Err(describe_command_failure(&output.status, &output.stderr));
    }

    parse_gallerydl_media_items(&String::from_utf8_lossy(&output.stdout))
        .map_err(|err| format!("failed to parse gallery-dl output: {err}"))
}

pub fn download_media_items(
    binary: &Path,
    browser: Browser,
    url: &str,
    items: &[ExtractedMediaItem],
    output_dir: &Path,
) -> Result<Vec<PathBuf>, IgdlError> {
    download_media_items_with_progress(
        MediaDownloadRequest {
            binary,
            browser,
            url,
            items,
            output_dir,
            ytdlp_binary: None,
            verbose: false,
        },
        |_| {},
    )
}

pub fn download_media_items_with_progress<F>(
    request: MediaDownloadRequest<'_>,
    mut on_progress: F,
) -> Result<Vec<PathBuf>, IgdlError>
where
    F: FnMut(usize),
{
    let mut ordered = request.items.to_vec();
    ordered.sort_by_key(|item| item.index);

    let temp_dir = temp_media_download_dir(request.output_dir);
    reset_temp_media_download_dir(&temp_dir)?;

    let result = (|| -> Result<Vec<PathBuf>, IgdlError> {
        let download_result = run_media_download(
            MediaDownloadExecution {
                request: MediaDownloadRequest {
                    items: &ordered,
                    ..request
                },
                temp_dir: &temp_dir,
            },
            &mut on_progress,
        );
        let finalized = finalize_downloaded_media_files(&temp_dir, request.output_dir, &ordered)?;

        resolve_media_download_result(download_result, finalized)
    })();

    let cleanup_result = cleanup_temp_media_download_dir(&temp_dir);

    match (result, cleanup_result) {
        (Ok(paths), Ok(())) => Ok(paths),
        (Ok(_), Err(err)) => Err(err),
        (Err(err), Ok(())) => Err(err),
        (Err(err), Err(_)) => Err(err),
    }
}

struct FinalizedMediaFiles {
    paths: Vec<PathBuf>,
    missing: usize,
}

fn finalize_downloaded_media_files(
    temp_dir: &Path,
    output_dir: &Path,
    ordered: &[ExtractedMediaItem],
) -> Result<FinalizedMediaFiles, IgdlError> {
    let use_index = ordered.len() > 1;
    let mut paths = Vec::with_capacity(ordered.len());
    let mut missing = 0;

    for item in ordered {
        let source = temp_dir.join(intermediate_media_filename(item));
        if !source.is_file() {
            missing += 1;
            continue;
        }

        let path = final_media_path(output_dir, item, use_index);
        if path.is_file() {
            std::fs::remove_file(&path)?;
        }
        std::fs::rename(&source, &path)?;
        paths.push(path);
    }

    Ok(FinalizedMediaFiles { paths, missing })
}

fn resolve_media_download_result(
    download_result: Result<(), IgdlError>,
    finalized: FinalizedMediaFiles,
) -> Result<Vec<PathBuf>, IgdlError> {
    let downloaded = finalized.paths.len();
    let failed = finalized.missing;

    if downloaded > 0 && failed > 0 {
        return Err(IgdlError::PostMediaDownloadPartial {
            downloaded,
            failed,
            total: downloaded + failed,
        });
    }

    match download_result {
        Ok(()) if failed == 0 => Ok(finalized.paths),
        Ok(()) => Err(missing_media_files_error(failed)),
        Err(err) => Err(err),
    }
}

fn final_media_path(output_dir: &Path, item: &ExtractedMediaItem, use_index: bool) -> PathBuf {
    output_dir.join(build_media_filename(
        item.description.as_deref().unwrap_or("media"),
        &item.shortcode,
        use_index.then_some(item.index),
        &item.extension,
    ))
}

fn missing_media_files_error(failed: usize) -> IgdlError {
    IgdlError::MediaDownload(format!(
        "gallery-dl did not produce {failed} expected media file{}",
        if failed == 1 { "" } else { "s" }
    ))
}

pub fn parse_gallerydl_media_items(
    stdout: &str,
) -> Result<Vec<ExtractedMediaItem>, serde_json::Error> {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Ok(Vec::new());
    }

    match serde_json::from_str::<Value>(stdout) {
        Ok(value) => Ok(collect_media_items(json_media_events(&value))),
        Err(error) => match parse_gallerydl_media_items_from_lines(stdout) {
            Ok(items) => Ok(items),
            Err(_) => Err(error),
        },
    }
}

fn parse_gallerydl_media_items_from_lines(
    stdout: &str,
) -> Result<Vec<ExtractedMediaItem>, serde_json::Error> {
    let mut events = Vec::new();

    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        events.push(serde_json::from_str(line)?);
    }

    Ok(collect_media_items(events.iter()))
}

fn bootstrap_managed_gallerydl_windows(home: &Path) -> Result<PathBuf, IgdlError> {
    let url = "https://github.com/mikf/gallery-dl/releases/latest/download/gallery-dl.exe";
    let client = Client::builder()
        .timeout(GALLERYDL_BOOTSTRAP_TIMEOUT)
        .build()
        .map_err(|err| {
            IgdlError::GalleryDlBootstrap(format!("failed to create download client: {err}"))
        })?;
    let bytes = download_release_asset(&client, url).map_err(|err| match err {
        IgdlError::YtDlpBootstrap(message) => IgdlError::GalleryDlBootstrap(message),
        other => other,
    })?;

    install_managed_gallerydl(home, &bytes)
}

fn bootstrap_managed_gallerydl_venv(home: &Path) -> Result<PathBuf, IgdlError> {
    let venv_dir = managed_gallerydl_venv_dir_from(home);
    if let Some(parent) = venv_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let python = resolve_python_binary()?;
    run_bootstrap_command(
        Command::new(&python).arg("-m").arg("venv").arg(&venv_dir),
        "failed to create gallery-dl runtime",
    )?;

    let venv_python = venv_python_path(&venv_dir);
    run_bootstrap_command(
        Command::new(&venv_python)
            .arg("-m")
            .arg("pip")
            .arg("install")
            .arg("--disable-pip-version-check")
            .arg("gallery-dl"),
        "failed to install gallery-dl",
    )?;

    let managed = managed_gallerydl_binary_path_from(home);
    if managed.is_file() {
        Ok(managed)
    } else {
        Err(IgdlError::GalleryDlBootstrap(
            "gallery-dl did not appear after bootstrap".to_owned(),
        ))
    }
}

fn install_managed_gallerydl(home: &Path, bytes: &[u8]) -> Result<PathBuf, IgdlError> {
    let managed = managed_gallerydl_binary_path_from(home);
    let Some(parent) = managed.parent() else {
        return Err(IgdlError::GalleryDlBootstrap(
            "managed gallery-dl path has no parent directory".to_owned(),
        ));
    };

    std::fs::create_dir_all(parent)?;
    let temporary = temporary_download_path(&managed);
    let write_result = (|| -> Result<(), IgdlError> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        std::io::Write::write_all(&mut file, bytes)?;
        file.sync_all()?;

        #[cfg(unix)]
        {
            let mut permissions = file.metadata()?.permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&temporary, permissions)?;
        }

        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&temporary);
        return Err(err);
    }

    std::fs::rename(&temporary, &managed)?;
    Ok(managed)
}

fn resolve_python_binary() -> Result<PathBuf, IgdlError> {
    which::which("python3")
        .or_else(|_| which::which("python"))
        .map_err(|_| {
            IgdlError::GalleryDlBootstrap(
                "python3 or python is required to bootstrap gallery-dl".to_owned(),
            )
        })
}

fn venv_python_path(venv_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        venv_dir.join("Scripts").join("python.exe")
    } else {
        venv_dir.join("bin").join("python")
    }
}

fn run_bootstrap_command(cmd: &mut Command, prefix: &str) -> Result<(), IgdlError> {
    let output = cmd
        .output()
        .map_err(|err| IgdlError::GalleryDlBootstrap(format!("{prefix}: {err}")))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        return Err(IgdlError::GalleryDlBootstrap(format!(
            "{prefix}: command exited unsuccessfully"
        )));
    }

    Err(IgdlError::GalleryDlBootstrap(format!("{prefix}: {stderr}")))
}

fn run_media_download(
    execution: MediaDownloadExecution<'_>,
    on_progress: &mut dyn FnMut(usize),
) -> Result<(), IgdlError> {
    let request = execution.request;
    let temp_dir = execution.temp_dir;
    let mut child = build_media_download_command_with_ytdlp(
        request.binary,
        request.browser,
        request.url,
        temp_dir,
        request.ytdlp_binary,
    )
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|err| IgdlError::MediaDownload(format!("failed to run gallery-dl: {err}")))?;

    let stdout = child
        .stdout
        .take()
        .expect("gallery-dl stdout should be piped");
    let stderr = child
        .stderr
        .take()
        .expect("gallery-dl stderr should be piped");

    let (sender, receiver) = mpsc::channel();
    spawn_output_reader(OutputStream::Stdout, stdout, sender.clone());
    spawn_output_reader(OutputStream::Stderr, stderr, sender);

    let expected_files = request
        .items
        .iter()
        .map(intermediate_media_filename)
        .collect::<Vec<_>>();
    let mut completed = HashSet::new();
    let mut stderr_lines = Vec::new();

    for output_line in receiver {
        if request.verbose {
            eprintln!("{}", output_line.line);
        }

        if matches!(output_line.stream, OutputStream::Stderr) {
            stderr_lines.push(output_line.line.clone());
        }

        maybe_emit_completed_media_progress(
            &output_line.line,
            temp_dir,
            &expected_files,
            &mut completed,
            on_progress,
        );
    }

    let status = child
        .wait()
        .map_err(|err| IgdlError::MediaDownload(format!("failed to wait for gallery-dl: {err}")))?;
    if status.success() {
        return Ok(());
    }

    let stderr = stderr_lines.join("\n");

    Err(IgdlError::MediaDownload(describe_command_failure(
        &status,
        stderr.as_bytes(),
    )))
}

#[derive(Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
}

struct OutputLine {
    stream: OutputStream,
    line: String,
}

fn spawn_output_reader<R>(stream: OutputStream, reader: R, sender: mpsc::Sender<OutputLine>)
where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else {
                break;
            };

            if sender.send(OutputLine { stream, line }).is_err() {
                break;
            }
        }
    });
}

fn maybe_emit_completed_media_progress(
    line: &str,
    temp_dir: &Path,
    expected_files: &[String],
    completed: &mut HashSet<String>,
    on_progress: &mut dyn FnMut(usize),
) {
    for expected_file in expected_files {
        if completed.contains(expected_file) || !line.contains(expected_file) {
            continue;
        }

        if temp_dir.join(expected_file).is_file() {
            completed.insert(expected_file.clone());
            on_progress(completed.len());
            break;
        }
    }
}

fn configure_command_ytdlp_path(cmd: &mut Command, ytdlp_binary: Option<&Path>) {
    let Some(path) = ytdlp_binary.and_then(gallerydl_path_with_ytdlp) else {
        return;
    };

    cmd.env("PATH", path);
}

fn gallerydl_path_with_ytdlp(ytdlp_binary: &Path) -> Option<std::ffi::OsString> {
    let ytdlp_dir = ytdlp_binary.parent()?;
    let mut paths = vec![ytdlp_dir.to_path_buf()];
    if let Some(current_path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current_path));
    }

    std::env::join_paths(&paths)
        .ok()
        .or_else(|| std::env::join_paths([ytdlp_dir]).ok())
}

fn temp_media_download_dir(output_dir: &Path) -> PathBuf {
    output_dir.join(".igdl-gallerydl-download")
}

fn reset_temp_media_download_dir(temp_dir: &Path) -> Result<(), IgdlError> {
    if temp_dir.exists() {
        std::fs::remove_dir_all(temp_dir)?;
    }
    std::fs::create_dir_all(temp_dir)?;
    Ok(())
}

fn cleanup_temp_media_download_dir(temp_dir: &Path) -> Result<(), IgdlError> {
    if temp_dir.exists() {
        std::fs::remove_dir_all(temp_dir)?;
    }
    Ok(())
}

fn intermediate_media_filename(item: &ExtractedMediaItem) -> String {
    format!("{}_{:02}.{}", item.shortcode, item.index, item.extension)
}

fn describe_command_failure(status: &ExitStatus, stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_owned();
    }

    match status.code() {
        Some(code) => format!("gallery-dl exited with status {code}"),
        None => "gallery-dl exited unsuccessfully".to_owned(),
    }
}

fn json_media_events(value: &Value) -> JsonMediaEvents<'_> {
    match value {
        Value::Array(events) if extract_media_item(value, 1).is_none() => {
            JsonMediaEvents::Many(events.iter())
        }
        _ => JsonMediaEvents::One(std::iter::once(value)),
    }
}

enum JsonMediaEvents<'a> {
    One(std::iter::Once<&'a Value>),
    Many(std::slice::Iter<'a, Value>),
}

impl<'a> Iterator for JsonMediaEvents<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            JsonMediaEvents::One(iter) => iter.next(),
            JsonMediaEvents::Many(iter) => iter.next(),
        }
    }
}

fn collect_media_items<'a>(events: impl IntoIterator<Item = &'a Value>) -> Vec<ExtractedMediaItem> {
    let mut items = Vec::new();

    for event in events {
        if let Some(item) = extract_media_item(event, items.len() + 1) {
            items.push(item);
        }
    }

    items
}

fn extract_media_item(event: &Value, fallback_index: usize) -> Option<ExtractedMediaItem> {
    let event = event.as_array()?;
    if event.len() != 3 || event[0].as_u64()? != 3 {
        return None;
    }

    let url = event[1].as_str()?.to_owned();
    let metadata = event[2].as_object()?;
    let extension = metadata.get("extension")?.as_str()?.to_owned();
    let shortcode = metadata.get("post_shortcode")?.as_str()?.to_owned();
    if extension.is_empty() || shortcode.is_empty() {
        return None;
    }

    let description = metadata
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let index = metadata
        .get("num")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(fallback_index);

    Some(ExtractedMediaItem {
        url,
        extension,
        description,
        shortcode,
        index,
    })
}

fn temporary_download_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .expect("managed gallery-dl path should include a file name")
        .to_os_string();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    name.push(format!(".tmp-{}-{nonce}", std::process::id()));
    path.with_file_name(name)
}
