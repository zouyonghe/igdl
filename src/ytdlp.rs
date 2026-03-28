use crate::browser::Browser;
use crate::error::IgdlError;
use crate::paths::managed_binary_path_from;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const YTDLP_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const YTDLP_STREAM_POLL_INTERVAL: Duration = Duration::from_millis(50);
const IGDL_PROGRESS_PREFIX: &str = "__IGDL_PROGRESS__";

#[derive(Clone, Debug, PartialEq)]
pub struct YtDlpProgressUpdate {
    pub percentage: Option<f32>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_second: Option<u64>,
    pub eta: Option<Duration>,
}

#[derive(Debug)]
enum YtDlpStreamEvent {
    DownloadedPath(PathBuf),
    Progress(YtDlpProgressUpdate),
    StderrLine(String),
}

pub fn platform_asset_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") | ("macos", "x86_64") => Some("yt-dlp_macos"),
        _ => None,
    }
}

pub fn bootstrap_managed_ytdlp(home: &Path) -> Result<PathBuf, IgdlError> {
    let managed = managed_binary_path_from(home);
    if managed.is_file() {
        return Ok(managed);
    }

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let asset = platform_asset_name(os, arch).ok_or_else(|| {
        IgdlError::YtDlpBootstrap(format!("unsupported yt-dlp platform: {os}/{arch}"))
    })?;
    let url = format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{asset}");
    let client = reqwest::blocking::Client::builder()
        .timeout(YTDLP_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|err| {
            IgdlError::YtDlpBootstrap(format!("failed to create download client: {err}"))
        })?;
    let bytes = download_release_asset(&client, &url)?;

    install_managed_ytdlp(home, &bytes)
}

pub fn download_release_asset(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<Vec<u8>, IgdlError> {
    let response = client
        .get(url)
        .send()
        .map_err(|err| IgdlError::YtDlpBootstrap(format!("failed to download {url}: {err}")))?;
    let response = response
        .error_for_status()
        .map_err(|err| IgdlError::YtDlpBootstrap(format!("failed to download {url}: {err}")))?;
    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|err| IgdlError::YtDlpBootstrap(format!("failed to read {url}: {err}")))
}

