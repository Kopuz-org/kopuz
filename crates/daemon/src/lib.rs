//! The Kopuz daemon core: playback session, queue state, and (as they land)
//! library, config, and job services. Pure tokio, no Dioxus, no wire; the
//! `grpc` feature adds the tonic shell.

#[cfg(feature = "kopuzd")]
pub mod boot;

pub mod artwork;
pub mod config_service;
pub mod downloads;
pub mod external;
pub mod favorites;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod integrations;
pub mod jobs;
pub mod library;
pub mod os_media;
pub mod ownership;
pub mod persistence;
mod playback;
pub mod queue_model;
pub mod scrobbler;
pub mod session;
mod wire;

pub use artwork::ArtworkService;
pub use config_service::ConfigService;
pub use downloads::DownloadsService;
pub use external::{ExternalPlayer, ExternalReport};
pub use favorites::FavoritesService;
pub use integrations::SourceRecorder;
pub use jobs::JobRunner;
pub use library::LibraryService;
pub use ownership::DatabaseLease;
pub use persistence::{DbQueueStore, QueueStore};
pub use queue_model::{NextOutcome, QueueModel};
pub use scrobbler::Scrobbler;
pub use session::{
    LocalApi, PlaybackServices, QueueMaterializer, QueueMirrorSnapshot, SessionHandle,
};
