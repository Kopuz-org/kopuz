//! Page-level Dioxus components for Kopuz: album, artist, discover, home,
//! search, settings, playlist views, and associated sub-components.

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
pub mod scroll_persist;
pub mod search;
pub mod server;
pub mod settings;
pub mod settings_actions;
#[cfg(not(target_os = "android"))]
pub mod theme_editor;
#[cfg(not(target_os = "android"))]
pub mod ytdlp;

/// A panel the app supplies through context, for surfaces that need something
/// `pages` deliberately cannot reach. The debug database tools are the only
/// one: they need a write-capable handle, which lives with the daemon core
/// the app hosts.
#[derive(Clone, Copy)]
pub struct DebugPanel(pub fn() -> dioxus::prelude::Element);
