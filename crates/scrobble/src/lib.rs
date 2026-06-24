//! Scrobbling support for Kopuz: sends now-playing and listened tracks to
//! Last.fm, Libre.fm, and MusicBrainz ListenBrainz services.

pub mod lastfm;
pub mod librefm;
pub mod musicbrainz;
#[cfg(not(target_arch = "wasm32"))]
pub mod queue;
