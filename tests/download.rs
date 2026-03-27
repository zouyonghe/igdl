use clap::Parser;
use igdl::browser::Browser;
use igdl::cli::CliArgs;
use igdl::download::choose_successful_browser;
use igdl::download::execute_download_plan;
use igdl::download::plan_download;
use igdl::download::DownloadPlan;
use igdl::error::IgdlError;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

    let outcome = execute_download_plan(&plan, &script_path).unwrap();

    assert_eq!(outcome.browser, Browser::Chrome);
    assert_eq!(outcome.paths, vec![output_dir.join("chrome.mp4")]);
    assert_eq!(std::fs::read_to_string(log_path).unwrap(), "chrome\n");
}
