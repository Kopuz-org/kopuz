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
    /// An id the source issued; empty when the caller only knows a name.
    pub id: String,
    pub continuation: Option<String>,
    /// What to look the entity up by when no id is held, for a source that can
    /// resolve one. A caller holding both sends the id: it is the exact one.
    pub name: Option<String>,
}

impl CatalogDetailRequest {
    /// Open `id`, which the source issued.
    pub fn by_id(kind: CatalogItemKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            ..Default::default()
        }
    }

    /// Open whatever the source resolves `name` to, for a caller holding no id.
    pub fn by_name(kind: CatalogItemKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: Some(name.into()),
            ..Default::default()
        }
    }

    /// Which of the two the caller gave, or `None` for a request naming nothing.
    pub fn reference(&self) -> Option<Reference<'_>> {
        match self.id.trim() {
            "" => self
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(Reference::Name),
            id => Some(Reference::Id(id)),
        }
    }
}

/// What a caller is opening an entity by, said by the type rather than guessed
/// at from the string's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reference<'a> {
    Id(&'a str),
    Name(&'a str),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_read_as_an_id() {
        let request = CatalogDetailRequest::by_id(CatalogItemKind::Artist, "UCabc123");
        assert_eq!(request.reference(), Some(Reference::Id("UCabc123")));
    }

    /// The guess this replaced read a `UC` prefix off the id, so a band called
    /// UCHU CONBINI was fetched as a channel that does not exist.
    #[test]
    fn a_name_shaped_like_an_id_is_still_a_name() {
        let request = CatalogDetailRequest::by_name(CatalogItemKind::Artist, "UCHU CONBINI");
        assert_eq!(request.reference(), Some(Reference::Name("UCHU CONBINI")));
    }

    /// And the same guess sent every id of a source whose ids look nothing like
    /// YouTube's down the name path.
    #[test]
    fn an_id_that_looks_nothing_like_youtube_is_still_an_id() {
        let request = CatalogDetailRequest::by_id(CatalogItemKind::Artist, "ar-0f31b2");
        assert_eq!(request.reference(), Some(Reference::Id("ar-0f31b2")));
    }

    #[test]
    fn an_id_wins_when_a_caller_holds_both() {
        let mut request = CatalogDetailRequest::by_id(CatalogItemKind::Artist, "UCabc123");
        request.name = Some("Whoever".into());
        assert_eq!(request.reference(), Some(Reference::Id("UCabc123")));
    }

    #[test]
    fn a_request_naming_nothing_asks_for_nothing() {
        assert_eq!(CatalogDetailRequest::default().reference(), None);
        let blank = CatalogDetailRequest::by_name(CatalogItemKind::Artist, "   ");
        assert_eq!(blank.reference(), None);
    }
}
