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
    /// What the first page's header says about the playlist, where the
    /// source reads one.
    pub header: Option<crate::ytmusic::discover::BrowsePage>,
    /// What this one playlist allows, where the source can tell it apart
    /// from its others; read off the first page. `None` leaves
    /// [`Capabilities::playlists`] to say.
    pub ops: Option<PlaylistOps>,
}

/// The account actions a source takes beyond favorites, each backing one
/// optional [`MediaSource`](super::MediaSource) op.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LibraryActions {
    /// Backs [`MediaSource::rate`](super::MediaSource::rate).
    pub rate: bool,
    /// Backs [`MediaSource::follow`](super::MediaSource::follow).
    pub follow: bool,
    /// Backs [`MediaSource::save`](super::MediaSource::save).
    pub save: bool,
    /// Backs [`MediaSource::remove_from_history`](super::MediaSource::remove_from_history).
    pub remove_from_history: bool,
    /// Backs [`MediaSource::edit_playlist`](super::MediaSource::edit_playlist).
    pub playlist_details: bool,
}

impl LibraryActions {
    pub const NONE: Self = Self {
        rate: false,
        follow: false,
        save: false,
        remove_from_history: false,
        playlist_details: false,
    };

    pub const ALL: Self = Self {
        rate: true,
        follow: true,
        save: true,
        remove_from_history: true,
        playlist_details: true,
    };
}

/// A playlist's details to push. A field left `None` keeps its value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistDetails {
    pub name: Option<String>,
    pub description: Option<String>,
    pub privacy: Option<crate::ytmusic::discover::Privacy>,
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
    pub artist_images: Vec<(reader::ArtistCredit, String)>,
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
    pub discover: bool,
    /// The person can tell it to stop recommending a track — a negative
    /// signal to the source's recommender, not a library edit.
    pub dont_recommend: bool,
    pub radio: RadioSeeds,
    pub playlists: PlaylistOps,
    pub artist_view: ArtistView,
    pub albums: AlbumType,
    pub favorites_sync: FavoritesSync,
    pub library_actions: LibraryActions,
}

/// A catalog page a source declares: the id that opens it, the translation
/// key of its label, and its icon class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPageEntry {
    pub id: String,
    pub label: &'static str,
    pub icon: &'static str,
}

/// A search filter a source offers: the id a client passes back and the
/// translation key of its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFilterEntry {
    pub id: &'static str,
    pub label: &'static str,
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
    pub artist_id: Option<String>,
    pub year: Option<String>,
    pub thumbnail: Option<String>,
    pub audio_playlist_id: Option<String>,
    pub tracks: Vec<reader::Track>,
    pub actions: crate::ytmusic::discover::ItemActions,
}

impl From<crate::ytmusic::discover::YtAlbum> for RemoteAlbum {
    fn from(a: crate::ytmusic::discover::YtAlbum) -> Self {
        Self {
            browse_id: a.browse_id,
            title: a.title,
            artist: a.artist,
            artist_id: a.artist_id,
            year: a.year,
            thumbnail: a.thumbnail,
            audio_playlist_id: a.audio_playlist_id,
            tracks: a.tracks,
            actions: a.actions,
        }
    }
}
