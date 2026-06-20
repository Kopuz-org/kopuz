pub mod activity;
pub mod album;
pub mod artist;
pub mod favorites;
pub mod favorites_body;
pub mod home;
pub mod home_body;
pub mod layout;
pub mod library;
pub mod playlists;
pub mod radio;
pub mod search;
pub mod server;
pub mod settings;
#[cfg(not(target_os = "android"))]
pub mod theme_editor;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub mod ytdlp;
