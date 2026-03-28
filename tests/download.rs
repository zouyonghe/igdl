use clap::Parser;
use igdl::browser::Browser;
use igdl::cli::CliArgs;
use igdl::download::DownloadBinaries;
use igdl::download::DownloadPlan;
use igdl::download::choose_successful_browser;
use igdl::download::choose_successful_media_extraction;
use igdl::download::execute_download_plan;
use igdl::download::execute_download_plan_with_progress;
use igdl::download::plan_download;
use igdl::error::IgdlError;
use igdl::gallerydl::ExtractedMediaItem;
use igdl::gallerydl::ImageDownloadProgressUpdate;
use igdl::gallerydl::MediaDownloadRequest;
use igdl::gallerydl::download_image_items_with_detailed_progress;
use igdl::gallerydl::download_media_items_with_progress;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
fn capture_cli_output_in_pseudo_tty(
    path: std::ffi::OsString,
    args: &[&std::ffi::OsStr],
) -> std::process::Output {
    let mut command = Command::new("script");
    command.args(pseudo_tty_invocation_args(
        env!("CARGO_BIN_EXE_igdl").as_ref(),
        args,
        cfg!(target_os = "linux"),
    ));
    command.env("PATH", path);
    command.output().unwrap()
}

#[cfg(unix)]
fn pseudo_tty_invocation_args(binary: &OsStr, args: &[&OsStr], is_linux: bool) -> Vec<OsString> {
    if is_linux {
        return vec![
            OsString::from("-q"),
            OsString::from("-e"),
            OsString::from("-c"),
            OsString::from(shell_quote_for_script_command(binary, args)),
            OsString::from("/dev/null"),
        ];
    }

    let mut command_args = vec![OsString::from("-q"), OsString::from("/dev/null")];
    command_args.push(binary.to_os_string());
    command_args.extend(args.iter().map(|arg| (*arg).to_os_string()));
    command_args
}

#[cfg(unix)]
fn shell_quote_for_script_command(binary: &OsStr, args: &[&OsStr]) -> String {
    std::iter::once(binary)
        .chain(args.iter().copied())
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(unix)]
fn shell_quote(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

#[cfg(unix)]
fn render_captured_output(output: &std::process::Output) -> String {
    normalize_captured_output(&format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

#[cfg(unix)]
fn normalize_captured_output(output: &str) -> String {
    let mut normalized = String::with_capacity(output.len());
    let mut chars = output.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' || chars.peek() != Some(&'[') {
            normalized.push(ch);
            continue;
        }

        chars.next();
        let mut parameter = String::new();
        while let Some(&next) = chars.peek() {
            if next.is_ascii_digit() || next == ';' {
                parameter.push(next);
                chars.next();
                continue;
            }

            break;
        }

        let Some(command) = chars.next() else {
            normalized.push('\u{1b}');
            normalized.push('[');
            normalized.push_str(&parameter);
            break;
        };

        match command {
            'A' => normalized.push_str(&format!(
                "<UP:{}>",
                if parameter.is_empty() {
                    "1"
                } else {
                    &parameter
                }
            )),
            'K' => normalized.push_str("<CLEAR_LINE>"),
            _ => {
                normalized.push('\u{1b}');
                normalized.push('[');
                normalized.push_str(&parameter);
                normalized.push(command);
            }
        }
    }

    normalized
}

#[derive(Clone)]
struct FakeImageRoute {
    status_code: u16,
    chunks: Vec<Vec<u8>>,
    chunk_delay: Duration,
    required_headers: Vec<(String, String)>,
}

impl FakeImageRoute {
    fn success(chunks: Vec<Vec<u8>>, chunk_delay: Duration) -> Self {
        Self {
            status_code: 200,
            chunks,
            chunk_delay,
            required_headers: Vec::new(),
        }
    }

    fn failure(status_code: u16) -> Self {
        Self {
            status_code,
            chunks: Vec::new(),
            chunk_delay: Duration::from_millis(0),
            required_headers: Vec::new(),
        }
    }

    fn requiring_header(mut self, name: &str, value: &str) -> Self {
        self.required_headers
            .push((name.to_ascii_lowercase(), value.to_owned()));
        self
    }
}

struct FakeImageBackend {
    address: std::net::SocketAddr,
    max_active_requests: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    server_thread: Option<thread::JoinHandle<()>>,
}

impl FakeImageBackend {
    fn new(routes: HashMap<String, FakeImageRoute>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();

        let address = listener.local_addr().unwrap();
        let routes = Arc::new(routes);
        let active_requests = Arc::new(AtomicUsize::new(0));
        let max_active_requests = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));

        let server_thread = thread::spawn({
            let routes = Arc::clone(&routes);
            let active_requests = Arc::clone(&active_requests);
            let max_active_requests = Arc::clone(&max_active_requests);
            let shutdown = Arc::clone(&shutdown);

            move || {
                let mut workers = Vec::new();

                while !shutdown.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let routes = Arc::clone(&routes);
                            let active_requests = Arc::clone(&active_requests);
                            let max_active_requests = Arc::clone(&max_active_requests);
                            workers.push(thread::spawn(move || {
                                serve_fake_image_request(
                                    stream,
                                    &routes,
                                    &active_requests,
                                    &max_active_requests,
                                );
                            }));
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(err) => panic!("failed to accept fake image request: {err}"),
                    }
                }

                for worker in workers {
                    worker.join().unwrap();
                }
            }
        });

        Self {
            address,
            max_active_requests,
            shutdown,
            server_thread: Some(server_thread),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.address, path)
    }

    fn max_active_requests(&self) -> usize {
        self.max_active_requests.load(Ordering::SeqCst)
    }
}

impl Drop for FakeImageBackend {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(server_thread) = self.server_thread.take() {
            server_thread.join().unwrap();
        }
    }
}

fn serve_fake_image_request(
    mut stream: TcpStream,
    routes: &HashMap<String, FakeImageRoute>,
    active_requests: &AtomicUsize,
    max_active_requests: &AtomicUsize,
) {
    let current = active_requests.fetch_add(1, Ordering::SeqCst) + 1;
    update_max_active_requests(max_active_requests, current);

    let (path, headers) = read_request(&stream).unwrap();
    let route = routes
        .get(&path)
        .unwrap_or_else(|| panic!("missing fake image route for {path}"));

    if route.required_headers.iter().any(|(name, value)| {
        headers
            .get(name)
            .map(|candidate| candidate != value)
            .unwrap_or(true)
    }) {
        write!(
            stream,
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        active_requests.fetch_sub(1, Ordering::SeqCst);
        return;
    }

    let status_text = match route.status_code {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let content_length = route.chunks.iter().map(Vec::len).sum::<usize>();

    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        route.status_code, status_text, content_length
    )
    .unwrap();

    for (index, chunk) in route.chunks.iter().enumerate() {
        stream.write_all(chunk).unwrap();
        stream.flush().unwrap();
        if index + 1 < route.chunks.len() && !route.chunk_delay.is_zero() {
            thread::sleep(route.chunk_delay);
        }
    }

    active_requests.fetch_sub(1, Ordering::SeqCst);
}

fn read_request(stream: &TcpStream) -> Option<(String, HashMap<String, String>)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    let path = request_line.split_whitespace().nth(1)?.to_owned();
    let mut headers = HashMap::new();

    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 || header == "\r\n" {
            break;
        }

        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    Some((path, headers))
}

