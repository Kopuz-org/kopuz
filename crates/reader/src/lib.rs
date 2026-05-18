#[cfg(not(target_arch = "wasm32"))]
pub mod metadata;
pub mod models;
#[cfg(not(target_arch = "wasm32"))]
pub mod scanner;
#[cfg(not(target_arch = "wasm32"))]
pub mod utils;

#[cfg(not(target_arch = "wasm32"))]
pub use metadata::read;
pub use models::{Album, FavoritesStore, Library, PlaylistFolder, PlaylistStore, Track};
#[cfg(not(target_arch = "wasm32"))]
pub use scanner::scan_directory;
