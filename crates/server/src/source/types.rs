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

pub struct FavoritesPage {
    pub tracks: Vec<reader::Track>,
    pub next: Option<String>,
}

pub struct PlaylistPage {
    pub tracks: Vec<reader::Track>,
    pub next: Option<String>,
}

#[derive(Default)]
pub struct LibrarySnapshot {
    pub albums: Vec<reader::Album>,
    pub tracks: Vec<reader::Track>,
    pub artist_images: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub edit_tags: bool,
    pub delete_from_disk: bool,
    pub scan_folders: bool,
    pub folders: bool,
    pub sync: bool,
    pub downloads: bool,
    pub discover: bool,
    pub radio: bool,
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
}

pub struct PlaylistMeta {
    pub id: String,
    pub name: String,
    pub image_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteAlbum {
    pub browse_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub year: Option<String>,
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
            year: a.year,
            thumbnail: a.thumbnail,
            audio_playlist_id: a.audio_playlist_id,
            tracks: a.tracks,
        }
    }
}
