//! Media file reader for Kopuz: parses audio metadata (tags, cover art),
//! manages favorites, and provides library scanning utilities.

pub mod cover_fetcher;
pub mod cover_indexer;
pub mod metadata;
pub mod models;
pub mod scanner;
pub mod sort;
pub mod utils;

pub use cover_indexer::{LocalCoverIndexReport, index_local_covers};
pub use metadata::{read, read_cover, write_tags};
pub use models::{
    Album, ArtistImageRef, CoverChange, FavoritesStore, Library, PlaylistFolder, PlaylistStore,
    Track, TrackEdits, TrackId,
};
pub use scanner::scan_directory;
