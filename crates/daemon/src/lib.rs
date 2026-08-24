//! The Kopuz daemon core: playback session, queue state, and (as they land)
//! library, config, and job services. Pure tokio, no Dioxus, no HTTP; the
//! `http` feature will add the axum shell. See `docs/daemon-split-plan.md`.

pub mod queue_model;
pub mod session;

pub use queue_model::{NextOutcome, QueueModel};
pub use session::{LocalApi, QueueMaterializer, SessionHandle};
