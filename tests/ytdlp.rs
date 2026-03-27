use igdl::browser::Browser;
use igdl::error::IgdlError;
use igdl::paths::managed_binary_path_from;
use igdl::ytdlp::{
    build_download_command, download_release_asset, install_managed_ytdlp, parse_downloaded_paths,
    platform_asset_name, resolve_ytdlp_binary,
};
use std::ffi::OsStr;
use std::ffi::OsString;
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tempfile::tempdir;

static PATH_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn chooses_expected_macos_asset_name() {
    assert_eq!(
        platform_asset_name("macos", "aarch64"),
        Some("yt-dlp_macos")
    );
    assert_eq!(platform_asset_name("macos", "x86_64"), Some("yt-dlp_macos"));
}

#[test]
fn returns_none_for_unsupported_platform() {
    assert_eq!(platform_asset_name("linux", "x86_64"), None);
}

#[test]
fn installs_managed_ytdlp_into_cache_and_marks_it_executable() {
    let home = tempdir().unwrap();
    let contents = b"#!/bin/sh\nexit 0\n";

    let installed = install_managed_ytdlp(home.path(), contents).unwrap();

    assert_eq!(installed, managed_binary_path_from(home.path()));
    assert_eq!(std::fs::read(&installed).unwrap(), contents);

    #[cfg(unix)]
    assert_ne!(
        std::fs::metadata(&installed).unwrap().permissions().mode() & 0o111,
        0
    );
}

#[test]
fn builds_ytdlp_command_with_browser_cookies_and_output_template() {
    let url = "https://www.instagram.com/reel/abc123/";
    let cmd = build_download_command(
        Path::new("/usr/local/bin/yt-dlp"),
        Browser::Chrome,
        url,
        Path::new("/tmp/instagram"),
    );

    assert_eq!(cmd.get_program(), OsStr::new("/usr/local/bin/yt-dlp"));

    let args: Vec<_> = cmd.get_args().map(|arg| arg.to_os_string()).collect();
    assert_eq!(
        args,
        vec![
            OsString::from("--cookies-from-browser"),
            OsString::from("chrome"),
            OsString::from("--no-progress"),
            OsString::from("--newline"),
            OsString::from("--print"),
            OsString::from("after_move:filepath"),
            OsString::from("-o"),
            OsString::from("/tmp/instagram/%(title)s [%(id)s].%(ext)s"),
            OsString::from(url),
        ]
    );
}

#[test]
fn extracts_after_move_filepaths_from_stdout() {
    let stdout = "/tmp/instagram/reel one.mp4\n/tmp/instagram/reel two.mp4\n";
    let paths = parse_downloaded_paths(stdout);
    assert_eq!(
        paths,
        vec![
            PathBuf::from("/tmp/instagram/reel one.mp4"),
            PathBuf::from("/tmp/instagram/reel two.mp4"),
        ]
    );
}

#[test]
fn skips_blank_lines_without_trimming_path_whitespace() {
    let stdout =
        "\n  /tmp/instagram/leading space.mp4\n\n/tmp/instagram/trailing space.mp4  \n   \n";
    let paths = parse_downloaded_paths(stdout);
    assert_eq!(
        paths,
        vec![
            PathBuf::from("  /tmp/instagram/leading space.mp4"),
            PathBuf::from("/tmp/instagram/trailing space.mp4  "),
        ]
    );
}

#[test]
fn download_release_asset_respects_client_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(300));
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    let started = Instant::now();
    let err = download_release_asset(&client, &format!("http://{address}/yt-dlp")).unwrap_err();

    match err {
        IgdlError::YtDlpBootstrap(message) => assert!(message.contains("failed to download")),
        other => panic!("expected bootstrap error, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(1));

    server.join().unwrap();
}

#[test]
fn uses_library_cache_for_managed_binary() {
    let home = tempdir().unwrap();
    let path = managed_binary_path_from(home.path());
    let expected = match std::env::consts::OS {
        "macos" => home
            .path()
            .join("Library")
            .join("Caches")
            .join("igdl")
            .join("yt-dlp"),
        "windows" => home
            .path()
            .join("AppData")
            .join("Local")
            .join("igdl")
            .join("yt-dlp"),
        _ => home.path().join(".cache").join("igdl").join("yt-dlp"),
    };

    assert_eq!(path, expected);
}

#[test]
fn resolves_ytdlp_from_path_before_managed_cache() {
    let _lock = PATH_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let bin_dir = tempdir().unwrap();
    let path_binary = bin_dir.path().join("yt-dlp");
    let managed_binary = managed_binary_path_from(home.path());

    write_executable(&path_binary);
    write_executable(&managed_binary);

    let _guard = PathGuard::set(bin_dir.path());
    let resolved = resolve_ytdlp_binary(home.path()).unwrap();

    assert_eq!(resolved, path_binary);
}

#[test]
fn uses_managed_cache_when_path_lookup_misses() {
    let _lock = PATH_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let empty_path = tempdir().unwrap();
    let managed_binary = managed_binary_path_from(home.path());

    write_executable(&managed_binary);

    let _guard = PathGuard::set(empty_path.path());
    let resolved = resolve_ytdlp_binary(home.path()).unwrap();

    assert_eq!(resolved, managed_binary);
}

#[test]
fn returns_bootstrap_error_when_ytdlp_is_unavailable() {
    let _lock = PATH_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let empty_path = tempdir().unwrap();
    let _guard = PathGuard::set(empty_path.path());

    let err = resolve_ytdlp_binary(home.path()).unwrap_err();

    match err {
        IgdlError::YtDlpBootstrap(message) => {
            assert_eq!(message, "yt-dlp not found on PATH or managed cache")
        }
        other => panic!("expected bootstrap error, got {other:?}"),
    }
}

fn write_executable(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"").unwrap();

    #[cfg(unix)]
    {
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }
}

struct PathGuard {
    original: Option<OsString>,
}

impl PathGuard {
    fn set(path: &Path) -> Self {
        let original = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", path);
        }
        Self { original }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        match self.original.as_ref() {
            Some(path) => unsafe {
                std::env::set_var("PATH", path);
            },
            None => unsafe {
                std::env::remove_var("PATH");
            },
        }
    }
}
