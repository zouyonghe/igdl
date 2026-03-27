use thiserror::Error;

#[derive(Debug, Error)]
pub enum IgdlError {
    #[error("HOME environment variable is not set")]
    HomeDirectoryUnavailable,
    #[error("unsupported url: {0}")]
    UnsupportedUrl(String),
    #[error("browser cookies unavailable: {}", .0.join("; "))]
    BrowserCookiesUnavailable(Vec<String>),
    #[error("download produced no files")]
    DownloadProducedNoFiles,
    #[error("{0}")]
    YtDlpBootstrap(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
