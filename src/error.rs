use thiserror::Error;

#[derive(Debug, Error)]
pub enum IgdlError {
    #[error("HOME environment variable is not set")]
    HomeDirectoryUnavailable,
    #[error("unsupported url: {0}")]
    UnsupportedUrl(String),
    #[error("browser cookies unavailable: {}", .0.join("; "))]
    BrowserCookiesUnavailable(Vec<String>),
    #[error("media download failed: {}", .0.join("; "))]
    PostMediaDownloadFailed(Vec<String>),
    #[error("downloaded {downloaded} of {total} media items; {failed} failed")]
    PostMediaDownloadPartial {
        downloaded: usize,
        failed: usize,
        total: usize,
    },
    #[error("download produced no files")]
    DownloadProducedNoFiles,
    #[error("{0}")]
    YtDlpBootstrap(String),
    #[error("{0}")]
    GalleryDlBootstrap(String),
    #[error("{0}")]
    MediaDownload(String),
    #[error("missing download binary for {0}")]
    MissingDownloadBinary(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
