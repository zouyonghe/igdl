use igdl::browser::Browser;
use igdl::error::IgdlError;
use igdl::gallerydl::{
    ExtractedMediaItem, build_media_download_command, build_media_download_command_with_ytdlp,
    build_media_extraction_command, parse_gallerydl_media_items, resolve_gallerydl_binary,
};
use igdl::paths::{managed_binary_path_from, managed_gallerydl_binary_path_from};
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
    let progress_template = "download:__IGDL_PROGRESS__ percent=%(progress._percent_str)s downloaded_bytes=%(progress.downloaded_bytes)s total_bytes=%(progress.total_bytes)s speed=%(progress.speed)s eta=%(progress.eta)s";
    let cmd = build_download_command(
        Path::new("/usr/local/bin/yt-dlp"),
        Browser::Chrome,
        url,
        Path::new("/tmp/instagram"),
    );

    assert_eq!(cmd.get_program(), OsStr::new("/usr/local/bin/yt-dlp"));

    let args: Vec<_> = cmd.get_args().map(|arg| arg.to_os_string()).collect();
    let progress_template_arg = args
        .windows(2)
        .find(|window| window[0].as_os_str() == OsStr::new("--progress-template"))
        .map(|window| window[1].to_string_lossy().into_owned())
        .expect("yt-dlp command should include --progress-template");

    assert!(progress_template_arg.contains("__IGDL_PROGRESS__"));
    assert_eq!(
        args,
        vec![
            OsString::from("--cookies-from-browser"),
            OsString::from("chrome"),
            OsString::from("--quiet"),
            OsString::from("--progress"),
            OsString::from("--progress-template"),
            OsString::from(progress_template),
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
fn parses_gallerydl_image_file_events() {
    let stdout = concat!(
        "[1, \"https://www.instagram.com/p/DWWJVEdgSjW/\", {\"num\": 1}]\n",
        "[3, \"https://cdn.example.com/post-1.jpg\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1, \"_http_headers\": {\"User-Agent\": \"Example\"}}]\n"
    );

    let items = parse_gallerydl_media_items(stdout).unwrap();

    assert_eq!(
        items,
        vec![ExtractedMediaItem {
            url: "https://cdn.example.com/post-1.jpg".to_string(),
            extension: "jpg".to_string(),
            description: Some("Weekend dump".to_string()),
            shortcode: "DWWJVEdgSjW".to_string(),
            index: 1,
            http_headers: vec![("User-Agent".to_string(), "Example".to_string())],
        }]
    );
}

#[test]
fn preserves_gallerydl_carousel_image_order() {
    let stdout = concat!(
        "[3, \"https://cdn.example.com/post-1.jpg\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}]\n",
        "[3, \"https://cdn.example.com/post-2.jpg\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 2}]\n"
    );

    let items = parse_gallerydl_media_items(stdout).unwrap();

    assert_eq!(
        items.iter().map(|item| item.index).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        items
            .iter()
            .map(|item| item.url.as_str())
            .collect::<Vec<_>>(),
        vec![
            "https://cdn.example.com/post-1.jpg",
            "https://cdn.example.com/post-2.jpg",
        ]
    );
}

#[test]
fn parses_gallerydl_json_array_output() {
    let stdout = r#"[
  [1, "https://www.instagram.com/p/DWWJVEdgSjW/", {"num": 1}],
  [3, "https://cdn.example.com/post-1.jpg", {"extension": "jpg", "description": "Weekend dump", "post_shortcode": "DWWJVEdgSjW", "num": 1}],
  [3, "https://cdn.example.com/post-2.jpg", {"extension": "jpg", "description": "Weekend dump", "post_shortcode": "DWWJVEdgSjW", "num": 2}]
]"#;

    let items = parse_gallerydl_media_items(stdout).unwrap();

    assert_eq!(
        items,
        vec![
            ExtractedMediaItem {
                url: "https://cdn.example.com/post-1.jpg".to_string(),
                extension: "jpg".to_string(),
                description: Some("Weekend dump".to_string()),
                shortcode: "DWWJVEdgSjW".to_string(),
                index: 1,
                http_headers: Vec::new(),
            },
            ExtractedMediaItem {
                url: "https://cdn.example.com/post-2.jpg".to_string(),
                extension: "jpg".to_string(),
                description: Some("Weekend dump".to_string()),
                shortcode: "DWWJVEdgSjW".to_string(),
                index: 2,
                http_headers: Vec::new(),
            },
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

#[test]
fn builds_gallerydl_command_with_browser_cookies_and_json_output() {
    let url = "https://www.instagram.com/p/abc123/";
    let cmd = build_media_extraction_command(
        Path::new("/usr/local/bin/gallery-dl"),
        Browser::Chrome,
        url,
    );

    assert_eq!(cmd.get_program(), OsStr::new("/usr/local/bin/gallery-dl"));

    let args: Vec<_> = cmd.get_args().map(|arg| arg.to_os_string()).collect();
    assert_eq!(
        args,
        vec![
            OsString::from("--cookies-from-browser"),
            OsString::from("chrome"),
            OsString::from("-j"),
            OsString::from(url),
        ]
    );
}

#[test]
fn builds_gallerydl_download_command_with_ytdlp_directory_on_path() {
    let ytdlp_dir = tempdir().unwrap();
    let ytdlp_path = ytdlp_dir.path().join("yt-dlp");
    std::fs::write(&ytdlp_path, b"").unwrap();

    let cmd = build_media_download_command_with_ytdlp(
        Path::new("/usr/local/bin/gallery-dl"),
        Browser::Edge,
        "https://www.instagram.com/p/abc123/",
        Path::new("/tmp/instagram/.igdl-gallerydl-download"),
        Some(&ytdlp_path),
    );

    let path_env = cmd
        .get_envs()
        .find(|(key, _)| *key == OsStr::new("PATH"))
        .and_then(|(_, value)| value)
        .expect("PATH should be set for gallery-dl command");

    let path_entries = std::env::split_paths(path_env).collect::<Vec<_>>();
    assert_eq!(path_entries.first(), Some(&ytdlp_dir.path().to_path_buf()));
}

#[test]
fn builds_gallerydl_download_command_with_temp_dir_and_filename_template() {
    let url = "https://www.instagram.com/p/abc123/";
    let cmd = build_media_download_command(
        Path::new("/usr/local/bin/gallery-dl"),
        Browser::Edge,
        url,
        Path::new("/tmp/instagram/.igdl-gallerydl-download"),
    );

    assert_eq!(cmd.get_program(), OsStr::new("/usr/local/bin/gallery-dl"));

    let args: Vec<_> = cmd.get_args().map(|arg| arg.to_os_string()).collect();
    assert_eq!(
        args,
        vec![
            OsString::from("--cookies-from-browser"),
            OsString::from("edge"),
            OsString::from("-D"),
            OsString::from("/tmp/instagram/.igdl-gallerydl-download"),
            OsString::from("-f"),
            OsString::from("{post_shortcode}_{num:>02}.{extension}"),
            OsString::from(url),
        ]
    );
}

#[test]
fn resolves_gallerydl_from_path_before_managed_cache() {
    let _lock = PATH_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let bin_dir = tempdir().unwrap();
    let path_binary = bin_dir.path().join(gallerydl_binary_name());
    let managed_binary = managed_gallerydl_binary_path_from(home.path());

    write_executable(&path_binary);
    write_executable(&managed_binary);

    let _guard = PathGuard::set(bin_dir.path());
    let resolved = resolve_gallerydl_binary(home.path()).unwrap();

    assert_eq!(resolved, path_binary);
}

#[test]
fn uses_gallerydl_managed_cache_when_path_lookup_misses() {
    let _lock = PATH_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let empty_path = tempdir().unwrap();
    let managed_binary = managed_gallerydl_binary_path_from(home.path());

    write_executable(&managed_binary);

    let _guard = PathGuard::set(empty_path.path());
    let resolved = resolve_gallerydl_binary(home.path()).unwrap();

    assert_eq!(resolved, managed_binary);
}

#[test]
fn returns_bootstrap_error_when_gallerydl_is_unavailable() {
    let _lock = PATH_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let empty_path = tempdir().unwrap();
    let _guard = PathGuard::set(empty_path.path());

    let err = resolve_gallerydl_binary(home.path()).unwrap_err();

    match err {
        IgdlError::GalleryDlBootstrap(message) => {
            assert_eq!(message, "gallery-dl not found on PATH or managed cache")
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

fn gallerydl_binary_name() -> &'static str {
    if cfg!(windows) {
        "gallery-dl.exe"
    } else {
        "gallery-dl"
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