fn update_max_active_requests(max_active_requests: &AtomicUsize, current: usize) {
    let mut previous = max_active_requests.load(Ordering::SeqCst);
    while current > previous {
        match max_active_requests.compare_exchange(
            previous,
            current,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => break,
            Err(next) => previous = next,
        }
    }
}

fn slow_image_chunks(seed: u8) -> Vec<Vec<u8>> {
    vec![vec![seed; 2048], vec![seed + 1; 2048], vec![seed + 2; 2048]]
}

fn expected_slow_image_bytes(seed: u8) -> Vec<u8> {
    slow_image_chunks(seed).into_iter().flatten().collect()
}

fn build_test_image_item(url: String, index: usize) -> ExtractedMediaItem {
    ExtractedMediaItem {
        url,
        extension: "jpg".to_string(),
        description: Some("Weekend dump".to_string()),
        shortcode: "DWWJVEdgSjW".to_string(),
        index,
        http_headers: Vec::new(),
    }
}

fn build_test_image_item_with_headers(
    url: String,
    index: usize,
    http_headers: Vec<(String, String)>,
) -> ExtractedMediaItem {
    ExtractedMediaItem {
        http_headers,
        ..build_test_image_item(url, index)
    }
}

fn collect_updates_by_label(
    updates: &[ImageDownloadProgressUpdate],
) -> HashMap<String, Vec<ImageDownloadProgressUpdate>> {
    let mut grouped = HashMap::new();

    for update in updates {
        grouped
            .entry(update.label.clone())
            .or_insert_with(Vec::new)
            .push(update.clone());
    }

    grouped
}

#[test]
#[cfg(unix)]
fn linux_pseudo_tty_invocation_uses_command_flag() {
    let args = pseudo_tty_invocation_args(
        OsStr::new("/tmp/igdl"),
        &[OsStr::new("--browser"), OsStr::new("chrome")],
        true,
    );

    assert_eq!(
        args,
        vec![
            OsString::from("-q"),
            OsString::from("-e"),
            OsString::from("-c"),
            OsString::from("'/tmp/igdl' '--browser' 'chrome'"),
            OsString::from("/dev/null"),
        ]
    );
}

#[test]
fn stops_after_first_successful_browser_attempt() {
    let attempts = vec![
        (Browser::Chrome, Err("cookie failure".to_string())),
        (Browser::Edge, Ok(vec!["/tmp/instagram/reel.mp4".into()])),
        (
            Browser::Firefox,
            Ok(vec!["/tmp/instagram/unused.mp4".into()]),
        ),
    ];

    let result = choose_successful_browser(attempts).unwrap();
    assert_eq!(result.browser, Browser::Edge);
    assert_eq!(result.paths.len(), 1);
}

#[test]
fn skips_empty_successes_until_a_browser_returns_files() {
    let attempts = vec![
        (Browser::Chrome, Ok(vec![])),
        (Browser::Edge, Err("cookie failure".to_string())),
        (Browser::Firefox, Ok(vec!["/tmp/instagram/reel.mp4".into()])),
    ];

    let result = choose_successful_browser(attempts).unwrap();
    assert_eq!(result.browser, Browser::Firefox);
    assert_eq!(result.paths.len(), 1);
}

#[test]
fn returns_collected_browser_failures_when_no_attempt_succeeds() {
    let attempts = vec![
        (Browser::Chrome, Err("cookie failure".to_string())),
        (Browser::Edge, Ok(vec![])),
    ];

    let err = choose_successful_browser(attempts).unwrap_err();

    match err {
        IgdlError::BrowserCookiesUnavailable(failures) => assert_eq!(
            failures,
            vec![
                "chrome: cookie failure".to_string(),
                "edge: download produced no files".to_string(),
            ]
        ),
        other => panic!("expected browser cookie error, got {other:?}"),
    }
}

#[test]
fn treats_image_media_extraction_as_success() {
    let attempts = vec![
        (
            Browser::Chrome,
            Err("ERROR: [Instagram] DWWJVEdgSjW: No video formats found!".to_string()),
        ),
        (
            Browser::Edge,
            Ok(vec![ExtractedMediaItem {
                url: "https://cdn.example.com/post-1.jpg".to_string(),
                extension: "jpg".to_string(),
                description: Some("Weekend dump".to_string()),
                shortcode: "DWWJVEdgSjW".to_string(),
                index: 1,
                http_headers: Vec::new(),
            }]),
        ),
    ];

    let result = choose_successful_media_extraction(attempts).unwrap();

    assert_eq!(result.browser, Browser::Edge);
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].extension, "jpg");
}

#[test]
fn post_media_extraction_failures_use_media_aware_messages() {
    let attempts = vec![
        (
            Browser::Chrome,
            Err("ERROR: [Instagram] DWWJVEdgSjW: No video formats found!".to_string()),
        ),
        (Browser::Edge, Err("No media extracted".to_string())),
    ];

    let err = choose_successful_media_extraction(attempts).unwrap_err();

    match &err {
        IgdlError::PostMediaDownloadFailed(failures) => assert_eq!(
            failures,
            &vec![
                "chrome: post media unavailable".to_string(),
                "edge: post media unavailable".to_string(),
            ]
        ),
        other => panic!("expected post media download error, got {other:?}"),
    }
    assert_eq!(
        format!("{err}"),
        "media download failed: chrome: post media unavailable; edge: post media unavailable"
    );
}

#[test]
fn cli_plan_uses_manual_output_override() {
    let args = CliArgs::parse_from([
        "igdl",
        "https://www.instagram.com/reel/abc123/",
        "--output",
        "/tmp/custom",
    ]);

    let plan = plan_download(&args, std::path::Path::new("/Users/demo")).unwrap();
    assert_eq!(plan.output_dir, std::path::PathBuf::from("/tmp/custom"));
}

