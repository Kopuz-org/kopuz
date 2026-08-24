//! `LocalApi`: the in-process implementation of [`api::KopuzApi`].

use super::*;

/// In-process implementation of [`api::KopuzApi`] over a running session.
pub struct LocalApi {
    pub(super) session: SessionHandle,
    pub(super) library: Option<Arc<crate::library::LibraryService>>,
    pub(super) config: Option<Arc<crate::config_service::ConfigService>>,
    pub(super) jobs: Option<Arc<crate::jobs::JobRunner>>,
    pub(super) downloads: Option<Arc<crate::downloads::DownloadsService>>,
    pub(super) favorites: Option<Arc<crate::favorites::FavoritesService>>,
}

impl LocalApi {
    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            library: None,
            config: None,
            jobs: None,
            downloads: None,
            favorites: None,
        }
    }

    pub fn with_library(mut self, library: Arc<crate::library::LibraryService>) -> Self {
        self.library = Some(library);
        self
    }

    pub fn with_config(mut self, config: Arc<crate::config_service::ConfigService>) -> Self {
        self.config = Some(config);
        self
    }

    pub fn with_jobs(mut self, jobs: Arc<crate::jobs::JobRunner>) -> Self {
        self.jobs = Some(jobs);
        self
    }

    pub fn with_favorites(mut self, favorites: Arc<crate::favorites::FavoritesService>) -> Self {
        self.favorites = Some(favorites);
        self
    }

    pub fn with_downloads(mut self, downloads: Arc<crate::downloads::DownloadsService>) -> Self {
        self.downloads = Some(downloads);
        self
    }
}

#[async_trait::async_trait]
impl api::KopuzApi for LocalApi {
    async fn player_state(&self) -> Result<PlayerState, ApiError> {
        Ok(self.session.state())
    }

    async fn player_command(&self, command: PlayerCommand) -> Result<CommandAck, ApiError> {
        self.session.player_command(command).await
    }

    async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        self.session.queue_window(page).await
    }

    async fn set_queue(&self, request: SetQueueRequest) -> Result<CommandAck, ApiError> {
        self.session.set_queue(request).await
    }

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError> {
        self.session.queue_edit(edit).await
    }

    async fn tracks(
        &self,
        filter: api::TrackFilter,
        page: Page,
    ) -> Result<api::TrackPage, ApiError> {
        match &self.library {
            Some(library) => library.tracks(filter, page).await,
            None => Err(ApiError::unsupported(
                "this daemon runs without a library service",
            )),
        }
    }

    async fn config(&self) -> Result<api::ConfigView, ApiError> {
        match &self.config {
            Some(service) => service.view().await,
            None => Err(ApiError::unsupported(
                "this daemon runs without a config service",
            )),
        }
    }

    async fn patch_config(&self, patch: serde_json::Value) -> Result<api::ConfigView, ApiError> {
        let Some(service) = &self.config else {
            return Err(ApiError::unsupported(
                "this daemon runs without a config service",
            ));
        };
        let (view, updated, changed) = service.patch(patch).await?;
        self.session.set_config(updated, changed);
        Ok(view)
    }

    async fn favorites(&self) -> Result<api::FavoritesView, ApiError> {
        match &self.favorites {
            Some(service) => service.list().await,
            None => Err(ApiError::unsupported(
                "this daemon runs without a favorites service",
            )),
        }
    }

    async fn set_favorite(&self, key: String, favorite: bool) -> Result<(), ApiError> {
        match &self.favorites {
            Some(service) => service.set(&key, favorite).await,
            None => Err(ApiError::unsupported(
                "this daemon runs without a favorites service",
            )),
        }
    }

    async fn start_job(&self, kind: api::JobKind) -> Result<api::JobRef, ApiError> {
        let Some(runner) = &self.jobs else {
            return Err(ApiError::unsupported(
                "this daemon runs without a job runner",
            ));
        };
        match kind {
            api::JobKind::Scan => match &self.library {
                Some(library) => library.spawn_scan(runner),
                None => Err(ApiError::unsupported("no library service")),
            },
            api::JobKind::LibrarySync => match &self.library {
                Some(library) => library.spawn_remote_sync(runner),
                None => Err(ApiError::unsupported("no library service")),
            },
            api::JobKind::FavoritesSync => match &self.favorites {
                Some(favorites) => favorites.spawn_sync(runner),
                None => Err(ApiError::unsupported("no favorites service")),
            },
            api::JobKind::PlaylistSync | api::JobKind::Download | api::JobKind::Unknown => Err(
                ApiError::unsupported("this job kind has no direct starter yet"),
            ),
        }
    }

    async fn download(&self, keys: Vec<String>) -> Result<api::JobRef, ApiError> {
        let (Some(service), Some(runner)) = (&self.downloads, &self.jobs) else {
            return Err(ApiError::unsupported(
                "this daemon runs without a downloads service",
            ));
        };
        service.spawn_download(runner, keys)
    }

    async fn downloads(&self) -> Result<Vec<String>, ApiError> {
        match &self.downloads {
            Some(service) => Ok(service.list().await),
            None => Err(ApiError::unsupported(
                "this daemon runs without a downloads service",
            )),
        }
    }

    async fn remove_download(&self, key: String) -> Result<(), ApiError> {
        match &self.downloads {
            Some(service) => service.remove(&key).await,
            None => Err(ApiError::unsupported(
                "this daemon runs without a downloads service",
            )),
        }
    }

    async fn jobs(&self) -> Result<Vec<api::JobStatus>, ApiError> {
        match &self.jobs {
            Some(runner) => Ok(runner.list()),
            None => Err(ApiError::unsupported(
                "this daemon runs without a job runner",
            )),
        }
    }

    async fn cancel_job(&self, id: String) -> Result<(), ApiError> {
        match &self.jobs {
            Some(runner) => runner.cancel(&id),
            None => Err(ApiError::unsupported(
                "this daemon runs without a job runner",
            )),
        }
    }

    fn events(&self) -> api::EventStream {
        use futures_util::StreamExt;
        let rx = self.session.subscribe();
        futures_util::stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Ok((_, event)) => Some((event, rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => Some((ApiEvent::Resync, rx)),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .boxed()
    }
}
