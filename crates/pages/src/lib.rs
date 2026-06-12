pub mod activity;
pub mod album;
pub mod artist;
pub mod favorites;
pub mod home;
pub mod layout;
pub mod library;
pub mod local;
pub mod playlists;
pub mod radio;
pub mod search;
pub mod server;
pub mod settings;
#[cfg(not(target_os = "android"))]
pub mod theme_editor;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub mod ytdlp;
// Spotify settings UI. Desktop only: requires loopback OAuth listener,
// keyring, and a system browser to drive PKCE.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub mod spotify_page;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub mod spotify_search;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub mod spotify_settings;