pub fn install_managed_ytdlp(home: &Path, bytes: &[u8]) -> Result<PathBuf, IgdlError> {
    let managed = managed_binary_path_from(home);
    let Some(parent) = managed.parent() else {
        return Err(IgdlError::YtDlpBootstrap(
            "managed yt-dlp path has no parent directory".to_owned(),
        ));
    };

    std::fs::create_dir_all(parent)?;
    let temporary = temporary_download_path(&managed);
    let write_result = (|| -> Result<(), IgdlError> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
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

pub fn resolve_ytdlp_binary(home: &Path) -> Result<PathBuf, IgdlError> {
    if let Ok(path) = which::which("yt-dlp") {
        return Ok(path);
    }

    let managed = managed_binary_path_from(home);
    if managed.is_file() {
        return Ok(managed);
    }

    Err(IgdlError::YtDlpBootstrap(
        "yt-dlp not found on PATH or managed cache".to_owned(),
    ))
}

pub fn build_download_command(
    binary: &Path,
    browser: Browser,
    url: &str,
    output_dir: &Path,
) -> Command {
    let template = output_dir.join("%(title)s [%(id)s].%(ext)s");
    let mut cmd = Command::new(binary);
    cmd.arg("--cookies-from-browser")
        .arg(browser.as_ytdlp_arg())
        .arg("--quiet")
        .arg("--progress")
        .arg("--progress-template")
        .arg(download_progress_template())
        .arg("--newline")
        .arg("--print")
        .arg("after_move:filepath")
        .arg("-o")
        .arg(template)
        .arg(url);
    cmd
}

fn download_progress_template() -> &'static str {
    "download:__IGDL_PROGRESS__ percent=%(progress._percent_str)s downloaded_bytes=%(progress.downloaded_bytes)s total_bytes=%(progress.total_bytes)s speed=%(progress.speed)s eta=%(progress.eta)s"
}

pub(crate) fn run_download_with_progress<F>(
    binary: &Path,
    browser: Browser,
    url: &str,
    output_dir: &Path,
    mut on_progress: F,
) -> Result<Vec<PathBuf>, IgdlError>
where
    F: FnMut(YtDlpProgressUpdate),
{
    let mut child = build_download_command(binary, browser, url, output_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| {
        IgdlError::MediaDownload("yt-dlp stdout pipe was not captured".to_owned())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        IgdlError::MediaDownload("yt-dlp stderr pipe was not captured".to_owned())
    })?;

    let (sender, receiver) = mpsc::channel();
    let stdout_handle = std::thread::spawn({
        let sender = sender.clone();
        move || forward_stdout_events(stdout, sender)
    });
    let stderr_handle = std::thread::spawn(move || forward_stderr_lines(stderr, sender));

    let mut paths = Vec::new();
    let mut stderr_message = String::new();
    let mut status = None;

    loop {
        match receiver.recv_timeout(YTDLP_STREAM_POLL_INTERVAL) {
            Ok(event) => match event {
                YtDlpStreamEvent::DownloadedPath(path) => paths.push(path),
                YtDlpStreamEvent::Progress(progress) => on_progress(progress),
                YtDlpStreamEvent::StderrLine(line) => {
                    append_stderr_line(&mut stderr_message, &line)
                }
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if status.is_none() {
            status = child.try_wait()?;
        }
    }

    let status = match status {
        Some(status) => status,
        None => child.wait()?,
    };

    join_reader_thread(stdout_handle)?;
    join_reader_thread(stderr_handle)?;

    if !status.success() {
        return Err(IgdlError::MediaDownload(describe_command_failure(
            &status,
            stderr_message.as_bytes(),
        )));
    }

    Ok(paths)
}

pub fn parse_downloaded_paths(stdout: &str) -> Vec<PathBuf> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn parse_progress_line(line: &str) -> Option<YtDlpProgressUpdate> {
    let remainder = line.strip_prefix(IGDL_PROGRESS_PREFIX)?;

    parse_template_progress_line(remainder)
}

fn parse_template_progress_line(line: &str) -> Option<YtDlpProgressUpdate> {
    let mut percentage = None;
    let mut downloaded_bytes = None;
    let mut total_bytes = None;
    let mut speed_bytes_per_second = None;
    let mut eta = None;
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let mut index = 0;

    while index < tokens.len() {
        let Some((key, mut value)) = tokens[index].split_once('=') else {
            index += 1;
            continue;
        };

        if value.is_empty() && index + 1 < tokens.len() && !tokens[index + 1].contains('=') {
            value = tokens[index + 1];
            index += 1;
        }

        match key {
            "percent" => percentage = parse_template_percentage(value),
            "downloaded" | "downloaded_bytes" => downloaded_bytes = parse_template_u64(value),
            "total" | "total_bytes" => total_bytes = parse_template_u64(value),
            "speed" | "speed_bytes_per_second" => {
                speed_bytes_per_second = parse_template_u64(value)
            }
            "eta" => eta = parse_template_u64(value).map(Duration::from_secs),
            _ => {}
        }

        index += 1;
    }

    if percentage.is_none()
        && downloaded_bytes.is_none()
        && total_bytes.is_none()
        && speed_bytes_per_second.is_none()
        && eta.is_none()
    {
        return None;
    }

    Some(YtDlpProgressUpdate {
        percentage,
        downloaded_bytes,
        total_bytes,
        speed_bytes_per_second,
        eta,
    })
}

fn parse_template_percentage(value: &str) -> Option<f32> {
    let value = normalize_template_value(value)?;
    let value = value.strip_suffix('%').unwrap_or(value);
    value.parse().ok()
}

fn parse_template_u64(value: &str) -> Option<u64> {
    let value = normalize_template_value(value)?;
    value.parse::<u64>().ok().or_else(|| {
        value
            .parse::<f64>()
            .ok()
            .map(|parsed| parsed.round() as u64)
    })
}

fn normalize_template_value(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("NA")
        || value.eq_ignore_ascii_case("unavailable")
    {
        return None;
    }

    Some(value)
}

pub fn describe_command_failure(status: &ExitStatus, stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_owned();
    }

    match status.code() {
        Some(code) => format!("yt-dlp exited with status {code}"),
        None => "yt-dlp exited unsuccessfully".to_owned(),
    }
}

fn temporary_download_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .expect("managed yt-dlp path should include a file name")
        .to_os_string();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    name.push(format!(".tmp-{}-{nonce}", std::process::id()));
    path.with_file_name(name)
}

fn forward_stdout_events<R>(reader: R, sender: Sender<YtDlpStreamEvent>) -> std::io::Result<()>
where
    R: Read,
{
    read_lossy_lines(reader, |line| {
        let line = line.trim_end_matches('\r');
        if let Some(progress) = parse_progress_line(line) {
            let _ = sender.send(YtDlpStreamEvent::Progress(progress));
        } else if !line.trim().is_empty() {
            let _ = sender.send(YtDlpStreamEvent::DownloadedPath(PathBuf::from(line)));
        }
    })
}

fn forward_stderr_lines<R>(reader: R, sender: Sender<YtDlpStreamEvent>) -> std::io::Result<()>
where
    R: Read,
{
    read_lossy_lines(reader, |line| {
        let line = line.trim_end_matches('\r');
        if !line.trim().is_empty() {
            let _ = sender.send(YtDlpStreamEvent::StderrLine(line.to_owned()));
        }
    })
}

fn append_stderr_line(message: &mut String, line: &str) {
    if !message.is_empty() {
        message.push('\n');
    }
    message.push_str(line);
}

fn read_lossy_lines<R, F>(reader: R, mut on_line: F) -> std::io::Result<()>
where
    R: Read,
    F: FnMut(String),
{
    let mut reader = BufReader::new(reader);
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        if reader.read_until(b'\n', &mut buffer)? == 0 {
            break;
        }

        if buffer.last() == Some(&b'\n') {
            buffer.pop();
        }

        on_line(String::from_utf8_lossy(&buffer).into_owned());
    }

    Ok(())
}

fn join_reader_thread<T>(
    handle: std::thread::JoinHandle<std::io::Result<T>>,
) -> Result<T, IgdlError> {
    match handle.join() {
        Ok(result) => result.map_err(IgdlError::Io),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}
