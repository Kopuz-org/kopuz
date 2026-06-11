pub mod lastfm;
pub mod librefm;
pub mod musicbrainz;
#[cfg(not(target_arch = "wasm32"))]
pub mod queue;
