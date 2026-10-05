//! Media file reader for Kopuz: parses audio metadata (tags, cover art),
//! manages favorites, and provides library scanning utilities.

pub mod cover_fetcher;
pub mod cover_indexer;
pub mod metadata;
pub mod models;
pub mod probe;
pub mod scanner;
pub mod utils;

pub use cover_indexer::{LocalCoverIndexReport, index_local_covers, missing_cover_ids};
pub use metadata::{
    ScannedTrack, VARIOUS_ARTISTS, album_credit, read, read_cover, read_metadata, write_tags,
};
pub use models::{
    Album, ArtistCredit, ArtistImageRef, Counterpart, CoverChange, CoverRef, FavoritesStore,
    Library, PlaylistEntry, PlaylistFolder, PlaylistStore, SharedSegment, Track, TrackEdits,
    TrackId,
};
pub use scanner::scan_directory;
