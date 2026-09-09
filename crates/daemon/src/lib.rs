//! The Kopuz daemon core: playback session, queue state, library, config, and
//! job services, with a [`LocalApi`] over them. Pure tokio -- no Dioxus, no
//! wire, no socket. Whoever hosts it decides how (or whether) to serve it:
//! `kopuz-kopuzd` puts it behind gRPC, the app calls it in-process.

pub mod artwork;
pub mod boot;
pub mod catalog;
pub mod config_service;
pub mod downloads;
pub mod external;
pub mod favorites;
pub mod integrations;
pub mod jobs;
pub mod library;
pub mod mutations;
pub mod os_media;
pub mod ownership;
pub mod persistence;
mod playback;
pub mod playlists;
pub mod queue_model;
pub mod radio;
pub mod scrobbler;
pub mod services;
pub mod session;
pub mod sources;
pub mod spotify;
mod wire;
pub mod ytdlp;

pub use artwork::ArtworkService;
pub use catalog::CatalogService;
pub use config_service::ConfigService;
pub use downloads::DownloadsService;
pub use external::{ExternalPlayer, ExternalReport};
pub use favorites::FavoritesService;
pub use integrations::IntegrationService;
pub use integrations::SourceRecorder;
pub use jobs::JobRunner;
pub use library::LibraryService;
pub use mutations::MutationService;
pub use ownership::DatabaseLease;
pub use persistence::{DbQueueStore, QueueStore};
pub use playlists::PlaylistService;
pub use queue_model::{NextOutcome, QueueModel};
pub use radio::RadioService;
pub use scrobbler::Scrobbler;
pub use session::{LocalApi, PlaybackServices, QueueMaterializer, SessionHandle};
pub use sources::SourceService;
pub use spotify::SpotifySink;
pub use ytdlp::YtdlpService;
