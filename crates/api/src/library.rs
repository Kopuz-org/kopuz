use serde::{Deserialize, Serialize};

use crate::player::TrackKind;

/// A track row on the wire. `key` is the stable library ref used everywhere
/// else in the API; `artwork` is a daemon-relative URL (append your bearer
/// token as `?token=` when loading from an <img> tag); local filesystem
/// paths and credentialed remote URLs never appear here.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub key: String,
    pub uid: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub khz: u32,
    pub bitrate: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disc_number: Option<u32>,
    pub kind: TrackKind,
    pub seekable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork: Option<String>,
    pub offline: bool,
}

pub const DEFAULT_PAGE_LIMIT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub offset: u32,
    pub limit: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: DEFAULT_PAGE_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricChunkView {
    pub start_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricLineView {
    pub start_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<LyricChunkView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_line_index: Option<u32>,
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub opposite_turn: bool,
}

/// Lyrics for one track: `synced` when timing exists (chunks carry word or
/// syllable timing where the provider has it), `plain` otherwise.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LyricsView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plain: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub synced: Vec<LyricLineView>,
}

/// Listening stats: play counts keyed by track uid.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatsView {
    pub listen_counts: std::collections::HashMap<String, u64>,
}

/// A window into a filtered track listing. `total` always reflects the whole
/// filtered set so clients can paginate without a second count request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackPage {
    pub total: u32,
    pub offset: u32,
    pub items: Vec<TrackInfo>,
}
