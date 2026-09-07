use crate::player::TrackKind;

/// A track row on the wire. `key` is the stable library ref used everywhere
/// else in the API, and the entity id `GetArtwork` takes; local filesystem
/// paths and credentialed remote URLs never appear here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackInfo {
    pub key: String,
    pub uid: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: String,
    pub duration_ms: Option<u64>,
    pub khz: u32,
    pub bitrate: u16,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub kind: TrackKind,
    pub seekable: bool,
    pub offline: bool,
}

pub const DEFAULT_PAGE_LIMIT: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackFilter {
    pub search: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub favorite: Option<bool>,
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricChunkView {
    pub start_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricLineView {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
    pub chunks: Vec<LyricChunkView>,
    pub parent_line_index: Option<u32>,
    pub background: bool,
    pub opposite_turn: bool,
}

/// Lyrics for one track: `synced` when timing exists (chunks carry word or
/// syllable timing where the provider has it), `plain` otherwise.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricsView {
    pub plain: Option<String>,
    pub synced: Vec<LyricLineView>,
}

/// Listening stats: play counts keyed by track uid.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatsView {
    pub listen_counts: std::collections::HashMap<String, u64>,
}

/// A window into a filtered track listing. `total` always reflects the whole
/// filtered set so clients can paginate without a second count request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackPage {
    pub total: u32,
    pub offset: u32,
    pub items: Vec<TrackInfo>,
}
