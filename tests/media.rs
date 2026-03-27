use igdl::media::build_media_filename;

#[test]
fn builds_single_item_filename_with_shortcode_and_extension() {
    let name = build_media_filename("Sunset at the beach", "CrABC12", None, "jpg");

    assert_eq!(name, "sunset-at-the-beach-CrABC12.jpg");
}

#[test]
fn numbers_carousel_items_in_order() {
    assert_eq!(
        build_media_filename("Weekend dump", "CrABC12", Some(1), "jpg"),
        "weekend-dump-CrABC12-01.jpg"
    );
    assert_eq!(
        build_media_filename("Weekend dump", "CrABC12", Some(2), "jpg"),
        "weekend-dump-CrABC12-02.jpg"
    );
    assert_eq!(
        build_media_filename("Weekend dump", "CrABC12", Some(3), "jpg"),
        "weekend-dump-CrABC12-03.jpg"
    );
}

#[test]
fn sanitizes_caption_fragments_deterministically() {
    let name = build_media_filename("  wow... neat / clip!!!  ", "CrABC12", None, "mp4");

    assert_eq!(name, "wow-neat-clip-CrABC12.mp4");
}

#[test]
fn falls_back_to_media_when_fragment_is_empty_after_sanitizing() {
    let name = build_media_filename("!!!", "CrABC12", None, "webp");

    assert_eq!(name, "media-CrABC12.webp");
}

#[test]
fn truncates_long_sanitized_fragments_predictably() {
    let name = build_media_filename(&"A".repeat(400), "CrABC12", None, "jpg");

    assert_eq!(name, format!("{}-CrABC12.jpg", "a".repeat(243)));
    assert_eq!(name.len(), 255);
}
