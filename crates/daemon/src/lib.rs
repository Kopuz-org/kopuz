//! The Kopuz daemon core: playback session, queue state, and (as they land)
//! library, config, and job services. Pure tokio, no Dioxus, no HTTP; the
//! `http` feature will add the axum shell.

#[cfg(feature = "http")]
pub mod http;
pub mod library;
mod playback;
pub mod queue_model;
pub mod session;

pub use library::LibraryService;
pub use queue_model::{NextOutcome, QueueModel};
pub use session::{LocalApi, PlaybackServices, QueueMaterializer, SessionHandle};
