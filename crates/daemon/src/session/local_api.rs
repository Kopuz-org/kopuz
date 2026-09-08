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
    pub(super) artwork: Option<Arc<crate::artwork::ArtworkService>>,
    pub(super) playlists: Option<Arc<crate::playlists::PlaylistService>>,
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
            artwork: None,
            playlists: None,
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

    pub fn with_artwork(mut self, artwork: Arc<crate::artwork::ArtworkService>) -> Self {
        self.artwork = Some(artwork);
        self
    }

    pub fn with_playlists(mut self, playlists: Arc<crate::playlists::PlaylistService>) -> Self {
        self.playlists = Some(playlists);
        self
    }

    fn library(&self) -> Result<&crate::library::LibraryService, ApiError> {
        self.library
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a library service"))
    }

    fn playlists(&self) -> Result<&crate::playlists::PlaylistService, ApiError> {
        self.playlists
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a playlist service"))
    }
}

#[async_trait::async_trait]
impl api::PlaylistApi for LocalApi {
    async fn playlists(&self) -> Result<api::PlaylistCatalog, ApiError> {
        self.playlists()?.catalog().await
    }

    async fn create_playlist(&self, name: String, keys: Vec<String>) -> Result<String, ApiError> {
        self.playlists()?.create(&name, &keys).await
    }

    async fn rename_playlist(&self, id: String, name: String) -> Result<(), ApiError> {
        self.playlists()?.rename(&id, &name).await
    }

    async fn delete_playlist(&self, id: String) -> Result<(), ApiError> {
        self.playlists()?.delete(&id).await
    }

    async fn add_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        self.playlists()?.add_tracks(&id, &keys).await
    }

    async fn remove_playlist_track(&self, id: String, index: u32) -> Result<(), ApiError> {
        self.playlists()?.remove_track(&id, index).await
    }

    async fn reorder_playlist(
        &self,
        id: String,
        reorder: api::PlaylistReorder,
    ) -> Result<(), ApiError> {
        self.playlists()?.reorder(&id, reorder).await
    }

    async fn refresh_playlist(&self, id: String) -> Result<(), ApiError> {
        self.playlists()?.refresh(&id).await
    }

    async fn create_playlist_folder(&self, name: String) -> Result<String, ApiError> {
        self.playlists()?.create_folder(&name).await
    }

    async fn rename_playlist_folder(&self, id: String, name: String) -> Result<(), ApiError> {
        self.playlists()?.rename_folder(&id, &name).await
    }

    async fn delete_playlist_folder(&self, id: String) -> Result<(), ApiError> {
        self.playlists()?.delete_folder(&id).await
    }

    async fn move_playlist(
        &self,
        playlist_id: String,
        folder_id: Option<String>,
    ) -> Result<(), ApiError> {
        self.playlists()?
            .move_playlist(&playlist_id, folder_id.as_deref())
            .await
    }
}

#[async_trait::async_trait]
impl api::PlayerApi for LocalApi {
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
}

#[async_trait::async_trait]
impl api::LibraryApi for LocalApi {
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

    async fn tracks_by_keys(&self, keys: Vec<String>) -> Result<Vec<api::TrackInfo>, ApiError> {
        self.library()?.tracks_by_keys(&keys).await
    }

    async fn albums(&self, page: Page) -> Result<api::AlbumPage, ApiError> {
        self.library()?.albums(page).await
    }

    async fn album(&self, id: String) -> Result<Option<api::AlbumInfo>, ApiError> {
        self.library()?.album(&id).await
    }

    async fn album_tracks(&self, id: String, page: Page) -> Result<api::TrackPage, ApiError> {
        self.library()?.album_tracks(&id, page).await
    }

    async fn artists(&self, page: Page) -> Result<api::ArtistPage, ApiError> {
        self.library()?.artists(page).await
    }

    async fn artist_tracks(&self, artist: String, page: Page) -> Result<api::TrackPage, ApiError> {
        self.library()?.artist_tracks(&artist, page).await
    }

    async fn artist_sample_tracks(&self, page: Page) -> Result<api::TrackPage, ApiError> {
        self.library()?.artist_sample_tracks(page).await
    }

