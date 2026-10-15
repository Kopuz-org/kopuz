//! Spotify through librespot, the desktop client's own protocol.
//!
//! - [`auth`] — Authorization-Code + PKCE against `accounts.spotify.com` with
//!   the desktop client id, then a librespot login that turns the OAuth token
//!   into credentials that do not expire. No app to register, no password.
//! - [`session`] — the one live access-point connection per account, shared
//!   by every caller in the process.
//! - [`catalog`] — library, liked songs, playlists, albums and the home page
//!   over the client's own endpoints (`spclient`). The public Web API is not
//!   used: Spotify throttles it for the client id every librespot player
//!   shares.
//! - [`search`] — the web player's GraphQL search.
//! - [`stream`] — audio. The track's file id from metadata, its key from the
//!   access point, and the decrypted Ogg Vorbis as a seekable reader the
//!   engine decodes like any other remote stream. Premium is required for
//!   playback; the reads work on any account.
//!
//! The stored credentials live in the server's `access_token` column as JSON
//! (see [`session::pack_credentials`]).

pub mod auth;
pub mod catalog;
pub mod search;
pub mod session;
pub mod stream;
