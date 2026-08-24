use serde::{Deserialize, Serialize};

use crate::library::TrackFilter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueMode {
    #[default]
    Replace,
    Append,
    PlayNext,
}

/// What to put in the queue. Reference-shaped contexts are materialized by
/// the daemon from its own database, so "play this album" never round-trips
/// the track list through the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetQueueRequest {
    #[serde(default)]
    pub mode: QueueMode,
    pub context: QueueContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shuffle: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueItem {
    pub index: u32,
    pub track: reader::Track,
}

/// A window into the queue in play order. `rev` matches
/// `QueueSummary::rev`; a `queue.changed` event with a newer `rev` means the
/// window is stale.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueWindow {
    pub rev: u64,
    pub total: u32,
    pub offset: u32,
    pub items: Vec<QueueItem>,
}
