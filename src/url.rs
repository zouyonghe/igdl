use crate::error::IgdlError;
use reqwest::Url;

pub fn validate_instagram_url(url: &str) -> Result<(), IgdlError> {
    let parsed = Url::parse(url).map_err(|_| unsupported_url(url))?;

    let host = parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| unsupported_url(url))?;

    let valid_host = matches!(
        host.as_str(),
        "instagram.com" | "www.instagram.com" | "instagr.am" | "www.instagr.am"
    );

    let valid_path = parsed
        .path_segments()
        .and_then(|mut segments| segments.next())
        .map(|segment| matches!(segment, "reel" | "reels" | "p" | "tv"))
        .unwrap_or(false);

    if valid_host && valid_path {
        Ok(())
    } else {
        Err(unsupported_url(url))
    }
}

fn unsupported_url(url: &str) -> IgdlError {
    IgdlError::UnsupportedUrl(format!("Instagram URL required: {url}"))
}
