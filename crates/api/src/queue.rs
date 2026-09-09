use crate::library::TrackFilter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum QueueMode {
    #[default]
    Replace,
    Append,
    PlayNext,
}

/// What to put in the queue. Reference-shaped contexts are materialized by
/// the daemon from its own database, so "play this album" never round-trips
/// the track list through the client.
#[derive(Debug, Clone, PartialEq)]
pub enum QueueContext {
    Tracks {
        keys: Vec<String>,
    },
    Album {
        id: String,
    },
    Artist {
        name: String,
    },
    Genre {
        name: String,
    },
    Playlist {
        id: String,
    },
    Filter {
        filter: TrackFilter,
    },
    Radio {
        station_id: String,
        stream_id: String,
    },
    /// The source's "start a mix from this track" feed. The daemon fetches it
    /// and pins the seed at the front, so the caller does not have to merge
    /// the seed into a list the source returned in its own order.
    TrackRadio {
        key: String,
    },
    PlaylistRadio {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetQueueRequest {
    pub mode: QueueMode,
    pub context: QueueContext,
    pub start_index: Option<u32>,
    pub shuffle: Option<bool>,
}

/// In-place queue edits. Positions are play-order (logical) indices, the same
/// space `QueueSummary::index` and `QueueItem::index` use -- except
/// [`QueueEdit::JumpPhysical`], which says so in its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueEdit {
    Jump {
        index: u32,
    },
    /// Jump by position in the unshuffled queue, re-pinning the shuffle order
    /// around it. What a click on a track list means while shuffle is on: the
    /// clicked row plays, and the rest is reshuffled after it.
    JumpPhysical {
        index: u32,
    },
    Move {
        from: u32,
        to: u32,
    },
    Remove {
        index: u32,
    },
    /// Put tracks at a play-order position without disturbing what plays now.
    Insert {
        index: u32,
        keys: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueueItem {
    pub index: u32,
    pub track: crate::library::TrackInfo,
}

/// A window into the queue in play order. `rev` matches
/// `QueueSummary::rev`; a `queue.changed` event with a newer `rev` means the
/// window is stale.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueueWindow {
    pub rev: u64,
    pub total: u32,
    pub offset: u32,
    pub items: Vec<QueueItem>,
}

/// The whole queue in one answer: the rows in play order, the shuffle
/// permutation behind them, and where playback sits.
///
/// Paged windows are the right shape for a list a user scrolls; they are the
/// wrong shape for a frontend that reads the queue synchronously while
/// rendering (every "is this track queued?" check, the shuffle order, the
/// drag-and-drop model). Queues are bounded by what a person queues, so the
/// whole thing fits in one message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueueSnapshot {
    pub rev: u64,
    /// Play order: index 0 plays first.
    pub items: Vec<crate::library::TrackInfo>,
    /// For each play-order position, the index it has in the unshuffled
    /// queue. Empty while shuffle is off.
    pub shuffle_order: Vec<u32>,
    pub position: Option<u32>,
    pub shuffle: bool,
}
