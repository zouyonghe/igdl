use igdl::ytdlp::{YtDlpProgressUpdate, parse_progress_line};
use std::time::Duration;

#[test]
fn parses_template_progress_line_with_all_fields() {
    let line = "__IGDL_PROGRESS__percent=  42.3% downloaded_bytes=5473378 total_bytes=12939428 speed=1289748.6 eta=12";

    assert_eq!(
        parse_progress_line(line),
        Some(YtDlpProgressUpdate {
            percentage: Some(42.3),
            downloaded_bytes: Some(5_473_378),
            total_bytes: Some(12_939_428),
            speed_bytes_per_second: Some(1_289_749),
            eta: Some(Duration::from_secs(12)),
        })
    );
}

#[test]
fn treats_missing_or_unavailable_template_values_as_none() {
    let line = "__IGDL_PROGRESS__percent=  42.3% downloaded_bytes= total_bytes=NA speed=unavailable eta=12";

    assert_eq!(
        parse_progress_line(line),
        Some(YtDlpProgressUpdate {
            percentage: Some(42.3),
            downloaded_bytes: None,
            total_bytes: None,
            speed_bytes_per_second: None,
            eta: Some(Duration::from_secs(12)),
        })
    );
}

#[test]
fn parses_completed_template_progress_line_into_final_update() {
    let line = "__IGDL_PROGRESS__percent= 100.0% downloaded_bytes=12939428 total_bytes=12939428 speed=NA eta=NA";

    assert_eq!(
        parse_progress_line(line),
        Some(YtDlpProgressUpdate {
            percentage: Some(100.0),
            downloaded_bytes: Some(12_939_428),
            total_bytes: Some(12_939_428),
            speed_bytes_per_second: None,
            eta: None,
        })
    );
}

#[test]
fn ignores_non_template_lines() {
    assert_eq!(parse_progress_line("ERROR: unable to download video"), None);
}

#[test]
fn ignores_prefixed_non_template_lines() {
    let line = "__IGDL_PROGRESS__[download] 100% of 12.34MiB in 00:15";

    assert_eq!(parse_progress_line(line), None);
}
