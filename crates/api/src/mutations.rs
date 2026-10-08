//! Changing the library: tags, deletions, artwork.
//!
//! These are the operations that touch a file on disk or push to a server, so
//! they belong to whoever holds the credentials and the library roots. A
//! client says what it wants changed and by which key; it never names a path.

/// What to do with an entity's picture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ArtworkChange {
    #[default]
    Keep,
    Remove,
    /// Replace it with these bytes. The format is detected, not declared.
    Set(Vec<u8>),
}

/// Edits to one track's tags. A field left `None` keeps its current value;
/// the `clear_*` flags are how a number is removed, since `None` already
/// means "unchanged".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackMetadataPatch {
    pub key: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<u32>,
    pub clear_track_number: bool,
    pub disc_number: Option<u32>,
    pub clear_disc_number: bool,
    /// Embedded cover art, changed in the same write as the tags.
    pub cover: ArtworkChange,
}

/// A picture for an entity that stores one outside a track's tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtworkUpload {
    pub target: crate::ArtworkTarget,
    pub content_type: String,
    pub bytes: Vec<u8>,
}
