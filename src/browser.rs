use crate::cli::BrowserArg;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Browser {
    Chrome,
    Edge,
    Brave,
    Firefox,
    Safari,
}

impl Browser {
    pub fn as_ytdlp_arg(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::Brave => "brave",
            Self::Firefox => "firefox",
            Self::Safari => "safari",
        }
    }
}

impl fmt::Display for Browser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str((*self).as_ytdlp_arg())
    }
}

pub fn browser_attempts(selected: Option<Browser>) -> Vec<Browser> {
    match selected {
        Some(browser) => vec![browser],
        None => vec![
            Browser::Chrome,
            Browser::Edge,
            Browser::Brave,
            Browser::Firefox,
            Browser::Safari,
        ],
    }
}

impl From<BrowserArg> for Browser {
    fn from(value: BrowserArg) -> Self {
        match value {
            BrowserArg::Chrome => Self::Chrome,
            BrowserArg::Edge => Self::Edge,
            BrowserArg::Brave => Self::Brave,
            BrowserArg::Firefox => Self::Firefox,
            BrowserArg::Safari => Self::Safari,
        }
    }
}
