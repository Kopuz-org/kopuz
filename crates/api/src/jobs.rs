//! Long-running work a client starts and watches on the event stream.

/// What yt-dlp should produce. `Video` keeps the picture; the rest extract
/// audio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum YtdlpAudioFormat {
    #[default]
    BestAudio,
    Mp3,
    Flac,
    Opus,
    Wav,
    Video,
}

/// A yt-dlp download. The options are the typed settings struct, not a bag of
/// JSON: the daemon builds the command line, so it has to understand them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct YtdlpRequest {
    pub url: String,
    /// Empty means the daemon's default download location.
    pub output_dir: String,
    pub format: YtdlpAudioFormat,
    pub options: config::YtdlpOptions,
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
