//! The Kopuz daemon core: playback session, queue state, and (as they land)
//! library, config, and job services. Pure tokio, no Dioxus, no HTTP; the
//! `http` feature will add the axum shell.

pub mod artwork;
pub mod config_service;
pub mod downloads;
pub mod favorites;
#[cfg(feature = "http")]
pub mod http;
pub mod integrations;
pub mod jobs;
pub mod library;
pub mod os_media;
pub mod persistence;
mod playback;
pub mod queue_model;
pub mod session;

pub use artwork::ArtworkService;
pub use config_service::ConfigService;
pub use downloads::DownloadsService;
pub use favorites::FavoritesService;
pub use integrations::SourceRecorder;
pub use jobs::JobRunner;
pub use library::LibraryService;
pub use persistence::{DbQueueStore, QueueStore};
pub use queue_model::{NextOutcome, QueueModel};
pub use session::{LocalApi, PlaybackServices, QueueMaterializer, SessionHandle};
