use clap::Parser;
use igdl::browser::Browser;
use igdl::cli::CliArgs;
use igdl::download::choose_successful_browser;
use igdl::download::choose_successful_media_extraction;
use igdl::download::execute_download_plan;
use igdl::download::execute_download_plan_with_progress;
use igdl::download::plan_download;
use igdl::download::DownloadBinaries;
use igdl::download::DownloadPlan;
use igdl::error::IgdlError;
use igdl::gallerydl::download_media_items_with_progress;
use igdl::gallerydl::ExtractedMediaItem;
use igdl::gallerydl::MediaDownloadRequest;
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
        },
        ExtractedMediaItem {
            url: "https://cdn.example.com/post-2.mp4".to_string(),
            extension: "mp4".to_string(),
            description: Some("Weekend dump".to_string()),
            shortcode: "DWWJVEdgSjW".to_string(),
            index: 2,
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