    async fn genres(&self) -> Result<Vec<String>, ApiError> {
        self.library()?.genres().await
    }

    async fn top_genre(&self) -> Result<Option<String>, ApiError> {
        self.library()?.top_genre().await
    }

    async fn genre_tracks(&self, genre: String, page: Page) -> Result<api::TrackPage, ApiError> {
        self.library()?.genre_tracks(&genre, page).await
    }

    async fn recent_tracks(&self, page: Page) -> Result<api::TrackPage, ApiError> {
        self.library()?.recent_tracks(page).await
    }

    async fn search(&self, query: String) -> Result<api::SearchResults, ApiError> {
        self.library()?.search(&query).await
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

    async fn folder_tracks(&self, prefix: String, page: Page) -> Result<api::TrackPage, ApiError> {
        match &self.library {
            Some(library) => library.folder_tracks(&prefix, page).await,
            None => Err(ApiError::unsupported("no library service")),
        }
    }

    async fn lyrics(&self, key: String) -> Result<api::LyricsView, ApiError> {
        match &self.library {
            Some(library) => library.lyrics(&key).await,
            None => Err(ApiError::unsupported("no library service")),
        }
    }

    async fn stats(&self) -> Result<api::StatsView, ApiError> {
        match &self.library {
            Some(library) => Ok(library.stats()),
            None => Err(ApiError::unsupported("no library service")),
        }
    }
}

#[async_trait::async_trait]
impl api::ArtworkApi for LocalApi {
    async fn artwork(&self, request: api::ArtworkRequest) -> Result<api::ArtworkData, ApiError> {
        let Some(artwork) = self.artwork.as_ref() else {
            return Err(ApiError::unsupported(
                "this daemon runs without an artwork service",
            ));
        };
        let entity = match &request.target {
            api::ArtworkTarget::Track(key) => crate::artwork::ArtworkEntity::Track(key),
            api::ArtworkTarget::Album(id) => crate::artwork::ArtworkEntity::Album(id),
            api::ArtworkTarget::Artist(name) => crate::artwork::ArtworkEntity::Artist(name),
            api::ArtworkTarget::Playlist(id) => crate::artwork::ArtworkEntity::Playlist(id),
        };
        let payload = artwork.fetch(entity, request.hq).await?;
        Ok(api::ArtworkData {
            content_type: payload.content_type.to_string(),
            bytes: payload.bytes,
        })
    }
}

#[async_trait::async_trait]
impl api::ConfigApi for LocalApi {
    async fn config(&self) -> Result<api::ConfigView, ApiError> {
        match &self.config {
            Some(service) => service.view().await,
            None => Err(ApiError::unsupported(
                "this daemon runs without a config service",
            )),
        }
    }

    async fn set_config(&self, config: config::AppConfig) -> Result<api::ConfigView, ApiError> {
        let Some(service) = &self.config else {
            return Err(ApiError::unsupported(
                "this daemon runs without a config service",
            ));
        };
        let (view, updated, changed) = service.set(config).await?;
        self.session.set_config(updated, changed);
        Ok(view)
    }
}

#[async_trait::async_trait]
impl api::JobApi for LocalApi {
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
            api::JobKind::PlaylistSync => match &self.playlists {
                Some(playlists) => playlists.spawn_sync(runner),
                None => Err(ApiError::unsupported("no playlist service")),
            },
            // Downloads carry their own request, so they start through
            // JobApi::download rather than by kind.
            api::JobKind::Download | api::JobKind::Unknown => {
                Err(ApiError::unsupported("this job kind has no direct starter"))
            }
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
}

impl api::EventApi for LocalApi {
    fn events(&self) -> api::EventStream {
        use futures_util::StreamExt;
        let rx = self.session.subscribe();
        // Greets with Resync like the wire implementation, so a consumer
        // sees the same first message whichever it is talking to.
        let greeting = futures_util::stream::once(async { ApiEvent::Resync });
        let live = futures_util::stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Ok(event) => Some((event, rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => Some((ApiEvent::Resync, rx)),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        });
        greeting.chain(live).boxed()
    }
}
