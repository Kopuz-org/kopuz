//! Audio playback engine for Kopuz: player state machine, decoder, equalizer,
//! system media controls, and audio output via cpal / symphonia.

pub mod decoder;
pub mod eq;
pub mod player;
pub mod systemint;
