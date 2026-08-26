//! The Kopuz daemon core: playback session, queue state, and (as they land)
//! library, config, and job services. Pure tokio, no Dioxus, no wire; the
//! `grpc` feature adds the tonic shell.

pub mod artwork;
pub mod config_service;
#[cfg(feature = "grpc")]
pub mod ctl;
#[cfg(feature = "grpc")]
pub mod discovery;
pub mod downloads;
mod error;
pub mod favorites;
pub mod frontend;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod integrations;
pub mod jobs;
pub mod library;
pub mod os_media;
pub mod persistence;
mod playback;
pub mod queue_model;
pub mod scrobbler;
pub mod session;
mod wire;
mod ytdlp;

pub use artwork::ArtworkService;
pub use config_service::ConfigService;
pub use downloads::DownloadsService;
pub use favorites::FavoritesService;
pub use frontend::FrontendService;
pub use integrations::SourceRecorder;
pub use jobs::JobRunner;
pub use library::LibraryService;
pub use persistence::{DbQueueStore, QueueStore};
pub use queue_model::{NextOutcome, QueueModel};
pub use scrobbler::Scrobbler;
pub use session::{
    LocalApi, PlaybackServices, QueueMaterializer, QueueMirrorSnapshot, SessionHandle,
};
pub use wire::{
    music_service_from_api, music_service_to_api, track_from_info_parts, track_info_for_persistence,
};

pub fn active_source_label(config: &config::AppConfig) -> String {
    match &config.active_source {
        config::Source::Local => "Local".to_string(),
        config::Source::LocalLibrary(id) => config
            .local_sources
            .iter()
            .find(|source| &source.id == id)
            .map(|source| source.name.clone())
            .unwrap_or_else(|| "Local library".to_string()),
        config::Source::Server(id) => config
            .servers
            .iter()
            .find(|server| &server.id == id)
            .map(|server| format!("{} ({})", server.name, server.service.display_name()))
            .or_else(|| {
                config.server.as_ref().and_then(|server| {
                    (server.id.as_deref() == Some(id.as_str()))
                        .then(|| format!("{} ({})", server.name, server.service.display_name()))
                })
            })
            .unwrap_or_else(|| "Server".to_string()),
    }
}
