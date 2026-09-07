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
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Scan,
    LibrarySync,
    FavoritesSync,
    PlaylistSync,
    Download,
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
    Online,
    AuthExpired,
    Offline,
}

/// One event on the daemon's Subscribe stream (`proto/kopuz.proto` carries the
/// wire shape). The serde tags are the stable event identities. Clients
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
    LibraryInvalidated {
        table: Table,
        generation: u64,
    },
    JobProgress(JobProgress),
    JobFinished {
        id: String,
        kind: JobKind,
        ok: bool,
        error: Option<ErrorBody>,
    },
    ConfigChanged {
        keys: Vec<String>,
    },
    SourceStatus {
        source: String,
        state: SourceState,
    },
    Notice {
        level: NoticeLevel,
        code: String,
        message: Option<String>,
    },
    Resync,
}
