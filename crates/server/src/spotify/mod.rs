//! Spotify provider, backed by [librespot](https://github.com/librespot-org/librespot).
//!
//! Sign-in is OAuth (PKCE, browser) — [`auth`]. A single persistent
//! [`session`]-held connection speaks Spotify's internal access-point protocol,
//! so library/playlist hydration ([`metadata`]) never touches the rate-limited
//! public Web API. Playback resolves each track to an anonymous YouTube stream
//! ([`match_yt`]): Spotify is the catalog, YouTube is the audio, so a full track
//! plays without a Spotify Premium audio key. ([`stream`] still holds the native
//! librespot download/decrypt + 30s-preview path.) The `MediaSource` glue lives
//! in [`crate::source`].

pub mod auth;
pub mod match_yt;
pub mod metadata;
pub mod session;
pub mod stream;
