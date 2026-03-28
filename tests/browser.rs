use igdl::browser::{Browser, browser_attempts};
use igdl::cli::BrowserArg;

#[test]
fn uses_expected_default_browser_order() {
    assert_eq!(
        browser_attempts(None),
        vec![
            Browser::Chrome,
            Browser::Edge,
            Browser::Brave,
            Browser::Firefox,
            Browser::Safari,
        ]
    );
}

#[test]
fn narrows_attempts_when_browser_is_explicit() {
    assert_eq!(
        browser_attempts(Some(Browser::Firefox)),
        vec![Browser::Firefox]
    );
}

#[test]
fn maps_browser_variants_to_ytdlp_names() {
    assert_eq!(Browser::Chrome.as_ytdlp_arg(), "chrome");
    assert_eq!(Browser::Edge.as_ytdlp_arg(), "edge");
    assert_eq!(Browser::Brave.as_ytdlp_arg(), "brave");
    assert_eq!(Browser::Firefox.as_ytdlp_arg(), "firefox");
    assert_eq!(Browser::Safari.as_ytdlp_arg(), "safari");
}

#[test]
fn converts_cli_browser_arg_to_browser() {
    assert_eq!(Browser::from(BrowserArg::Chrome), Browser::Chrome);
    assert_eq!(Browser::from(BrowserArg::Edge), Browser::Edge);
    assert_eq!(Browser::from(BrowserArg::Brave), Browser::Brave);
    assert_eq!(Browser::from(BrowserArg::Firefox), Browser::Firefox);
    assert_eq!(Browser::from(BrowserArg::Safari), Browser::Safari);
}
