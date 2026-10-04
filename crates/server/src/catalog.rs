//! Source-independent discovery shelves and artist details.

use reader::Track;

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoverHome {
    pub shelves: Vec<DiscoverShelf>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoverShelf {
    pub title: String,
    pub strapline: Option<String>,
    pub more_browse_id: Option<String>,
    pub items: Vec<DiscoverItem>,
    /// Render as a vertical song list (with row numbers / duration)
    /// instead of a horizontal tile carousel. Only set true for the
    /// artist-page "Top songs" shelf — discover-home shelves stay
    /// horizontal.
    pub is_song_list: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiscoverItem {
    Song(Box<Track>),
    Playlist {
        playlist_id: String,
        title: String,
        subtitle: String,
        thumbnail: Option<String>,
    },
    Album {
        browse_id: String,
        title: String,
        subtitle: String,
        thumbnail: Option<String>,
    },
    Artist {
        channel_id: String,
        name: String,
        thumbnail: Option<String>,
    },
    Mood {
        browse_id: String,
        title: String,
        thumbnail: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatalogArtist {
    pub channel_id: String,
    pub name: String,
    pub subscribers: Option<String>,
    pub description: Option<String>,
    pub banner_thumbnail: Option<String>,
    pub shuffle_playlist_id: Option<String>,
    pub sections: Vec<DiscoverShelf>,
}
