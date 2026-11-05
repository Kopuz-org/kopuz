//! The view inside one playlist folder, one playlist's tracks, and the dialogs their menus open.

mod cards;
mod detail;
mod dialogs;
mod folder;

pub use detail::PlaylistDetail;
pub use dialogs::register;
pub use folder::PlaylistFolder;
