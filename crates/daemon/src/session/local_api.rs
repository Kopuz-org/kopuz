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
    pub(super) catalog: Option<Arc<crate::catalog::CatalogService>>,
    pub(super) radio: Option<Arc<crate::radio::RadioService>>,
    pub(super) mutations: Option<Arc<crate::mutations::MutationService>>,
    pub(super) sources: Option<Arc<crate::sources::SourceService>>,
    pub(super) integrations: Option<Arc<crate::integrations::IntegrationService>>,
    pub(super) ytdlp: Option<Arc<crate::ytdlp::YtdlpService>>,
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
            catalog: None,
            radio: None,
            mutations: None,
            sources: None,
            integrations: None,
            ytdlp: None,
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

    pub fn with_catalog(mut self, catalog: Arc<crate::catalog::CatalogService>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    pub fn with_radio(mut self, radio: Arc<crate::radio::RadioService>) -> Self {
        self.radio = Some(radio);
        self
    }

    fn library(&self) -> Result<&crate::library::LibraryService, ApiError> {
        self.library
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a library service"))
    }

    pub fn with_mutations(mut self, mutations: Arc<crate::mutations::MutationService>) -> Self {
        self.mutations = Some(mutations);
        self
    }

    fn mutations(&self) -> Result<&crate::mutations::MutationService, ApiError> {
        self.mutations
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs read-only"))
    }

    fn catalog(&self) -> Result<&crate::catalog::CatalogService, ApiError> {
        self.catalog
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a catalog service"))
    }

    fn radio(&self) -> Result<&crate::radio::RadioService, ApiError> {
        self.radio
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a radio service"))
    }

    fn playlists(&self) -> Result<&crate::playlists::PlaylistService, ApiError> {
        self.playlists
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a playlist service"))
    }

    pub fn with_sources(mut self, sources: Arc<crate::sources::SourceService>) -> Self {
        self.sources = Some(sources);
        self
    }

    pub fn with_integrations(
        mut self,
        integrations: Arc<crate::integrations::IntegrationService>,
    ) -> Self {
        self.integrations = Some(integrations);
        self
    }

    fn sources(&self) -> Result<&crate::sources::SourceService, ApiError> {
        self.sources
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon manages no sources"))
    }

    fn integrations(&self) -> Result<&crate::integrations::IntegrationService, ApiError> {
        self.integrations
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon manages no integrations"))
    }

    pub fn with_ytdlp(mut self, ytdlp: Arc<crate::ytdlp::YtdlpService>) -> Self {
        self.ytdlp = Some(ytdlp);
        self
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

    async fn queue_snapshot(&self) -> Result<api::QueueSnapshot, ApiError> {
        let mirror = self.session.queue_mirror().await;
        let config = self.session.config_watch().borrow().clone();
        Ok(api::QueueSnapshot {
            rev: self.session.state().queue.rev,
            items: mirror
                .tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
            shuffle_order: mirror
                .shuffle_order
                .iter()
                .map(|index| *index as u32)
                .collect(),
            position: (!mirror.tracks.is_empty()).then_some(mirror.position as u32),
            shuffle: mirror.shuffle,
        })
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

    async fn catalog(&self, continuation: Option<String>) -> Result<api::CatalogPage, ApiError> {
        self.catalog()?.catalog(continuation.as_deref()).await
    }

    async fn catalog_detail(
        &self,
        request: api::CatalogDetailRequest,
    ) -> Result<api::CatalogDetail, ApiError> {
        self.catalog()?.detail(request).await
    }

    async fn radio_stations(&self) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        Ok(self.radio()?.stations().await)
    }

    async fn search_radio(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        self.radio()?.search(&query, limit).await
    }

    async fn pin_radio_station(&self, id: String, pinned: bool) -> Result<(), ApiError> {
        self.radio()?.pin(&id, pinned).await
    }

    async fn validate_radio_registry(&self, url: String) -> Result<u32, ApiError> {
        self.radio()?.validate_registry(&url).await
    }

    async fn update_track_metadata(
        &self,
        patch: api::TrackMetadataPatch,
    ) -> Result<api::TrackInfo, ApiError> {
        self.mutations()?.update_track_metadata(patch).await
    }

    async fn delete_tracks(&self, keys: Vec<String>, from_disk: bool) -> Result<(), ApiError> {
        self.mutations()?.delete_tracks(&keys, from_disk).await
    }

    async fn delete_album(&self, id: String, from_disk: bool) -> Result<(), ApiError> {
        self.mutations()?.delete_album(&id, from_disk).await
    }

    async fn upload_artwork(&self, upload: api::ArtworkUpload) -> Result<(), ApiError> {
        self.mutations()?.upload_artwork(upload).await
    }

    async fn remove_artwork(&self, target: api::ArtworkTarget) -> Result<(), ApiError> {
        self.mutations()?.remove_artwork(target).await
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

    async fn refresh_artist_artwork(&self, names: Vec<String>) -> Result<(), ApiError> {
        self.library()?.refresh_artist_artwork(names).await
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
        let payload = artwork.fetch(&request.target, request.hq).await?;
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
            // These carry their own request, so they start through their own
            // method rather than by kind.
            api::JobKind::Download | api::JobKind::Ytdlp | api::JobKind::Unknown => {
                Err(ApiError::unsupported("this job kind has no direct starter"))
            }
        }
    }

    async fn start_ytdlp(&self, request: api::YtdlpRequest) -> Result<api::JobRef, ApiError> {
        let (Some(service), Some(runner)) = (&self.ytdlp, &self.jobs) else {
            return Err(ApiError::unsupported("this daemon runs without yt-dlp"));
        };
        service.start(runner, request)
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

    async fn download_statuses(&self) -> Result<Vec<api::DownloadItemStatus>, ApiError> {
        match &self.downloads {
            Some(service) => Ok(service.statuses()),
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

#[async_trait::async_trait]
impl api::SourceApi for LocalApi {
    async fn sources(&self) -> Result<Vec<api::SourceInfo>, ApiError> {
        self.sources()?.sources().await
    }

    async fn switch_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.switch_source(&id).await
    }

    async fn upsert_local_source(
        &self,
        draft: api::LocalSourceDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.upsert_local_source(draft).await
    }

    async fn delete_local_source(&self, id: String) -> Result<(), ApiError> {
        self.sources()?.delete_local_source(&id).await
    }

    async fn set_source_directories(
        &self,
        id: String,
        directories: Vec<String>,
    ) -> Result<api::SourceInfo, ApiError> {
        self.sources()?
            .set_source_directories(&id, directories)
            .await
    }

    async fn upsert_server(&self, draft: api::ServerDraft) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.upsert_server(draft).await
    }

    async fn delete_server(&self, id: String) -> Result<(), ApiError> {
        self.sources()?.delete_server(&id).await
    }

    async fn provision_credentials(
        &self,
        provision: api::CredentialProvision,
    ) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.provision_credentials(provision).await
    }

    async fn login_source(
        &self,
        request: api::SourceLoginRequest,
    ) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.login_source(request).await
    }

    async fn clear_credentials(&self, id: String) -> Result<(), ApiError> {
        self.sources()?.clear_credentials(&id).await
    }

    async fn authenticate_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.authenticate_source(&id).await
    }

    async fn browse_source(
        &self,
        id: String,
        path: String,
    ) -> Result<Vec<api::SourceFolderEntry>, ApiError> {
        self.sources()?.browse_source(&id, &path).await
    }

    async fn validate_source(&self, id: String) -> Result<api::SourceState, ApiError> {
        self.sources()?.validate_source(&id).await
    }

    async fn integrations(&self) -> Result<Vec<api::IntegrationStatus>, ApiError> {
        Ok(self.integrations()?.statuses().await)
    }

    async fn provision_integration(
        &self,
        provision: api::IntegrationProvision,
    ) -> Result<api::IntegrationStatus, ApiError> {
        self.integrations()?.provision(provision).await
    }

    async fn clear_integration(&self, kind: api::IntegrationKind) -> Result<(), ApiError> {
        self.integrations()?.clear(kind).await
    }

    async fn authenticate_integration(
        &self,
        kind: api::IntegrationKind,
    ) -> Result<api::IntegrationStatus, ApiError> {
        self.integrations()?.authenticate(kind).await
    }
}
