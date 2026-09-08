/// What to fetch artwork for. The ids are the same `key` a
/// [`crate::TrackInfo`] carries, so a client never composes a cover URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtworkTarget {
    Track(String),
    Album(String),
    Artist(String),
    Playlist(String),
}

impl ArtworkTarget {
    pub fn id(&self) -> &str {
        match self {
            Self::Track(id) | Self::Album(id) | Self::Artist(id) | Self::Playlist(id) => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkRequest {
    pub target: ArtworkTarget,
    /// Full size rather than a grid thumbnail.
    pub hq: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtworkData {
    pub content_type: String,
    pub bytes: Vec<u8>,
}
