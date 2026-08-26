use serde::{Deserialize, Serialize};

use crate::error::ErrorBody;
use crate::player::{BufferedRange, PlayerState};

/// The invalidation tables, mirroring `hooks::db_reactivity::Table`. A
/// `library.invalidated` event tells clients to re-run reads that depend on
/// the table.
/// Fallback variants absorb values added in later daemon versions, so a known
/// event with an unknown enum value degrades instead of being dropped. The
/// daemon never serializes `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Table {
    Tracks,
    Albums,
    Playlists,
    Favorites,
    Folders,
    Servers,
    Recents,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Scan,
    LibrarySync,
    FavoritesSync,
    PlaylistSync,
    Download,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobProgress {
    pub id: String,
    pub kind: JobKind,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    #[serde(other)]
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Online,
    AuthExpired,
    #[serde(other)]
    Offline,
}

/// One event on the daemon's Attach stream (`proto/kopuz.proto` carries the
/// wire shape). The serde tags are the stable event identities. Clients
/// must ignore unknown event types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum ApiEvent {
    #[serde(rename = "player.state")]
    PlayerState(Box<PlayerState>),
    #[serde(rename = "player.position")]
    PlayerPosition {
        token: u64,
        position_ms: u64,
        at_ms: u64,
        playing: bool,
    },
    #[serde(rename = "player.buffered")]
    PlayerBuffered {
        token: u64,
        ranges: Vec<BufferedRange>,
    },
    #[serde(rename = "queue.changed")]
    QueueChanged {
        rev: u64,
        length: u32,
        index: Option<u32>,
    },
    #[serde(rename = "library.invalidated")]
    LibraryInvalidated { table: Table, generation: u64 },
    #[serde(rename = "job.progress")]
    JobProgress(JobProgress),
    #[serde(rename = "job.finished")]
    JobFinished {
        id: String,
        kind: JobKind,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ErrorBody>,
    },
    #[serde(rename = "config.changed")]
    ConfigChanged { keys: Vec<String> },
    #[serde(rename = "source.status")]
    SourceStatus { source: String, state: SourceState },
    #[serde(rename = "notice")]
    Notice {
        level: NoticeLevel,
        code: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    #[serde(rename = "resync")]
    Resync,
}
