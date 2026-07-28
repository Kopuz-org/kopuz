//! Spotify integration built on official public surfaces only.
//!
//! - [`auth`] — OAuth Authorization-Code + PKCE against `accounts.spotify.com`
//!   with a user-supplied Client ID and a loopback redirect. No password ever
//!   reaches kopuz.
//! - [`api`] — the public Web API (`api.spotify.com`) for library, playlists,
//!   liked songs, search, and issuing playback on a Connect device.
//! - [`host`] — playback via the official Web Playback SDK. The SDK needs a
//!   Widevine CDM, which kopuz's embedded webview lacks on macOS/Linux, so the
//!   host launches the user's own browser (which ships Widevine) and drives the
//!   SDK there over a localhost WebSocket. Premium is required for playback;
//!   library/metadata work on any account.
//!
//! The refresh token is packed into the existing `access_token` config column
//! as `"<access>\n<refresh>"` (see [`auth::pack_token`]) so no schema migration
//! is needed. The Client ID lives in the server's `url` field.

pub mod api;
pub mod auth;
pub mod host;
