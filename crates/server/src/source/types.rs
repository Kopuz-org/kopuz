use std::fmt;

use crate::ytmusic::player::AudioFormat;

/// Why a media-source operation failed, classified so the UI can react
/// differently instead of pattern-matching opaque strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    Unsupported(&'static str),
    Connectivity,
    Auth,
    InvalidInput(String),
    Backend(String),
}

impl SourceError {
    pub fn unsupported(op: &'static str) -> Self {
        SourceError::Unsupported(op)
    }

    /// A request that got no answer, not one the server refused.
    pub fn from_request(error: reqwest::Error) -> Self {
        if error.is_connect() || error.is_timeout() {
            SourceError::Connectivity
        } else {
            SourceError::Backend(error.without_url().to_string())
        }
    }

    /// A non-success HTTP status, where 401 and 403 mean the credentials were refused.
    pub fn from_status(status: reqwest::StatusCode, what: &str) -> Self {
        match status.as_u16() {
            401 | 403 => SourceError::Auth,
            _ => SourceError::Backend(format!("{what} failed: {status}")),
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Unsupported(op) => write!(f, "this source doesn't support {op}"),
            SourceError::Connectivity => f.write_str("the server has no active connection"),
            SourceError::Auth => {
                f.write_str("this source isn't signed in - open Settings to re-sign in")
            }
            SourceError::InvalidInput(m) | SourceError::Backend(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<String> for SourceError {
    fn from(m: String) -> Self {
        SourceError::Backend(m)
    }
}

impl From<db::DbError> for SourceError {
    fn from(e: db::DbError) -> Self {
        SourceError::Backend(e.to_string())
    }
}

/// One batch of a radio. The cursor is opaque to everyone but the source that
/// issued it, and is stored with the queue so a restart picks the radio up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RadioPage {
    pub tracks: Vec<reader::Track>,
    pub more: Option<String>,
}

impl RadioPage {
    pub fn last(tracks: Vec<reader::Track>) -> Self {
        Self { tracks, more: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaylistOps {
    None,
    AddRemove,
    Reorder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistView {
    Library,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FavoritesSync {
    Instant,
    Paginated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumType {
    Standard,
    YtMusic,
}

/// Which seeds a source can generate a radio/mix from. The two are separate
/// because backends implement them separately: the Subsonic API has a
/// song-seeded similar-songs call but nothing playlist-seeded, so a single flag
/// made the playlist cards offer an action that could only ever fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RadioSeeds {
    /// Backs [`MediaSource::start_radio`](super::MediaSource::start_radio).
    pub track: bool,
    /// Backs [`MediaSource::start_playlist_radio`](super::MediaSource::start_playlist_radio).
    pub playlist: bool,
    /// Playing a search result starts a radio from it instead of queueing the
    /// results: a catalog search is a list of unrelated matches, not an album.
    /// Needs `track`.
    pub search: bool,
}

impl RadioSeeds {
    /// No radio at all, the default for a source that overrides neither op.
    pub const NONE: Self = Self {
        track: false,
        playlist: false,
        search: false,
    };

    /// A song seed only, the Subsonic/OpenSubsonic shape.
    pub const TRACK: Self = Self {
        track: true,
        playlist: false,
        search: false,
    };

    /// Both seeds, the catalog-remote shape.
    pub const ALL: Self = Self {
        track: true,
        playlist: true,
        search: false,
    };
}

pub struct FavoritesPage {
    pub tracks: Vec<reader::Track>,
    pub next: Option<String>,
}

pub struct PlaylistPage {
    pub tracks: Vec<reader::Track>,
    pub next: Option<String>,
}

/// What looking one artist up found: a photo, and the name the source's own record gives when the lookup went by id.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ArtistLookup {
    pub image: Option<String>,
    pub name: Option<String>,
}

#[derive(Default)]
pub struct LibrarySnapshot {
    pub albums: Vec<reader::Album>,
    pub tracks: Vec<reader::Track>,
    /// The source's own artist listing, with each photo URL it has.
    pub artists: Vec<(reader::ArtistCredit, Option<String>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub edit_tags: bool,
    pub delete_from_disk: bool,
    pub scan_folders: bool,
    /// Playlists can be filed in folders.
    pub folders: bool,
    /// Its library is a directory tree to pick roots from, not a catalog.
    pub browse_folders: bool,
    /// It plays on devices of its own, which a client can list and move to.
    pub external_devices: bool,
    /// It plays through a browser on the host, not the engine.
    pub browser_playback: bool,
    pub sync: bool,
    pub downloads: bool,
    pub uploads: bool,
    pub storage_quota: bool,
    pub discover: bool,
    /// The person can tell it to stop recommending a track — a negative
    /// signal to the source's recommender, not a library edit.
    pub dont_recommend: bool,
    pub radio: RadioSeeds,
    /// It keeps a play queue of its own that other clients read and write.
    pub play_queue: bool,
    pub playlists: PlaylistOps,
    pub artist_view: ArtistView,
    pub albums: AlbumType,
    pub favorites_sync: FavoritesSync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    Valid,
    Expired,
    Unreachable,
}

pub struct StreamInfo {
    pub url: String,
    pub format: Option<(AudioFormat, bool)>,
    pub user_agent: Option<String>,
    pub duration_secs: Option<u64>,
    pub bitrate: Option<u32>,
    pub content_length: Option<u64>,
    pub loudness_db: Option<f32>,
}

pub struct PlaylistMeta {
    pub id: String,
    pub name: String,
    pub image_tag: Option<String>,
}

/// What kind of release a source files an album as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseKind {
    Album,
    Single,
    Ep,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteAlbum {
    pub browse_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub year: Option<String>,
    /// The kind the source reports; `None` when it reports none it names.
    pub kind: Option<ReleaseKind>,
    pub thumbnail: Option<String>,
    pub audio_playlist_id: Option<String>,
    pub tracks: Vec<reader::Track>,
}

impl From<crate::ytmusic::discover::YtAlbum> for RemoteAlbum {
    fn from(a: crate::ytmusic::discover::YtAlbum) -> Self {
        Self {
            browse_id: a.browse_id,
            title: a.title,
            artist: a.artist,
            artist_id: a.artist_id,
            year: a.year,
            kind: a.kind,
            thumbnail: a.thumbnail,
            audio_playlist_id: a.audio_playlist_id,
            tracks: a.tracks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageQuota {
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub remaining_bytes: u64,
}

pub struct RemotePlayQueue {
    pub tracks: Vec<reader::Track>,
    /// The playing track's item id, matched against `tracks[i].id.key()`.
    pub current_id: Option<String>,
    pub position_ms: u64,
    pub changed_by: Option<String>,
}