#[test]
#[cfg(unix)]
fn execute_download_plan_stops_after_first_browser_with_files() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let log_path = temp.path().join("attempts.log");
    let script_path = temp.path().join("fake-yt-dlp");

    std::fs::create_dir(&output_dir).unwrap();
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nprintf \"%s\\n\" \"$2\" >> '{}'\nif [ \"$2\" = \"chrome\" ]; then\n  printf \"%s\\n\" '{}'\n  exit 0\nfi\nprintf \"%s\\n\" '{}'\n",
            log_path.display(),
            output_dir.join("chrome.mp4").display(),
            output_dir.join("edge.mp4").display(),
        ),
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/reel/abc123/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Chrome, Browser::Edge],
        verbose: false,
    };

    let outcome = execute_download_plan(
        &plan,
        DownloadBinaries {
            ytdlp_binary: Some(&script_path),
            gallerydl_binary: None,
        },
    )
    .unwrap();

    assert_eq!(outcome.browser, Browser::Chrome);
    assert_eq!(outcome.paths, vec![output_dir.join("chrome.mp4")]);
    assert_eq!(std::fs::read_to_string(log_path).unwrap(), "chrome\n");
}

#[test]
#[cfg(unix)]
fn execute_download_plan_reel_progress_collects_paths_after_progress_output() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let script_path = temp.path().join("fake-yt-dlp-progress");
    let sentinel_path = temp.path().join("progress-observed");

    std::fs::create_dir(&output_dir).unwrap();
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nprogress_enabled=0\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"--no-progress\" ]; then\n    printf '%s\\n' 'progress output was disabled' >&2\n    exit 1\n  fi\n  if [ \"$arg\" = \"--progress\" ]; then\n    progress_enabled=1\n  fi\ndone\nif [ \"$progress_enabled\" -eq 1 ]; then\n  printf '%s\\n' '__IGDL_PROGRESS__ percent=  42.3% downloaded_bytes=5473378 total_bytes=12939428 speed=1289748.6 eta=12'\nfi\nattempts=0\nwhile [ ! -f '{}' ]; do\n  attempts=$((attempts + 1))\n  if [ \"$attempts\" -ge 20 ]; then\n    printf '%s\\n' 'progress callback was not observed before completion' >&2\n    exit 1\n  fi\n  sleep 0.1\ndone\nif [ \"$progress_enabled\" -eq 1 ]; then\n  printf '%s\\n' '__IGDL_PROGRESS__ percent= 100.0% downloaded_bytes=12939428 total_bytes=12939428 speed=NA eta=NA'\nfi\nprintf '%s\\n' '{}'\n",
            sentinel_path.display(),
            output_dir.join("chrome-progress.mp4").display(),
        ),
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/reel/abc123/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Chrome],
        verbose: false,
    };

    let mut progress = Vec::new();
    let mut wrote_sentinel = false;
    let outcome = execute_download_plan_with_progress(
        &plan,
        DownloadBinaries {
            ytdlp_binary: Some(&script_path),
            gallerydl_binary: None,
        },
        |message| {
            if !wrote_sentinel && message == "42% | 5.5 MB / 12.9 MB | 1.3 MB/s | ETA 00:12" {
                std::fs::write(&sentinel_path, b"seen").unwrap();
                wrote_sentinel = true;
            }
            progress.push(message);
        },
    )
    .unwrap();

    assert_eq!(outcome.browser, Browser::Chrome);
    assert_eq!(outcome.paths, vec![output_dir.join("chrome-progress.mp4")]);
    assert!(wrote_sentinel);
    assert_eq!(
        progress,
        vec![
            "Downloading 1/1".to_string(),
            "42% | 5.5 MB / 12.9 MB | 1.3 MB/s | ETA 00:12".to_string(),
            "100% | 12.9 MB / 12.9 MB".to_string(),
        ]
    );
}

#[test]
#[cfg(unix)]
fn execute_download_plan_reel_progress_uses_dynamic_bar_on_tty() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let bin_dir = temp.path().join("bin");
    let script_path = bin_dir.join("yt-dlp");
    let download_path = output_dir.join("chrome-progress.mp4");

    std::fs::create_dir(&output_dir).unwrap();
    std::fs::create_dir(&bin_dir).unwrap();
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nprogress_enabled=0\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"--progress\" ]; then\n    progress_enabled=1\n  fi\ndone\nif [ \"$progress_enabled\" -eq 1 ]; then\n  printf '%s\\n' '__IGDL_PROGRESS__ percent=  42.3% downloaded_bytes=5473378 total_bytes=12939428 speed=1289748.6 eta=12'\n  printf '%s\\n' '__IGDL_PROGRESS__ percent= 100.0% downloaded_bytes=12939428 total_bytes=12939428 speed=NA eta=NA'\nfi\nprintf '%s\\n' '{}'\n",
            download_path.display(),
        ),
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let mut path = std::ffi::OsString::new();
    path.push(&bin_dir);
    if let Some(existing_path) = std::env::var_os("PATH") {
        path.push(":");
        path.push(existing_path);
    }

    let output = capture_cli_output_in_pseudo_tty(
        path,
        &[
            "https://www.instagram.com/reel/abc123/".as_ref(),
            "--browser".as_ref(),
            "chrome".as_ref(),
            "--output".as_ref(),
            output_dir.as_os_str(),
        ],
    );
    let rendered = render_captured_output(&output);

    assert!(output.status.success(), "unexpected output: {rendered}");
    assert!(
        !rendered.contains("Downloading 1/1"),
        "unexpected output: {rendered}"
    );
    assert!(
        rendered.contains("[########------------] 42% | 1.3 MB/s | ETA 00:12"),
        "unexpected output: {rendered}"
    );
    assert!(
        rendered.contains(&download_path.display().to_string()),
        "unexpected output: {rendered}"
    );
}

