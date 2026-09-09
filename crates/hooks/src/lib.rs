//! Dioxus hooks for Kopuz: player controller, library item management,
//! search data, and async player task orchestration.

pub mod api;
pub mod artist_images;
pub mod db_reactivity;
pub mod favorites;
pub mod jobs;
pub mod library_actions;
pub mod playlist_actions;
pub mod scrobble_scheduler;
mod session_projector;
pub mod source_switch;
pub mod sources;
pub mod toast;
pub mod use_db_queries;
pub mod use_player_controller;
pub mod use_player_task;
pub mod use_search_data;
pub mod wire;

pub use api::{consume_api, use_api};
pub use use_player_controller::*;
pub use use_player_task::*;
pub use use_search_data::*;

// The query types the UI composes, re-exported here (the query layer) so
// `pages`/`components` depend on `hooks`, not on the wire crate directly.
pub use ::api::{JobKind, Page, TrackFilter, TrackSort};
