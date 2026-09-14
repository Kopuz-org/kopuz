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
    #[default]
    Unknown,
}

/// One tile. `id` is what [`CatalogDetailRequest`] takes to open it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogItem {
    pub kind: CatalogItemKind,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub artwork: Option<crate::ArtworkRef>,
    /// Present for [`CatalogItemKind::Track`], so a shelf of songs is
    /// playable without a second round trip.
    pub track: Option<TrackInfo>,
}

/// A row of tiles. `list` marks the shelves a source renders as a track list
/// rather than a carousel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CatalogShelf {
    pub title: String,
    pub strapline: Option<String>,
    pub items: Vec<CatalogItem>,
    /// Opens the shelf's own page, when it has one.
    pub more_ref: Option<String>,
    pub list: bool,
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
    pub id: String,
    pub continuation: Option<String>,
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
}