#[test]
#[cfg(unix)]
fn execute_download_plan_post_media_keeps_line_progress_on_tty() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let bin_dir = temp.path().join("bin");
    let gallerydl_path = bin_dir.join("gallery-dl");
    let ytdlp_path = bin_dir.join("yt-dlp");

    std::fs::create_dir(&output_dir).unwrap();
    std::fs::create_dir(&bin_dir).unwrap();
    std::fs::write(
        &ytdlp_path,
        "#!/bin/sh
exit 0
",
    )
    .unwrap();
    let mut ytdlp_permissions = std::fs::metadata(&ytdlp_path).unwrap().permissions();
    ytdlp_permissions.set_mode(0o755);
    std::fs::set_permissions(&ytdlp_path, ytdlp_permissions).unwrap();

    std::fs::write(
        &gallerydl_path,
        "#!/bin/sh
if [ \"$3\" = \"-j\" ]; then
  printf '%s\n' '[3, \"ytdl:https://cdn.example.com/post-1\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}]'
  printf '%s\n' '[3, \"ytdl:https://cdn.example.com/post-2\", {\"extension\": \"mp4\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 2}]'
  exit 0
fi
if [ \"$3\" = \"-D\" ]; then
  mkdir -p \"$4\"
  printf '%s' 'first-image' > \"$4/DWWJVEdgSjW_01.jpg\"
  printf 'completed %s\n' \"$4/DWWJVEdgSjW_01.jpg\" >&2
  printf '%s' 'second-video' > \"$4/DWWJVEdgSjW_02.mp4\"
  printf 'completed %s\n' \"$4/DWWJVEdgSjW_02.mp4\" >&2
  exit 0
fi
printf 'unexpected invocation\n' >&2
exit 1
",
    )
    .unwrap();
    let mut gallerydl_permissions = std::fs::metadata(&gallerydl_path).unwrap().permissions();
    gallerydl_permissions.set_mode(0o755);
    std::fs::set_permissions(&gallerydl_path, gallerydl_permissions).unwrap();

    let mut path = std::ffi::OsString::new();
    path.push(&bin_dir);
    if let Some(existing_path) = std::env::var_os("PATH") {
        path.push(":");
        path.push(existing_path);
    }

    let output = capture_cli_output_in_pseudo_tty(
        path,
        &[
            "https://www.instagram.com/p/DWWJVEdgSjW/".as_ref(),
            "--browser".as_ref(),
            "edge".as_ref(),
            "--output".as_ref(),
            output_dir.as_os_str(),
        ],
    );
    let rendered = render_captured_output(&output);

    assert!(output.status.success(), "unexpected output: {rendered}");
    assert!(
        rendered.contains("1/2\r\n2/2\r\n"),
        "unexpected output: {rendered:?}"
    );
    assert!(
        !rendered.contains("\r1/2\r2/2"),
        "unexpected output: {rendered:?}"
    );
}

#[test]
#[cfg(unix)]
fn execute_download_plan_image_parallel_tty_renders_rows_in_place() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let bin_dir = temp.path().join("bin");
    let gallerydl_path = bin_dir.join("gallery-dl");
    let ytdlp_path = bin_dir.join("yt-dlp");
    let backend = FakeImageBackend::new(HashMap::from([
        (
            "/post-1.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(90), Duration::from_millis(90)),
        ),
        (
            "/post-2.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(100), Duration::from_millis(90)),
        ),
        (
            "/post-3.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(110), Duration::from_millis(90)),
        ),
    ]));

    std::fs::create_dir(&output_dir).unwrap();
    std::fs::create_dir(&bin_dir).unwrap();
    std::fs::write(
        &ytdlp_path,
        "#!/bin/sh
exit 0
",
    )
    .unwrap();
    let mut ytdlp_permissions = std::fs::metadata(&ytdlp_path).unwrap().permissions();
    ytdlp_permissions.set_mode(0o755);
    std::fs::set_permissions(&ytdlp_path, ytdlp_permissions).unwrap();

    std::fs::write(
        &gallerydl_path,
        format!(
            "#!/bin/sh
if [ \"$3\" = \"-j\" ]; then
  printf '%s\\n' '[3, \"{}\", {{\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}}]'
  printf '%s\\n' '[3, \"{}\", {{\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 2}}]'
  printf '%s\\n' '[3, \"{}\", {{\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 3}}]'
  exit 0
fi
printf 'unexpected invocation\\n' >&2
exit 1
",
            backend.url("/post-1.jpg"),
            backend.url("/post-2.jpg"),
            backend.url("/post-3.jpg"),
        ),
    )
    .unwrap();
    let mut gallerydl_permissions = std::fs::metadata(&gallerydl_path).unwrap().permissions();
    gallerydl_permissions.set_mode(0o755);
    std::fs::set_permissions(&gallerydl_path, gallerydl_permissions).unwrap();

    let mut path = std::ffi::OsString::new();
    path.push(&bin_dir);
    if let Some(existing_path) = std::env::var_os("PATH") {
        path.push(":");
        path.push(existing_path);
    }

    let output = capture_cli_output_in_pseudo_tty(
        path,
        &[
            "https://www.instagram.com/p/DWWJVEdgSjW/".as_ref(),
            "--browser".as_ref(),
            "edge".as_ref(),
            "--output".as_ref(),
            output_dir.as_os_str(),
        ],
    );
    let rendered = render_captured_output(&output);

    assert!(output.status.success(), "unexpected output: {rendered}");
    assert!(rendered.contains("<UP:"), "unexpected output: {rendered:?}");
    for row in [
        "weekend-dump-DWWJVEdgSjW-01.jpg 100% | done",
        "weekend-dump-DWWJVEdgSjW-02.jpg 100% | done",
        "weekend-dump-DWWJVEdgSjW-03.jpg 100% | done",
    ] {
        assert!(
            rendered.contains(row),
            "missing final row {row}: {rendered:?}"
        );
    }
    assert!(
        !rendered.contains("Downloading 1/3"),
        "unexpected output: {rendered:?}"
    );
    assert!(
        !rendered.contains("Downloading 2/3"),
        "unexpected output: {rendered:?}"
    );
    assert!(
        !rendered.contains("\r\n1/3\r\n2/3"),
        "unexpected output: {rendered:?}"
    );
}

#[test]
#[cfg(unix)]
fn execute_download_plan_image_parallel_tty_single_image_avoids_counter_placeholder() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let bin_dir = temp.path().join("bin");
    let gallerydl_path = bin_dir.join("gallery-dl");
    let ytdlp_path = bin_dir.join("yt-dlp");
    let backend = FakeImageBackend::new(HashMap::from([(
        "/post-1.jpg".to_string(),
        FakeImageRoute::success(slow_image_chunks(120), Duration::from_millis(90)),
    )]));

    std::fs::create_dir(&output_dir).unwrap();
    std::fs::create_dir(&bin_dir).unwrap();
    std::fs::write(
        &ytdlp_path,
        "#!/bin/sh
exit 0
",
    )
    .unwrap();
    let mut ytdlp_permissions = std::fs::metadata(&ytdlp_path).unwrap().permissions();
    ytdlp_permissions.set_mode(0o755);
    std::fs::set_permissions(&ytdlp_path, ytdlp_permissions).unwrap();

    std::fs::write(
        &gallerydl_path,
        format!(
            "#!/bin/sh
if [ \"$3\" = \"-j\" ]; then
  printf '%s\\n' '[3, \"{}\", {{\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}}]'
  exit 0
fi
printf 'unexpected invocation\\n' >&2
exit 1
",
            backend.url("/post-1.jpg"),
        ),
    )
    .unwrap();
    let mut gallerydl_permissions = std::fs::metadata(&gallerydl_path).unwrap().permissions();
    gallerydl_permissions.set_mode(0o755);
    std::fs::set_permissions(&gallerydl_path, gallerydl_permissions).unwrap();

    let mut path = std::ffi::OsString::new();
    path.push(&bin_dir);
    if let Some(existing_path) = std::env::var_os("PATH") {
        path.push(":");
        path.push(existing_path);
    }

    let output = capture_cli_output_in_pseudo_tty(
        path,
        &[
            "https://www.instagram.com/p/DWWJVEdgSjW/".as_ref(),
            "--browser".as_ref(),
            "edge".as_ref(),
            "--output".as_ref(),
            output_dir.as_os_str(),
        ],
    );
    let rendered = render_captured_output(&output);

    assert!(output.status.success(), "unexpected output: {rendered}");
    assert!(
        rendered.contains("weekend-dump-DWWJVEdgSjW.jpg 100% | done"),
        "unexpected output: {rendered:?}"
    );
    assert!(
        !rendered.contains("Downloading 1/1"),
        "unexpected output: {rendered:?}"
    );
    assert!(!rendered.contains("1/1"), "unexpected output: {rendered:?}");
}

#[test]
#[cfg(unix)]
fn execute_download_plan_downloads_post_media_items_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let log_path = temp.path().join("gallery-attempts.log");
    let script_path = temp.path().join("fake-gallery-dl");

    std::fs::create_dir(&output_dir).unwrap();

    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nif [ \"$3\" = \"-j\" ]; then\n  printf \"extract:%s\\n\" \"$2\" >> '{}'\n  if [ \"$2\" = \"chrome\" ]; then\n    printf \"%s\\n\" \"No media extracted\" >&2\n    exit 1\n  fi\n  printf '%s\\n' '{}'\n  printf '%s\\n' '{}'\n  exit 0\nfi\nif [ \"$3\" = \"-D\" ]; then\n  printf \"download:%s:%s:%s\\n\" \"$2\" \"$4\" \"$6\" >> '{}'\n  mkdir -p \"$4\"\n  printf '%s' 'first-image' > \"$4/DWWJVEdgSjW_01.jpg\"\n  printf '%s' 'second-video' > \"$4/DWWJVEdgSjW_02.mp4\"\n  exit 0\nfi\nprintf \"unexpected invocation\\n\" >&2\nexit 1\n",
            log_path.display(),
            "[3, \"ytdl:https://cdn.example.com/post-1\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1, \"_http_headers\": {\"User-Agent\": \"Example\"}}]",
            "[3, \"ytdl:https://cdn.example.com/post-2\", {\"extension\": \"mp4\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 2}]",
            log_path.display(),
        ),
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/p/DWWJVEdgSjW/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Chrome, Browser::Edge],
        verbose: false,
    };

    let outcome = execute_download_plan(
        &plan,
        DownloadBinaries {
            ytdlp_binary: None,
            gallerydl_binary: Some(&script_path),
        },
    )
    .unwrap();

    let expected = vec![
        output_dir.join("weekend-dump-DWWJVEdgSjW-01.jpg"),
        output_dir.join("weekend-dump-DWWJVEdgSjW-02.mp4"),
    ];
    assert_eq!(outcome.browser, Browser::Edge);
    assert_eq!(outcome.paths, expected);
    assert_eq!(
        std::fs::read_to_string(log_path).unwrap(),
        format!(
            "extract:chrome\nextract:edge\ndownload:edge:{}:{{post_shortcode}}_{{num:>02}}.{{extension}}\n",
            output_dir.join(".igdl-gallerydl-download").display()
        )
    );
    assert_eq!(std::fs::read(&outcome.paths[0]).unwrap(), b"first-image");
    assert_eq!(std::fs::read(&outcome.paths[1]).unwrap(), b"second-video");
    assert!(!output_dir.join(".igdl-gallerydl-download").exists());
}

