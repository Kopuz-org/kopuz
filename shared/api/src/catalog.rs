//! The source's own browse catalog: shelves of albums, playlists, artists and
//! songs that the library does not hold.
//!
//! A client renders these rows without knowing which service produced them or
//! how to talk to it. Songs arrive as ordinary [`TrackInfo`], so queueing one
//! is the same call as queueing a library track; the daemon remembers the row
//! so that key still resolves.

use crate::library::{AlbumInfo, TrackInfo};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CatalogItemKind {
    Track,
    Album,
    Playlist,
    Artist,
    /// A curated mood or genre page.
    Mood,
    Podcast,
    Episode,
    Video,
    /// Any other page the source can open, such as a chart.
    Page,
    #[default]
    Unknown,
}

/// One tile. `id` is what [`CatalogDetailRequest`] takes to open it; an artist tile's is its key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogItem {
    pub kind: CatalogItemKind,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub artwork: Option<crate::ArtworkRef>,
    /// Present for the kinds that play as one song, so a shelf of them is
    /// playable without a second round trip.
    pub track: Option<TrackInfo>,
    /// `#rrggbb`, where the source gives the tile a colour.
    pub accent: Option<String>,
    pub explicit: bool,
    pub web_url: Option<String>,
    pub actions: CatalogActions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Rating {
    #[default]
    None,
    Like,
    Dislike,
}

/// Refs are opaque and go back as they came. `None` means the source has no
/// such action, or for a state, that the page did not say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogActions {
    pub rate_ref: Option<String>,
    pub rating: Option<Rating>,
    pub save_ref: Option<String>,
    pub saved: Option<bool>,
    pub follow_ref: Option<String>,
    pub followed: Option<bool>,
    /// Only a row of the listening history has one.
    pub history_token: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShelfLayout {
    #[default]
    Carousel,
    Grid,
    List,
    /// Track rows in columns of four that scroll sideways ("Quick picks").
    TrackGrid,
    /// One large tile with a few rows under it ("Top result").
    Hero,
}

/// A row of tiles, laid out as `layout` says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogShelf {
    pub title: String,
    pub strapline: Option<String>,
    pub items: Vec<CatalogItem>,
    /// Opens the shelf's own page, when it has one.
    pub more_ref: Option<String>,
    pub layout: ShelfLayout,
    /// The kind that opens `more_ref`.
    pub more_kind: CatalogItemKind,
    /// More items for this shelf, not more shelves for the page.
    pub continuation: Option<String>,
    /// On an "all" search, the filter that shows every result of this kind.
    pub search_filter: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogPage {
    pub shelves: Vec<CatalogShelf>,
    /// Pass back to [`crate::LibraryApi::catalog`] for the next page.
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogDetailRequest {
    pub kind: CatalogItemKind,
    /// What the daemon handed out for the entity: a tile's `id`, or an artist's key.
    pub id: String,
    pub continuation: Option<String>,
}

impl CatalogDetailRequest {
    pub fn new(kind: CatalogItemKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            continuation: None,
        }
    }

    pub fn artist(artist: &str) -> Self {
        Self::new(CatalogItemKind::Artist, artist)
    }

    pub fn page(id: impl Into<String>) -> Self {
        Self::new(CatalogItemKind::Page, id)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CatalogHeader {
    #[default]
    None,
    Title,
    /// Artwork beside the title.
    Detail,
    /// A full-width banner.
    Artist,
}

/// A filter over a page; `id` opens the filtered page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogChip {
    pub id: String,
    pub label: String,
    pub selected: bool,
}

/// One catalog entity opened: its tracks, or its own shelves, or both.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogDetail {
    pub kind: CatalogItemKind,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub artwork: Option<crate::ArtworkRef>,
    /// The id to play the whole thing, where that differs from `id`.
    pub playback_id: Option<String>,
    pub year: Option<String>,
    pub tracks: Vec<TrackInfo>,
    pub shelves: Vec<CatalogShelf>,
    pub continuation: Option<String>,
    /// For an album, the artist its header opens; absent when it bills nobody.
    pub artist_key: Option<String>,
    pub header: CatalogHeader,
    pub chips: Vec<CatalogChip>,
    /// For an album, the kind of release its source files it as.
    pub release_kind: Option<ReleaseKind>,
    /// For an artist; `subtitle` carries its subscribers.
    pub monthly_listeners: Option<String>,
    /// For an artist, `QueueContext::PlaylistRadio` ids of its Shuffle and Mix.
    pub shuffle_radio: Option<String>,
    pub mix_radio: Option<String>,
    pub owner: Option<String>,
    /// As the source wrote it ("6.9M views").
    pub plays: Option<String>,
    pub web_url: Option<String>,
    pub actions: CatalogActions,
    /// Set for a playlist the account owns.
    pub privacy: Option<crate::PlaylistPrivacy>,
}

/// What kind of release a source files an album as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseKind {
    Album,
    Single,
    Ep,
    Audiobook,
}

/// The active source's "For you" mix: songs from radios seeded by a few of its liked tracks.
#[derive(Debug, Clone, PartialEq)]
pub enum ForYou {
    /// Shuffled, without repeats; the same until refreshed or the daemon restarts.
    Mix(Vec<TrackInfo>),
    /// There is nothing to mix from, so a frontend hides the row.
    Unavailable(ForYouUnavailable),
}

/// The active source's Discover rows; a row the source cannot fill is empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscoverRows {
    /// The mixes the source made for this account, as playlist tiles.
    pub mixes: Vec<CatalogItem>,
    /// Albums with no play, offered only where [`crate::SourceCapabilities::complete_play_history`] holds.
    pub never_played: Vec<AlbumInfo>,
    /// Liked or much-played tracks not played for a while, likewise.
    pub forgotten_favorites: Vec<TrackInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForYouUnavailable {
    /// The source cannot start a radio from a track.
    NoTrackRadio,
    /// Nothing is liked on the source yet.
    NoLikes,
}
