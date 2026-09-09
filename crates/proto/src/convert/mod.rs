//! Lossless conversion between the `api` types and the generated wire
//! types, split by the same domains the `api` crate uses. Each module owns
//! both directions for its types and the round-trip test that guards them:
//! every `api` value must survive api -> proto -> api unchanged.
//!
//! The functions are re-exported flat, so callers stay on `convert::name`.

mod catalog;
mod config;
mod enums;
mod error;
mod events;
mod library;
mod player;
mod playlists;
mod queue;
mod schema;
mod service;
mod sources;

#[cfg(test)]
mod fixtures;

pub use catalog::*;
pub use config::*;
pub use enums::*;
pub use error::*;
pub use events::*;
pub use library::*;
pub use player::*;
pub use playlists::*;
pub use queue::*;
pub use schema::*;
pub use service::*;
pub use sources::*;