#[test]
#[cfg(unix)]
fn execute_download_plan_retries_remaining_browsers_after_post_download_failure() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let log_path = temp.path().join("gallery-fallback.log");
    let script_path = temp.path().join("fake-gallery-dl-fallback");

    std::fs::create_dir(&output_dir).unwrap();

    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nif [ \"$3\" = \"-j\" ]; then\n  printf \"extract:%s\\n\" \"$2\" >> '{}'\n  printf '%s\\n' '{}'\n  printf '%s\\n' '{}'\n  exit 0\nfi\nif [ \"$3\" = \"-D\" ]; then\n  printf \"download:%s:%s:%s\\n\" \"$2\" \"$4\" \"$6\" >> '{}'\n  if [ \"$2\" = \"chrome\" ]; then\n    printf \"%s\\n\" \"simulated gallery-dl download failure\" >&2\n    exit 1\n  fi\n  mkdir -p \"$4\"\n  printf '%s' 'first-image' > \"$4/DWWJVEdgSjW_01.jpg\"\n  printf '%s' 'second-video' > \"$4/DWWJVEdgSjW_02.mp4\"\n  exit 0\nfi\nprintf \"unexpected invocation\\n\" >&2\nexit 1\n",
            log_path.display(),
            "[3, \"ytdl:https://cdn.example.com/post-1\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}]",
            "[3, \"ytdl:https://cdn.example.com/post-2\", {\"extension\": \"mp4\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 2}]",
            log_path.display(),
        ),
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/p/DWWJVEdgSjW/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Chrome, Browser::Edge],
        verbose: false,
    };

    let outcome = execute_download_plan(
        &plan,
        DownloadBinaries {
            ytdlp_binary: None,
            gallerydl_binary: Some(&script_path),
        },
    )
    .unwrap();

    assert_eq!(outcome.browser, Browser::Edge);
    assert_eq!(
        outcome.paths,
        vec![
            output_dir.join("weekend-dump-DWWJVEdgSjW-01.jpg"),
            output_dir.join("weekend-dump-DWWJVEdgSjW-02.mp4"),
        ]
    );
    assert_eq!(
        std::fs::read_to_string(log_path).unwrap(),
        format!(
            "extract:chrome\ndownload:chrome:{}:{{post_shortcode}}_{{num:>02}}.{{extension}}\nextract:edge\ndownload:edge:{}:{{post_shortcode}}_{{num:>02}}.{{extension}}\n",
            output_dir.join(".igdl-gallerydl-download").display(),
            output_dir.join(".igdl-gallerydl-download").display(),
        )
    );
}

