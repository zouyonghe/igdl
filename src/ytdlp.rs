use crate::browser::Browser;
use crate::error::IgdlError;
use crate::paths::managed_binary_path_from;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const YTDLP_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

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
        .arg("--no-progress")
        .arg("--newline")
        .arg("--print")
        .arg("after_move:filepath")
        .arg("-o")
        .arg(template)
        .arg(url);
    cmd
}

pub fn parse_downloaded_paths(stdout: &str) -> Vec<PathBuf> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(PathBuf::from)
        .collect()
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
