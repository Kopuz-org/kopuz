//! Scrobbling support for Kopuz: sends now-playing and listened tracks to
//! Last.fm, Libre.fm, and MusicBrainz ListenBrainz services.

pub mod lastfm;
pub mod librefm;
pub mod musicbrainz;
pub mod queue;

// The scrobble destination enum is defined in `kopuz-db` (which owns the
// offline-queue table it indexes); re-exported here so the backends and queue
// share one type.
pub use db::Service;