#[test]
#[cfg(unix)]
fn execute_download_plan_emits_overall_progress_for_multi_item_posts() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let script_path = temp.path().join("fake-gallery-dl-progress");

    std::fs::create_dir(&output_dir).unwrap();

    std::fs::write(
        &script_path,
        "#!/bin/sh
if [ \"$3\" = \"-j\" ]; then
  printf '%s\n' '[3, \"ytdl:https://cdn.example.com/post-1\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}]'
  printf '%s\n' '[3, \"ytdl:https://cdn.example.com/post-2\", {\"extension\": \"mp4\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 2}]'
  exit 0
fi
if [ \"$3\" = \"-D\" ]; then
  mkdir -p \"$4\"
  printf '%s' 'first-image' > \"$4/DWWJVEdgSjW_01.jpg\"
  printf 'completed %s\n' \"$4/DWWJVEdgSjW_01.jpg\" >&2
  printf '%s' 'second-video' > \"$4/DWWJVEdgSjW_02.mp4\"
  printf 'completed %s\n' \"$4/DWWJVEdgSjW_02.mp4\" >&2
  exit 0
fi
printf 'unexpected invocation\n' >&2
exit 1
",
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/p/DWWJVEdgSjW/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Edge],
        verbose: false,
    };

    let mut progress = Vec::new();
    let outcome = execute_download_plan_with_progress(
        &plan,
        DownloadBinaries {
            ytdlp_binary: None,
            gallerydl_binary: Some(&script_path),
        },
        |message| progress.push(message),
    )
    .unwrap();

    assert_eq!(
        outcome.paths,
        vec![
            output_dir.join("weekend-dump-DWWJVEdgSjW-01.jpg"),
            output_dir.join("weekend-dump-DWWJVEdgSjW-02.mp4"),
        ]
    );
    assert_eq!(progress, vec!["1/2".to_string(), "2/2".to_string()]);
}

#[test]
#[cfg(unix)]
fn execute_download_plan_image_only_parallel_path_uses_item_progress() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let script_path = temp.path().join("fake-gallery-dl-image-progress");
    let backend = FakeImageBackend::new(HashMap::from([
        (
            "/post-1.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(1), Duration::from_millis(60)),
        ),
        (
            "/post-2.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(4), Duration::from_millis(60)),
        ),
    ]));

    std::fs::create_dir(&output_dir).unwrap();

    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh
if [ \"$3\" = \"-j\" ]; then
  printf '%s\\n' '[3, \"{}\", {{\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}}]'
  printf '%s\\n' '[3, \"{}\", {{\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 2}}]'
  exit 0
fi
printf 'unexpected invocation\\n' >&2
exit 1
",
            backend.url("/post-1.jpg"),
            backend.url("/post-2.jpg"),
        ),
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/p/DWWJVEdgSjW/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Edge],
        verbose: false,
    };

    let mut progress = Vec::new();
    let outcome = execute_download_plan_with_progress(
        &plan,
        DownloadBinaries {
            ytdlp_binary: None,
            gallerydl_binary: Some(&script_path),
        },
        |message| progress.push(message),
    )
    .unwrap();

    assert_eq!(
        outcome.paths,
        vec![
            output_dir.join("weekend-dump-DWWJVEdgSjW-01.jpg"),
            output_dir.join("weekend-dump-DWWJVEdgSjW-02.jpg"),
        ]
    );
    assert_eq!(
        progress,
        vec!["Downloading 1/2".to_string(), "Downloading 2/2".to_string(),]
    );
    assert_eq!(backend.max_active_requests(), 2);
}

#[test]
fn image_parallel_progress_emits_per_item_updates_and_caps_concurrency() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let backend = FakeImageBackend::new(HashMap::from([
        (
            "/post-1.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(10), Duration::from_millis(90)),
        ),
        (
            "/post-2.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(20), Duration::from_millis(90))
                .requiring_header("x-test-auth", "image-2"),
        ),
        (
            "/post-3.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(30), Duration::from_millis(90)),
        ),
        (
            "/post-4.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(40), Duration::from_millis(90)),
        ),
    ]));
    std::fs::create_dir(&output_dir).unwrap();

    let items = vec![
        build_test_image_item(backend.url("/post-1.jpg"), 1),
        build_test_image_item_with_headers(
            backend.url("/post-2.jpg"),
            2,
            vec![("x-test-auth".to_string(), "image-2".to_string())],
        ),
        build_test_image_item(backend.url("/post-3.jpg"), 3),
        build_test_image_item(backend.url("/post-4.jpg"), 4),
    ];

    let mut updates = Vec::new();
    let paths = download_image_items_with_detailed_progress(
        MediaDownloadRequest {
            binary: std::path::Path::new("gallery-dl"),
            browser: Browser::Edge,
            url: "https://www.instagram.com/p/DWWJVEdgSjW/",
            items: &items,
            output_dir: &output_dir,
            ytdlp_binary: None,
            verbose: false,
        },
        |update| updates.push(update),
    )
    .unwrap();

    assert_eq!(backend.max_active_requests(), 3);
    assert_eq!(
        paths,
        vec![
            output_dir.join("weekend-dump-DWWJVEdgSjW-01.jpg"),
            output_dir.join("weekend-dump-DWWJVEdgSjW-02.jpg"),
            output_dir.join("weekend-dump-DWWJVEdgSjW-03.jpg"),
            output_dir.join("weekend-dump-DWWJVEdgSjW-04.jpg"),
        ]
    );

    let updates_by_label = collect_updates_by_label(&updates);
    for label in [
        "weekend-dump-DWWJVEdgSjW-01.jpg",
        "weekend-dump-DWWJVEdgSjW-02.jpg",
        "weekend-dump-DWWJVEdgSjW-03.jpg",
        "weekend-dump-DWWJVEdgSjW-04.jpg",
    ] {
        let item_updates = updates_by_label
            .get(label)
            .unwrap_or_else(|| panic!("missing progress updates for {label}"));
        assert!(
            item_updates
                .iter()
                .any(|update| !update.completed && update.downloaded_bytes > 0),
            "expected active progress update for {label}: {item_updates:?}"
        );
        assert!(
            item_updates.iter().any(|update| update.completed),
            "expected completion update for {label}: {item_updates:?}"
        );
        assert!(
            item_updates
                .iter()
                .all(|update| !update.item_id.is_empty() && update.total_bytes.is_some()),
            "expected structured item identity and totals for {label}: {item_updates:?}"
        );
    }
}

