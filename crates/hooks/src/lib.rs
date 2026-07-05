//! Dioxus hooks for Kopuz: player controller, library item management,
//! search data, and async player task orchestration.

pub mod db_reactivity;
pub mod debug_db;
pub mod now_playing_api;
pub mod playback_ref;
mod player_controller_queue;
pub mod scrobble_scheduler;
pub mod source_switch;
pub mod use_db_queries;
pub mod use_player_controller;
pub mod use_player_task;
pub mod use_search_data;
pub mod use_sync_task;

pub use use_player_controller::*;
pub use use_player_task::*;
pub use use_search_data::*;

pub use debug_db::debug_db_section;

// The read-facing storage types the UI needs — re-exported here (the query
// layer) so `pages`/`components` depend on `hooks`, not `db`, and so cannot name
// the write-capable `db::Db` at all.
pub use db::{Page, ReadDb, TrackFilter, TrackSort};

/// Forces re-extraction of album covers from local embedded/folder art (no
/// online fetch). Provided by the app root; the settings page bumps it.
#[derive(Clone, Copy)]
pub struct CoverReextractTrigger(pub dioxus::prelude::Signal<i32>);
