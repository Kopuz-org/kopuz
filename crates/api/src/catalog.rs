//! The source's own browse catalog: shelves of albums, playlists, artists and
//! songs that the library does not hold.
//!
//! A client renders these rows without knowing which service produced them or
//! how to talk to it. Songs arrive as ordinary [`TrackInfo`], so queueing one
//! is the same call as queueing a library track; the daemon remembers the row
//! so that key still resolves.

use crate::library::TrackInfo;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CatalogItemKind {
    Track,
    Album,
    Playlist,
    Artist,
    /// A curated mood or genre page.
    Mood,
    Podcast,
    /// One podcast episode; it carries a track, so it plays like one.
    Episode,
    /// A music video; it carries a track, so it plays like one.
    Video,
    /// Any other page the source can open, such as a chart or a shortcut
    /// to one of its [`crate::PageEntry`] pages. Opened with this kind.
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
    /// Present for the kinds that play as one song ([`CatalogItemKind::Track`],
    /// `Video`, `Episode`), so a shelf of them is playable without a second
    /// round trip.
    pub track: Option<TrackInfo>,
    /// The tile's own colour, as `#rrggbb`, where the source gives one (mood tiles).
    pub accent: Option<String>,
}

/// How a shelf is laid out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShelfLayout {
    /// A horizontal row of square tiles.
    #[default]
    Carousel,
    /// A wrapping grid of tiles.
    Grid,
    /// A vertical list of track rows.
    List,
    /// Track rows in columns of four that scroll sideways ("Quick picks").
    TrackGrid,
    /// One large tile with a few rows under it ("Top result").
    Hero,
}

/// A row of tiles. `list` marks the shelves a source renders as a track list
/// rather than a carousel; `layout` says the same with more choices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogShelf {
    pub title: String,
    pub strapline: Option<String>,
    pub items: Vec<CatalogItem>,
    /// Opens the shelf's own page, when it has one, with [`Self::more_kind`].
    pub more_ref: Option<String>,
    pub list: bool,
    pub layout: ShelfLayout,
    /// The kind [`CatalogDetailRequest`] takes to open `more_ref`.
    pub more_kind: CatalogItemKind,
    /// More items for this shelf: pass it back as the request's
    /// `continuation`, and the answer's one shelf continues this one.
    pub continuation: Option<String>,
    /// On the results of a search's "all" filter, the filter that shows
    /// every result of this shelf's kind.
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

    /// One of the source's pages: a [`crate::PageEntry`], a chip or a `Page` tile.
    pub fn page(id: impl Into<String>) -> Self {
        Self::new(CatalogItemKind::Page, id)
    }
}

/// How a detail page's header is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CatalogHeader {
    /// No header: the shelves are the page.
    #[default]
    None,
    /// The title alone.
    Title,
    /// Artwork beside the title, as an album, playlist or podcast has.
    Detail,
    /// A full-width banner, as an artist has.
    Artist,
}

/// A filter over a page, as the mood chips over a home feed. `id` opens the
/// filtered page with [`CatalogItemKind::Page`].
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
}