#[test]
fn image_parallel_progress_partial_failure_preserves_completed_files() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let backend = FakeImageBackend::new(HashMap::from([
        (
            "/post-1.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(50), Duration::from_millis(60)),
        ),
        ("/post-2.jpg".to_string(), FakeImageRoute::failure(500)),
        (
            "/post-3.jpg".to_string(),
            FakeImageRoute::success(slow_image_chunks(70), Duration::from_millis(60)),
        ),
    ]));
    std::fs::create_dir(&output_dir).unwrap();

    let items = vec![
        build_test_image_item(backend.url("/post-1.jpg"), 1),
        build_test_image_item(backend.url("/post-2.jpg"), 2),
        build_test_image_item(backend.url("/post-3.jpg"), 3),
    ];

    let mut updates = Vec::new();
    let err = download_image_items_with_detailed_progress(
        MediaDownloadRequest {
            binary: std::path::Path::new("gallery-dl"),
            browser: Browser::Edge,
            url: "https://www.instagram.com/p/DWWJVEdgSjW/",
            items: &items,
            output_dir: &output_dir,
            ytdlp_binary: None,
            verbose: false,
        },
        |update| updates.push(update),
    )
    .unwrap_err();

    match err {
        IgdlError::PostMediaDownloadPartial {
            downloaded,
            failed,
            total,
        } => {
            assert_eq!(downloaded, 2);
            assert_eq!(failed, 1);
            assert_eq!(total, 3);
        }
        other => panic!("expected partial image download error, got {other:?}"),
    }

    assert_eq!(
        std::fs::read(output_dir.join("weekend-dump-DWWJVEdgSjW-01.jpg")).unwrap(),
        expected_slow_image_bytes(50)
    );
    assert!(!output_dir.join("weekend-dump-DWWJVEdgSjW-02.jpg").exists());
    assert_eq!(
        std::fs::read(output_dir.join("weekend-dump-DWWJVEdgSjW-03.jpg")).unwrap(),
        expected_slow_image_bytes(70)
    );

    let updates_by_label = collect_updates_by_label(&updates);
    for label in [
        "weekend-dump-DWWJVEdgSjW-01.jpg",
        "weekend-dump-DWWJVEdgSjW-03.jpg",
    ] {
        let item_updates = updates_by_label
            .get(label)
            .unwrap_or_else(|| panic!("missing progress updates for {label}"));
        assert!(
            item_updates.iter().any(|update| update.completed),
            "expected completion update for {label}: {item_updates:?}"
        );
    }
    assert!(
        updates_by_label
            .get("weekend-dump-DWWJVEdgSjW-02.jpg")
            .is_none_or(|item_updates| item_updates.iter().all(|update| !update.completed))
    );
}

#[test]
#[cfg(unix)]
fn execute_download_plan_cleans_up_temp_dir_after_post_download_failure() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let script_path = temp.path().join("fake-gallery-dl-cleanup");
    let temp_download_dir = output_dir.join(".igdl-gallerydl-download");

    std::fs::create_dir(&output_dir).unwrap();

    std::fs::write(
        &script_path,
        "#!/bin/sh
if [ \"$3\" = \"-j\" ]; then
  printf '%s\n' '[3, \"ytdl:https://cdn.example.com/post-1\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}]'
  exit 0
fi
if [ \"$3\" = \"-D\" ]; then
  mkdir -p \"$4\"
  printf '%s' 'partial-image' > \"$4/DWWJVEdgSjW_01.jpg\"
  printf '%s\n' 'simulated gallery-dl download failure' >&2
  exit 1
fi
printf 'unexpected invocation\n' >&2
exit 1
",
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/p/DWWJVEdgSjW/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Edge],
        verbose: false,
    };

    let err = execute_download_plan(
        &plan,
        DownloadBinaries {
            ytdlp_binary: None,
            gallerydl_binary: Some(&script_path),
        },
    )
    .unwrap_err();

    match err {
        IgdlError::PostMediaDownloadFailed(failures) => {
            assert_eq!(
                failures,
                vec!["edge: simulated gallery-dl download failure".to_string()]
            );
        }
        other => panic!("expected post media download error, got {other:?}"),
    }
    assert!(!temp_download_dir.exists());
}

#[test]
#[cfg(unix)]
fn execute_download_plan_reports_media_aware_post_failure_messages() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let script_path = temp.path().join("fake-gallery-dl-error");

    std::fs::create_dir(&output_dir).unwrap();

    std::fs::write(
        &script_path,
        "#!/bin/sh
if [ \"$3\" = \"-j\" ]; then
  printf '%s\n' 'ERROR: [Instagram] DWWJVEdgSjW: No video formats found!' >&2
  exit 1
fi
printf 'unexpected invocation\n' >&2
exit 1
",
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/p/DWWJVEdgSjW/".to_string(),
        output_dir,
        browsers: vec![Browser::Edge],
        verbose: false,
    };

    let err = execute_download_plan(
        &plan,
        DownloadBinaries {
            ytdlp_binary: None,
            gallerydl_binary: Some(&script_path),
        },
    )
    .unwrap_err();

    match &err {
        IgdlError::PostMediaDownloadFailed(failures) => {
            assert_eq!(failures, &vec!["edge: post media unavailable".to_string()]);
        }
        other => panic!("expected post media download error, got {other:?}"),
    }
    assert_eq!(
        format!("{err}"),
        "media download failed: edge: post media unavailable"
    );
}

