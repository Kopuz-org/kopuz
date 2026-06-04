//! Web Playback SDK assets and Rust-side bridge contract.

pub mod bridge;

/// The local HTML page that hosts Spotify Web Playback SDK.
///
/// The page must be served from a local origin under the app's control.
/// Tokens must NOT be baked into the HTML. The page asks Rust for a fresh
/// token through the WebView bridge each time the SDK calls `getOAuthToken`.
pub const SPOTIFY_PLAYER_HTML: &str = include_str!("spotify_player.html");
