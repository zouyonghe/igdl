use crate::error::IgdlError;
use reqwest::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstagramUrlKind {
    Reel,
    PostMedia,
}

pub fn validate_instagram_url(url: &str) -> Result<(), IgdlError> {
    instagram_url_kind(url).map(|_| ())
}

pub fn instagram_url_kind(url: &str) -> Result<InstagramUrlKind, IgdlError> {
    let parsed = Url::parse(url).map_err(|_| unsupported_url(url))?;

    let host = parsed
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| unsupported_url(url))?;

    let valid_host = matches!(
        host.as_str(),
        "instagram.com" | "www.instagram.com" | "instagr.am" | "www.instagr.am"
    );
    if !valid_host {
        return Err(unsupported_url(url));
    }

    match parsed
        .path_segments()
        .and_then(|mut segments| segments.next())
    {
        Some("reel") | Some("reels") => Ok(InstagramUrlKind::Reel),
        Some("p") | Some("tv") => Ok(InstagramUrlKind::PostMedia),
        _ => Err(unsupported_url(url)),
    }
}

fn unsupported_url(url: &str) -> IgdlError {
    IgdlError::UnsupportedUrl(format!("Instagram URL required: {url}"))
}
