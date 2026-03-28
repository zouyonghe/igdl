use igdl::progress::{
    ByteProgress, ImageProgressDisplay, ImageProgressState, ProgressOutputMode,
    VideoProgressDisplay, render_dynamic_progress_bar, render_image_progress_rows,
    render_item_progress, render_non_tty_progress_line, render_overall_progress,
    render_video_progress, select_progress_output_mode,
};
use std::time::Duration;

#[test]
fn formats_overall_progress_as_completed_over_total() {
    assert_eq!(render_overall_progress(2, 5), "2/5");
}

#[test]
fn formats_item_progress_with_percentage() {
    assert_eq!(
        render_item_progress(2, 5, Some(63), None),
        "Downloading 2/5: 63%"
    );
}

#[test]
fn formats_item_progress_with_bytes() {
    assert_eq!(
        render_item_progress(
            4,
            7,
            None,
            Some(ByteProgress {
                downloaded_bytes: 8_200_000,
                total_bytes: Some(14_700_000),
            }),
        ),
        "Downloading 4/7: 8.2 MB / 14.7 MB"
    );
}

#[test]
fn promotes_units_when_rounding_reaches_the_next_boundary() {
    assert_eq!(
        render_item_progress(
            1,
            1,
            None,
            Some(ByteProgress {
                downloaded_bytes: 999_950,
                total_bytes: None,
            }),
        ),
        "Downloading 1/1: 1.0 MB"
    );
}

#[test]
fn renders_dynamic_progress_bar_with_speed_and_eta() {
    assert_eq!(
        render_dynamic_progress_bar(sample_video_progress(), 20),
        "[########------------] 42% | 8.3 MB/s | ETA 00:12"
    );
}

#[test]
fn renders_non_tty_progress_line_with_bytes_speed_and_eta() {
    assert_eq!(
        render_non_tty_progress_line(sample_video_progress()),
        "42% | 5.5 MB / 13.0 MB | 8.3 MB/s | ETA 00:12"
    );
}

#[test]
fn selects_output_mode_from_tty_state() {
    assert_eq!(
        select_progress_output_mode(true),
        ProgressOutputMode::Interactive
    );
    assert_eq!(
        select_progress_output_mode(false),
        ProgressOutputMode::Plain
    );
}

#[test]
fn renders_video_progress_using_the_selected_output_mode() {
    let progress = sample_video_progress();

    assert_eq!(
        render_video_progress(progress, ProgressOutputMode::Interactive, 20),
        "[########------------] 42% | 8.3 MB/s | ETA 00:12"
    );
    assert_eq!(
        render_video_progress(progress, ProgressOutputMode::Plain, 20),
        "42% | 5.5 MB / 13.0 MB | 8.3 MB/s | ETA 00:12"
    );
}

#[test]
fn renders_image_progress_rows_in_interactive_mode() {
    let rows = render_image_progress_rows(
        &[
            ImageProgressDisplay {
                item_id: "image-1".to_owned(),
                label: "01".to_owned(),
                state: ImageProgressState::Active(sample_video_progress()),
            },
            ImageProgressDisplay {
                item_id: "image-2".to_owned(),
                label: "02".to_owned(),
                state: ImageProgressState::Completed,
            },
        ],
        ProgressOutputMode::Interactive,
        20,
    );

    assert_eq!(
        rows,
        vec![
            "01 [########------------] 42% | 8.3 MB/s | ETA 00:12".to_owned(),
            "02 100% | done".to_owned(),
        ]
    );
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| !row.contains("Downloading")));
    assert!(rows.iter().all(|row| !row.contains("1/2")));
}

#[test]
fn renders_image_progress_rows_in_plain_mode() {
    let rows = render_image_progress_rows(
        &[
            ImageProgressDisplay {
                item_id: "image-1".to_owned(),
                label: "01".to_owned(),
                state: ImageProgressState::Active(sample_video_progress()),
            },
            ImageProgressDisplay {
                item_id: "image-2".to_owned(),
                label: "02".to_owned(),
                state: ImageProgressState::Completed,
            },
        ],
        ProgressOutputMode::Plain,
        20,
    );

    assert_eq!(
        rows,
        vec![
            "01 42% | 5.5 MB / 13.0 MB | 8.3 MB/s | ETA 00:12".to_owned(),
            "02 100% | done".to_owned(),
        ]
    );
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| !row.contains("2/2")));
}

fn sample_video_progress() -> VideoProgressDisplay {
    VideoProgressDisplay {
        percentage: Some(42),
        bytes: Some(ByteProgress {
            downloaded_bytes: 5_500_000,
            total_bytes: Some(13_000_000),
        }),
        speed_bytes_per_second: Some(8_300_000),
        eta: Some(Duration::from_secs(12)),
    }
}
