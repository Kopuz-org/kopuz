//! Dioxus hooks for Kopuz: player controller, library item management,
//! search data, and async player task orchestration.

pub mod api;
pub mod artist_images;
pub mod db_reactivity;
pub mod debug_db;
pub mod favorites;
pub mod playlist_actions;
pub mod scrobble_scheduler;
mod session_projector;
pub mod source_switch;
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

pub use debug_db::debug_db_section;

// The query types the UI composes, re-exported here (the query layer) so
// `pages`/`components` depend on `hooks`, not on the wire crate directly.
pub use ::api::{Page, TrackFilter, TrackSort};
// Still storage-shaped: playlists and artist images have no API surface yet,
// so their hooks read the database and their callers name these.
pub use db::ReadDb;
