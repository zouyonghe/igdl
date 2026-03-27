use igdl::progress::{render_item_progress, render_overall_progress, ByteProgress};

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
