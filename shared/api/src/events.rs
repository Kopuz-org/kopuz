use crate::error::ErrorBody;
use crate::player::{BufferedRange, PlayerState};

/// The invalidation tables, mirroring `hooks::db_reactivity::Table`. A
/// `library.invalidated` event tells clients to re-run reads that depend on
/// the table.
/// Fallback variants absorb values added in later daemon versions, so a known
/// event with an unknown enum value degrades instead of being dropped. The
/// daemon never serializes `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Table {
    Tracks,
    Albums,
    Playlists,
    Favorites,
    Folders,
    Servers,
    Recents,
    /// The radio station list, including which stations are pinned.
    Stations,
    /// Artist rows on their own: their photos and the names their source gives them.
    Artists,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Scan,
    LibrarySync,
    FavoritesSync,
    PlaylistSync,
    Download,
    UrlDownload,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobProgress {
    pub id: String,
    pub kind: JobKind,
    pub phase: String,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub message: Option<String>,
    /// Started by the daemon's own schedule rather than by a user.
    pub automatic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    /// The daemon is probing it and has no answer yet.
    Checking,
    Online,
    AuthExpired,
    Offline,
}

/// Whether the daemon reaches the internet, as its last check after a source stopped answering found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkStatus {
    /// Every connectivity check failed, so only sources on this machine or its network are usable.
    pub internet_down: bool,
    /// The source the daemon moved off when the internet went down, kept so a frontend can offer it back once it is up.
    pub left_source: Option<String>,
    pub offline_mode: OfflineMode,
}

/// Whether listings and playback keep to what plays without the internet: downloads and cached songs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OfflineMode {
    #[default]
    Off,
    /// The internet went down with nothing on this machine or its network to play; once it is back a frontend offers to leave, and `set_offline(false)` does.
    Automatic,
    /// Asked for, by `--offline` or `set_offline(true)`, and left only by `set_offline(false)`.
    Requested,
}

impl OfflineMode {
    pub fn is_on(self) -> bool {
        self != Self::Off
    }
}

/// The offline playback cache: songs kept from their own stream, least recently played evicted first past the cap; downloads are apart and never count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OfflineCacheStatus {
    /// Off stops adding songs; the ones it holds stay and still play.
    pub enabled: bool,
    pub max_bytes: u64,
    pub used_bytes: u64,
    pub tracks: u32,
}

/// One event on the daemon's Subscribe stream (`proto/kopuz.proto` carries the
/// wire shape). The variant names are the event identities. Clients
/// must ignore unknown event types.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiEvent {
    PlayerState(Box<PlayerState>),
    PlayerPosition {
        token: u64,
        position_ms: u64,
        at_ms: u64,
        playing: bool,
    },
    PlayerBuffered {
        token: u64,
        ranges: Vec<BufferedRange>,
    },
    QueueChanged {
        rev: u64,
        length: u32,
        index: Option<u32>,
    },
    /// The named table changed; re-read what you hold from it. A pure dirty
    /// signal, deliberately carrying no version: the writers are independent
    /// services with no shared counter to report.
    LibraryInvalidated {
        table: Table,
    },
    JobProgress(JobProgress),
    JobFinished {
        id: String,
        kind: JobKind,
        ok: bool,
        error: Option<ErrorBody>,
        /// Started by the daemon's own schedule rather than by a user.
        automatic: bool,
    },
    /// The settings moved; re-read the view, whose revision this is.
    ConfigChanged {
        revision: u64,
    },
    /// A frontend's stored preferences changed (set or deleted) in these keys; re-read what you hold.
    FrontendPrefsChanged {
        frontend: String,
        keys: Vec<String>,
    },
    SourceStatus {
        source: String,
        state: SourceState,
    },
    /// The internet went down or came back, or the source offered back changed.
    NetworkStatus(NetworkStatus),
    Notice {
        level: NoticeLevel,
        code: String,
        message: Option<String>,
    },
    Resync,
}
