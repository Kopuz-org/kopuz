use crate::player::TrackKind;

/// Library ordering. `Default` is the daemon's own choice, so a caller that
/// does not care says nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TrackSort {
    #[default]
    Default,
    Title,
    Artist,
    Album,
    DateAdded,
    PlayCount,
    /// Stacked user criteria (the library's sort control): the first field
    /// decides, the rest break ties. Empty means [`TrackSort::Default`].
    Fields(Vec<config::SortCriterion<config::TrackSortField>>),
}

/// A track row on the wire. `key` is the stable library ref used everywhere
/// else in the API; local filesystem paths and credentialed remote URLs never
/// appear here.
///
/// This is what frontends render, so it carries everything a row displays --
/// including `artwork`, which says whether a cover exists at all.
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
    /// Which service the track came from; `None` for a local file.
    pub service: Option<config::MusicService>,
    /// The file's container, upper-cased ("FLAC"), for a local track that has
    /// one. A row from a service names no file, so it has none.
    pub format: Option<String>,
    /// Every credited artist, where the source distinguishes them from the
    /// single `artist` string.
    pub artists: Vec<String>,
    pub musicbrainz_release_id: Option<String>,
    pub musicbrainz_recording_id: Option<String>,
    pub musicbrainz_track_id: Option<String>,
    /// A playlist's own id for this entry, when the row came from one and the
    /// source distinguishes duplicate entries.
    pub playlist_item_id: Option<String>,
    pub artwork: Option<crate::ArtworkRef>,
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
    pub sort: TrackSort,
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

/// An album row on the wire. `artwork` is present when the daemon can resolve
/// a cover; clients fetch it through [`crate::ArtworkApi`] rather than
/// composing a URL, since the daemon holds the credentials that sign one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumInfo {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub genre: String,
    pub year: u16,
    pub artwork: Option<crate::ArtworkRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumPage {
    pub albums: Vec<AlbumInfo>,
    pub total: u32,
}

/// An artist and how many tracks the library holds for them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistInfo {
    pub name: String,
    pub track_count: u32,
    pub artwork: Option<crate::ArtworkRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistPage {
    pub artists: Vec<ArtistInfo>,
    pub total: u32,
}

/// What a search turned up. Remote sources answer over the network, so this
/// is one call rather than a filter the caller composes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchResults {
    pub tracks: Vec<TrackInfo>,
    pub albums: Vec<AlbumInfo>,
}

impl TrackInfo {
    /// Whole seconds, which is how a duration is shown. A radio stream has
    /// none: it plays until it is stopped.
    pub fn duration_secs(&self) -> Option<u64> {
        match self.kind {
            TrackKind::Radio => None,
            TrackKind::Normal => Some(self.duration_ms.unwrap_or_default() / 1000),
        }
    }

    pub fn is_radio(&self) -> bool {
        self.kind == TrackKind::Radio
    }
}
