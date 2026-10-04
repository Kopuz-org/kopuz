//! Scrobbling support for Kopuz: sends now-playing and listened tracks to
//! Last.fm, Libre.fm, and MusicBrainz ListenBrainz services.

mod audioscrobbler;
pub mod lastfm;
pub mod librefm;
pub mod musicbrainz;
pub mod queue;

pub use db::ScrobbleService;
