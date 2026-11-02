//! Which of the library's four shapes is on screen. Everything else about the page is shared.

use components::icons;

/// The four listings the one library page presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Facet {
    #[default]
    Playlists,
    Tracks,
    Albums,
    Artists,
}

impl Facet {
    pub const ALL: &'static [Facet] = &[
        Facet::Playlists,
        Facet::Tracks,
        Facet::Albums,
        Facet::Artists,
    ];

    /// The key this facet's remembered layout sits under in `layout.library`.
    pub fn name(self) -> &'static str {
        match self {
            Facet::Tracks => "tracks",
            Facet::Albums => "albums",
            Facet::Artists => "artists",
            Facet::Playlists => "playlists",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|facet| facet.name() == name)
    }

    pub fn label_key(self) -> &'static str {
        self.name()
    }

    pub fn icon(self) -> &'static str {
        match self {
            Facet::Tracks => icons::MUSIC,
            Facet::Albums => icons::DISC,
            Facet::Artists => icons::MIC,
            Facet::Playlists => icons::LIST,
        }
    }

    /// Tracks is a table; the other three are grids of tiles.
    pub fn is_grid(self) -> bool {
        self != Facet::Tracks
    }

    /// The placeholder a tile of this facet draws when it has no picture.
    pub fn placeholder(self) -> &'static str {
        match self {
            Facet::Artists => icons::MIC,
            Facet::Playlists => icons::LIST,
            _ => icons::DISC,
        }
    }

    /// Artists are drawn as circles, the way every player draws a person.
    pub fn round_tiles(self) -> bool {
        self == Facet::Artists
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_facet_round_trips_through_its_name() {
        for facet in Facet::ALL {
            assert_eq!(Facet::from_name(facet.name()), Some(*facet));
        }
        assert_eq!(Facet::from_name("nonsense"), None);
    }

    #[test]
    fn only_tracks_is_a_table() {
        assert!(!Facet::Tracks.is_grid());
        assert!(Facet::Albums.is_grid());
        assert!(Facet::Artists.is_grid());
        assert!(Facet::Playlists.is_grid());
    }
}
