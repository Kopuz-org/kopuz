//! Audio playback engine for Kopuz: player state machine, decoder, equalizer,
//! system media controls, and audio output via cpal / symphonia.

#[cfg(not(target_arch = "wasm32"))]
pub mod decoder;
#[cfg(not(target_arch = "wasm32"))]
pub mod eq;
pub mod player;
#[cfg(not(target_arch = "wasm32"))]
pub mod systemint;
