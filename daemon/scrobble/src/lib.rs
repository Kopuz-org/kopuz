//! Scrobbling support for Kopuz: sends now-playing and listened tracks to
//! Libre.fm and MusicBrainz ListenBrainz services.

mod audioscrobbler;
pub mod librefm;
pub mod listenbrainz;
pub mod musicbrainz;
pub mod queue;

pub use audioscrobbler::{NowPlaying, Scrobble, TrackMetadata};
pub use db::ScrobbleService;
