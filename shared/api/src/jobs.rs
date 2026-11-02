//! Long-running work a client starts and watches on the event stream.

/// One finished or failed URL download, as the downloader page lists them.
/// `format` is the id of one of [`crate::JobApi::download_formats`]'s options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadHistoryEntry {
    pub url: String,
    pub title: String,
    pub format: String,
    pub state: DownloadState,
    pub error: Option<String>,
}

/// One song the downloader found, ready to be handed back as `url`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadCandidate {
    pub url: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u64,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DownloadState {
    #[default]
    Finished,
    Failed,
}

/// Where one requested download has got to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DownloadItemState {
    #[default]
    Queued,
    Downloading,
    Failed,
}

/// One requested download, as the progress overlay renders it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadItemStatus {
    pub key: String,
    pub state: DownloadItemState,
}
