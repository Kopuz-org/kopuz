//! The playlists page, the view inside one folder, and one playlist's tracks.

mod cards;
mod detail;
mod dialogs;
mod folder;
mod page;

pub use detail::PlaylistDetail;
pub use dialogs::register;
pub use folder::PlaylistFolder;
pub use page::Playlists;