#[test]
#[cfg(unix)]
fn partial_post_download_preserves_completed_files() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let script_path = temp.path().join("fake-gallery-dl-partial");
    let temp_download_dir = output_dir.join(".igdl-gallerydl-download");

    std::fs::create_dir(&output_dir).unwrap();

    std::fs::write(
        &script_path,
        "#!/bin/sh
if [ \"$3\" = \"-D\" ]; then
  mkdir -p \"$4\"
  printf '%s' 'first-image' > \"$4/DWWJVEdgSjW_01.jpg\"
  printf '%s\n' 'simulated gallery-dl download failure' >&2
  exit 1
fi
printf 'unexpected invocation\n' >&2
exit 1
",
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let items = vec![
        ExtractedMediaItem {
            url: "https://cdn.example.com/post-1.jpg".to_string(),
            extension: "jpg".to_string(),
            description: Some("Weekend dump".to_string()),
            shortcode: "DWWJVEdgSjW".to_string(),
            index: 1,
            http_headers: Vec::new(),
        },
        ExtractedMediaItem {
            url: "https://cdn.example.com/post-2.mp4".to_string(),
            extension: "mp4".to_string(),
            description: Some("Weekend dump".to_string()),
            shortcode: "DWWJVEdgSjW".to_string(),
            index: 2,
            http_headers: Vec::new(),
        },
    ];

    let err = download_media_items_with_progress(
        MediaDownloadRequest {
            binary: &script_path,
            browser: Browser::Edge,
            url: "https://www.instagram.com/p/DWWJVEdgSjW/",
            items: &items,
            output_dir: &output_dir,
            ytdlp_binary: None,
            verbose: false,
        },
        |_| {},
    )
    .unwrap_err();

    match &err {
        IgdlError::PostMediaDownloadPartial {
            downloaded,
            failed,
            total,
        } => {
            assert_eq!(*downloaded, 1);
            assert_eq!(*failed, 1);
            assert_eq!(*total, 2);
        }
        other => panic!("expected partial post media error, got {other:?}"),
    }

    assert_eq!(
        std::fs::read(output_dir.join("weekend-dump-DWWJVEdgSjW-01.jpg")).unwrap(),
        b"first-image"
    );
    assert!(!output_dir.join("weekend-dump-DWWJVEdgSjW-02.mp4").exists());
    assert!(!temp_download_dir.exists());
}

#[test]
#[cfg(unix)]
fn partial_post_download_stops_retrying_later_browsers() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let log_path = temp.path().join("gallery-partial.log");
    let script_path = temp.path().join("fake-gallery-dl-partial-retry");
    let temp_download_dir = output_dir.join(".igdl-gallerydl-download");

    std::fs::create_dir(&output_dir).unwrap();

    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nif [ \"$3\" = \"-j\" ]; then\n  printf \"extract:%s\\n\" \"$2\" >> '{}'\n  printf '%s\\n' '{}'\n  printf '%s\\n' '{}'\n  exit 0\nfi\nif [ \"$3\" = \"-D\" ]; then\n  printf \"download:%s:%s:%s\\n\" \"$2\" \"$4\" \"$6\" >> '{}'\n  mkdir -p \"$4\"\n  printf '%s' 'first-image' > \"$4/DWWJVEdgSjW_01.jpg\"\n  printf '%s\\n' 'simulated gallery-dl download failure' >&2\n  exit 1\nfi\nprintf 'unexpected invocation\\n' >&2\nexit 1\n",
            log_path.display(),
            "[3, \"ytdl:https://cdn.example.com/post-1\", {\"extension\": \"jpg\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}]",
            "[3, \"ytdl:https://cdn.example.com/post-2\", {\"extension\": \"mp4\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 2}]",
            log_path.display(),
        ),
    )
    .unwrap();

    let mut permissions = std::fs::metadata(&script_path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/p/DWWJVEdgSjW/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Chrome, Browser::Edge],
        verbose: false,
    };

    let err = execute_download_plan(
        &plan,
        DownloadBinaries {
            ytdlp_binary: None,
            gallerydl_binary: Some(&script_path),
        },
    )
    .unwrap_err();

    match &err {
        IgdlError::PostMediaDownloadPartial {
            downloaded,
            failed,
            total,
        } => {
            assert_eq!(*downloaded, 1);
            assert_eq!(*failed, 1);
            assert_eq!(*total, 2);
        }
        other => panic!("expected partial post media error, got {other:?}"),
    }

    assert_eq!(
        std::fs::read_to_string(&log_path).unwrap(),
        format!(
            "extract:chrome\ndownload:chrome:{}:{{post_shortcode}}_{{num:>02}}.{{extension}}\n",
            output_dir.join(".igdl-gallerydl-download").display()
        )
    );
    assert_eq!(
        std::fs::read(output_dir.join("weekend-dump-DWWJVEdgSjW-01.jpg")).unwrap(),
        b"first-image"
    );
    assert!(!output_dir.join("weekend-dump-DWWJVEdgSjW-02.mp4").exists());
    assert!(!temp_download_dir.exists());
    assert_eq!(format!("{err}"), "downloaded 1 of 2 media items; 1 failed");
}

#[test]
#[cfg(unix)]
fn execute_download_plan_exposes_ytdlp_to_gallerydl_downloads() {
    let temp = tempfile::tempdir().unwrap();
    let output_dir = temp.path().join("downloads");
    let script_path = temp.path().join("fake-gallery-dl-ytdlp");
    let ytdlp_dir = temp.path().join("ytdlp-bin");
    let ytdlp_path = ytdlp_dir.join("yt-dlp");

    std::fs::create_dir(&output_dir).unwrap();
    std::fs::create_dir(&ytdlp_dir).unwrap();

    std::fs::write(
        &ytdlp_path,
        "#!/bin/sh
exit 0
",
    )
    .unwrap();
    let mut ytdlp_permissions = std::fs::metadata(&ytdlp_path).unwrap().permissions();
    ytdlp_permissions.set_mode(0o755);
    std::fs::set_permissions(&ytdlp_path, ytdlp_permissions).unwrap();

    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh
if [ \"$3\" = \"-j\" ]; then
  printf '%s\n' '[3, \"ytdl:https://cdn.example.com/post-1\", {{\"extension\": \"mp4\", \"description\": \"Weekend dump\", \"post_shortcode\": \"DWWJVEdgSjW\", \"num\": 1}}]'
  exit 0
fi
if [ \"$3\" = \"-D\" ]; then
  case \":$PATH:\" in
    *\":{}:\"*) ;;
    *)
      printf '%s\n' 'yt-dlp directory missing from PATH' >&2
      exit 1
      ;;
  esac
  command -v yt-dlp >/dev/null 2>&1 || {{
    printf '%s\n' 'yt-dlp missing from PATH' >&2
    exit 1
  }}
  mkdir -p \"$4\"
  printf '%s' 'video' > \"$4/DWWJVEdgSjW_01.mp4\"
  exit 0
fi
printf 'unexpected invocation\n' >&2
exit 1
",
            ytdlp_dir.display()
        ),
    )
    .unwrap();
    let mut script_permissions = std::fs::metadata(&script_path).unwrap().permissions();
    script_permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, script_permissions).unwrap();

    let plan = DownloadPlan {
        url: "https://www.instagram.com/p/DWWJVEdgSjW/".to_string(),
        output_dir: output_dir.clone(),
        browsers: vec![Browser::Edge],
        verbose: false,
    };

    let outcome = execute_download_plan(
        &plan,
        DownloadBinaries {
            ytdlp_binary: Some(&ytdlp_path),
            gallerydl_binary: Some(&script_path),
        },
    )
    .unwrap();

    assert_eq!(outcome.browser, Browser::Edge);
    assert_eq!(
        outcome.paths,
        vec![output_dir.join("weekend-dump-DWWJVEdgSjW.mp4")]
    );
}
