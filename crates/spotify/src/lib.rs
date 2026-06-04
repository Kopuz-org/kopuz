//! Official Spotify integration.
//!
//! This crate intentionally uses only Spotify's official public interfaces:
//! the Spotify Web API for metadata and playback control, and the Spotify
//! Web Playback SDK (hosted inside a WebView) for in-app audio.
//!
//! It does NOT use librespot, does NOT decode or proxy raw Spotify audio,
//! and does NOT implement offline Spotify caching.

#![cfg_attr(not(feature = "spotify"), allow(dead_code))]

pub mod auth;
pub mod backends;
pub mod error;
pub mod pkce;
pub mod provider;
pub mod token_store;
pub mod types;
pub mod web_api;

#[cfg(feature = "spotify-web-playback")]
pub mod web_playback;

pub use error::SpotifyError;
pub use provider::{
    PlaybackDevice, PlaybackState, PlaylistSummary, RepeatMode, SearchResult, StreamingProvider,
    TrackSummary,
};
pub use types::SpotifyConfig;
