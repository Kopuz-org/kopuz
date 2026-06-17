#[cfg(not(target_arch = "wasm32"))]
pub mod cover_fetcher;
#[cfg(not(target_arch = "wasm32"))]
pub mod metadata;
pub mod models;
#[cfg(not(target_arch = "wasm32"))]
pub mod scanner;
#[cfg(not(target_arch = "wasm32"))]
pub mod utils;

#[cfg(not(target_arch = "wasm32"))]
pub use metadata::{read, read_cover, write_tags};
pub use models::{
    Album, CoverChange, FavoritesStore, Library, PlaylistFolder, PlaylistStore, Track, TrackEdits,
    TrackId,
};
#[cfg(not(target_arch = "wasm32"))]
pub use scanner::scan_directory;
