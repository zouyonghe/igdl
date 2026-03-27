use clap::Parser;
use igdl::cli::{BrowserArg, CliArgs};
use igdl::url::validate_instagram_url;

#[test]
fn parses_required_url_and_optional_flags() {
    let args = CliArgs::parse_from([
        "igdl",
        "https://www.instagram.com/reel/abc123/",
        "--browser",
        "chrome",
        "--output",
        "/tmp/instagram",
        "--verbose",
    ]);

    assert_eq!(args.url.as_str(), "https://www.instagram.com/reel/abc123/");
    assert_eq!(args.browser, Some(BrowserArg::Chrome));
    assert_eq!(
        args.output.as_deref(),
        Some(std::path::Path::new("/tmp/instagram"))
    );
    assert!(args.verbose);
}

#[test]
fn accepts_raw_url_string_without_parsing() {
    let args = CliArgs::try_parse_from(["igdl", "not-a-url"]).unwrap();

    assert_eq!(args.url, "not-a-url");
}

#[test]
fn rejects_non_instagram_urls() {
    let err = validate_instagram_url("https://example.com/video").unwrap_err();
    assert!(err.to_string().contains("Instagram"));
}

#[test]
fn rejects_lookalike_instagram_hosts() {
    let err = validate_instagram_url("https://notinstagram.com/p/abc/").unwrap_err();
    assert!(err.to_string().contains("Instagram"));
}

#[test]
fn rejects_instagram_urls_hidden_in_query_strings() {
    let err = validate_instagram_url("https://example.com/?next=instagram.com/reel/x").unwrap_err();
    assert!(err.to_string().contains("Instagram"));
}

#[test]
fn accepts_reel_and_post_urls() {
    assert!(validate_instagram_url("https://www.instagram.com/reel/abc123/").is_ok());
    assert!(validate_instagram_url("https://www.instagram.com/p/abc123/").is_ok());
}
