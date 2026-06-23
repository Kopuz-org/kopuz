//! Spotify provider, backed by [librespot](https://github.com/librespot-org/librespot).
//!
//! Sign-in is OAuth (PKCE, browser) — [`auth`]. A single persistent
//! [`session`]-held connection speaks Spotify's internal access-point protocol,
//! so library/playlist hydration ([`metadata`]) never touches the rate-limited
//! public Web API. Playback ([`stream`]) downloads + decrypts a track to an
//! in-memory OGG buffer (Premium required). The `MediaSource` glue lives in
//! [`crate::source`]; the `__SP:<id>` stream sentinel is consumed by the player
//! controller.

pub mod auth;
pub mod metadata;
pub mod session;
pub mod stream;
