/// What to fetch artwork for. The ids are the same `key` a
/// [`crate::TrackInfo`] carries, so a client never composes a cover URL.
///
/// `Catalog` and `Station` name things the library does not hold -- a browse
/// shelf's tile, a radio station's icon. Their images are public URLs, but
/// they resolve through here anyway so that a frontend has exactly one way to
/// get a picture, and so a client that cannot fetch a URL itself (no webview)
/// still works.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArtworkTarget {
    Track(String),
    Album(String),
    Artist(String),
    Playlist(String),
    Catalog(String),
    Station(String),
}

impl ArtworkTarget {
    pub fn id(&self) -> &str {
        match self {
            Self::Track(id)
            | Self::Album(id)
            | Self::Artist(id)
            | Self::Playlist(id)
            | Self::Catalog(id)
            | Self::Station(id) => id,
        }
    }

    /// The entity kind, for a client that keys a cache or builds a URL by it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Track(_) => "track",
            Self::Album(_) => "album",
            Self::Artist(_) => "artist",
            Self::Playlist(_) => "playlist",
            Self::Catalog(_) => "catalog",
            Self::Station(_) => "station",
        }
    }
}

/// The artwork a row actually has. Absent on a row means the daemon has no
/// picture for it -- render the placeholder, do not ask.
///
/// `version` changes exactly when the image would: a photo search lands, a
/// scan indexes a cover, a server rotates its tag, a user uploads one. It is
/// derived from the resolved cover reference, never from a signed URL, so it
/// leaks nothing. Clients use it as the cache key: same version, same bytes,
/// cache forever.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtworkRef {
    pub target: ArtworkTarget,
    pub version: u64,
}

impl ArtworkRef {
    pub fn new(target: ArtworkTarget, version: u64) -> Self {
        Self { target, version }
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
