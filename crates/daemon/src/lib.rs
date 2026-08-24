//! The Kopuz daemon core: playback session, queue state, and (as they land)
//! library, config, and job services. Pure tokio, no Dioxus, no HTTP; the
//! `http` feature will add the axum shell.

pub mod config_service;
#[cfg(feature = "http")]
pub mod http;
pub mod library;
pub mod persistence;
mod playback;
pub mod queue_model;
pub mod session;

pub use config_service::ConfigService;
pub use library::LibraryService;
pub use persistence::{DbQueueStore, QueueStore};
pub use queue_model::{NextOutcome, QueueModel};
pub use session::{LocalApi, PlaybackServices, QueueMaterializer, SessionHandle};
