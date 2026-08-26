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
    pub(super) frontend: Option<Arc<crate::frontend::FrontendService>>,
    pub(super) artwork: Option<Arc<crate::artwork::ArtworkService>>,
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
            frontend: None,
            artwork: None,
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

    pub fn with_frontend(mut self, frontend: Arc<crate::frontend::FrontendService>) -> Self {
        self.frontend = Some(frontend);
        self
    }

    pub fn with_artwork(mut self, artwork: Arc<crate::artwork::ArtworkService>) -> Self {
        self.artwork = Some(artwork);
        self
    }

    fn frontend(&self) -> Result<&crate::frontend::FrontendService, ApiError> {
        self.frontend
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without frontend services"))
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

    async fn queue_snapshot(&self) -> Result<api::QueuePersistenceSnapshot, ApiError> {
        self.frontend()?.queue_snapshot().await
    }

    async fn save_queue_snapshot(
        &self,
        snapshot: api::QueuePersistenceSnapshot,
    ) -> Result<(), ApiError> {
        self.frontend()?.save_queue_snapshot(snapshot).await
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
        let previous_source = service.snapshot().await.active_source;
        let (view, updated, changed) = service.patch(patch).await?;
        let source_changed = previous_source != updated.active_source;
        if source_changed && let Some(frontend) = &self.frontend {
            frontend.finish_source_change(updated, changed).await?;
        } else {
            if source_changed {
                self.session
                    .set_active_source(Some(service.playback_source(&updated)));
            }
            self.session.set_config(updated, changed);
            if source_changed {
                self.session.reset_playback().await?;
            }
        }
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
            api::JobKind::PlaylistSync => match &self.library {
                Some(library) => library.spawn_playlist_sync(runner),
                None => Err(ApiError::unsupported("no library service")),
            },
            api::JobKind::Download | api::JobKind::Ytdlp | api::JobKind::Unknown => Err(
                ApiError::unsupported("this job kind has no direct starter yet"),
            ),
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

    fn lyrics_stream(&self, key: String) -> api::LyricsStream {
        use futures_util::StreamExt as _;
        match &self.library {
            Some(library) => library.lyrics_stream(key),
            None => futures_util::stream::once(async {
                Err(ApiError::unsupported("no library service"))
            })
            .boxed(),
        }
    }

    async fn stats(&self) -> Result<api::StatsView, ApiError> {
        match &self.library {
            Some(library) => Ok(library.stats()),
            None => Err(ApiError::unsupported("no library service")),
        }
    }

    async fn albums(
        &self,
        filter: api::AlbumFilter,
        page: Page,
    ) -> Result<api::AlbumPage, ApiError> {
        self.frontend()?.albums(filter, page).await
    }

    async fn album(&self, id: String) -> Result<api::AlbumInfo, ApiError> {
        self.frontend()?.album(&id).await
    }

    async fn artists(&self, page: Page) -> Result<api::ArtistPage, ApiError> {
        self.frontend()?.artists(page).await
    }

    async fn refresh_artist_artwork(&self, names: Vec<String>) -> Result<Vec<String>, ApiError> {
        self.frontend()?.refresh_artist_artwork(names).await
    }

    async fn genres(&self) -> Result<Vec<String>, ApiError> {
        self.frontend()?.genres().await
    }

    async fn recent_tracks(&self, page: Page) -> Result<api::TrackPage, ApiError> {
        self.frontend()?.recent_tracks(page).await
    }

    async fn album_tracks(&self, id: String, page: Page) -> Result<api::TrackPage, ApiError> {
        self.frontend()?.album_tracks(&id, page).await
    }

    async fn artist_tracks(&self, name: String, page: Page) -> Result<api::TrackPage, ApiError> {
        self.frontend()?.artist_tracks(&name, page).await
    }

    async fn genre_tracks(&self, name: String, page: Page) -> Result<api::TrackPage, ApiError> {
        self.frontend()?.genre_tracks(&name, page).await
    }

    async fn artist_sample_tracks(&self, page: Page) -> Result<api::TrackPage, ApiError> {
        self.frontend()?.artist_sample_tracks(page).await
    }

    async fn tracks_by_keys(&self, keys: Vec<String>) -> Result<Vec<api::TrackInfo>, ApiError> {
        self.frontend()?.tracks_by_keys(&keys).await
    }

    async fn track_web_url(&self, key: String) -> Result<Option<String>, ApiError> {
        self.frontend()?.track_web_url(&key).await
    }

    async fn album_web_url(&self, id: String) -> Result<Option<String>, ApiError> {
        self.frontend()?.album_web_url(&id).await
    }

    async fn top_genre(&self) -> Result<Option<String>, ApiError> {
        self.frontend()?.top_genre().await
    }

    async fn search(&self, query: String) -> Result<api::SearchResults, ApiError> {
        self.frontend()?.search(&query).await
    }

    async fn playlists(&self) -> Result<api::PlaylistCatalog, ApiError> {
        self.frontend()?.playlists().await
    }

    async fn playlist_tracks(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<api::TrackPage, ApiError> {
        self.frontend()?.playlist_tracks(request).await
    }

    async fn refresh_playlist(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<api::TrackPage, ApiError> {
        self.frontend()?.refresh_playlist(request).await
    }

    async fn create_playlist(&self, name: String, keys: Vec<String>) -> Result<String, ApiError> {
        self.frontend()?.create_playlist(&name, &keys).await
    }

    async fn rename_playlist(&self, id: String, name: String) -> Result<(), ApiError> {
        self.frontend()?.rename_playlist(&id, &name).await
    }

    async fn delete_playlist(&self, id: String) -> Result<(), ApiError> {
        self.frontend()?.delete_playlist(&id).await
    }

    async fn add_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        self.frontend()?.add_playlist_tracks(&id, &keys).await
    }

    async fn remove_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        self.frontend()?.remove_playlist_tracks(&id, &keys).await
    }

    async fn reorder_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        self.frontend()?.reorder_playlist_tracks(&id, &keys).await
    }

    async fn create_playlist_folder(&self, name: String) -> Result<String, ApiError> {
        self.frontend()?.create_folder(&name).await
    }

    async fn rename_playlist_folder(&self, id: String, name: String) -> Result<(), ApiError> {
        self.frontend()?.rename_folder(&id, &name).await
    }

    async fn delete_playlist_folder(&self, id: String) -> Result<(), ApiError> {
        self.frontend()?.delete_folder(&id).await
    }

    async fn move_playlist(&self, id: String, folder_id: Option<String>) -> Result<(), ApiError> {
        self.frontend()?
            .move_playlist(&id, folder_id.as_deref())
            .await
    }

    async fn sources(&self) -> Result<Vec<api::SourceInfo>, ApiError> {
        self.frontend()?.sources().await
    }

    async fn switch_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        self.frontend()?.switch_source(&id).await
    }

    async fn upsert_local_source(
        &self,
        source: api::LocalSourceDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        self.frontend()?.upsert_local_source(source).await
    }

    async fn delete_local_source(&self, id: String) -> Result<(), ApiError> {
        self.frontend()?.delete_local_source(&id).await
    }

    async fn set_source_directories(
        &self,
        id: String,
        directories: Vec<String>,
    ) -> Result<api::SourceInfo, ApiError> {
        self.frontend()?
            .set_source_directories(&id, directories)
            .await
    }

    async fn upsert_server(&self, server: api::ServerDraft) -> Result<api::SourceInfo, ApiError> {
        self.frontend()?.upsert_server(server).await
    }

    async fn delete_server(&self, id: String) -> Result<(), ApiError> {
        self.frontend()?.delete_server(&id).await
    }

    async fn provision_credentials(
        &self,
        provision: api::CredentialProvision,
    ) -> Result<api::SourceInfo, ApiError> {
        self.frontend()?.provision(provision).await
    }

    async fn login_source(
        &self,
        request: api::SourceLoginRequest,
    ) -> Result<api::SourceInfo, ApiError> {
        self.frontend()?.login_source(request).await
    }

    async fn clear_credentials(&self, id: String) -> Result<(), ApiError> {
        self.frontend()?.clear_credentials(&id).await
    }

    async fn authenticate_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        self.frontend()?.authenticate_source(&id).await
    }

    async fn browse_source(
        &self,
        id: String,
        path: String,
    ) -> Result<Vec<api::SourceFolderEntry>, ApiError> {
        self.frontend()?.browse_source(&id, &path).await
    }

    async fn integration_credentials(
        &self,
    ) -> Result<Vec<api::IntegrationCredentialStatus>, ApiError> {
        Ok(self.frontend()?.integration_credentials().await)
    }

    async fn provision_integration_credentials(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        self.frontend()?.provision_integration(provision).await
    }

    async fn clear_integration_credentials(
        &self,
        kind: api::IntegrationKind,
    ) -> Result<(), ApiError> {
        self.frontend()?.clear_integration(kind).await
    }

    async fn authenticate_integration(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        self.frontend()?.authenticate_integration(provision).await
    }

    async fn validate_source(&self, id: String) -> Result<api::SourceState, ApiError> {
        self.frontend()?.validate_source(&id).await
    }

    async fn external_access(&self, kind: String) -> Result<api::ExternalAccess, ApiError> {
        self.frontend()?.external_access(&kind).await
    }

    async fn set_external_playback(
        &self,
        _external: Option<api::ExternalPlayback>,
    ) -> Result<(), ApiError> {
        Err(ApiError::unsupported(
            "use the external playback lease operations",
        ))
    }

    async fn claim_external_playback(
        &self,
        external: api::ExternalPlayback,
    ) -> Result<api::ExternalPlaybackLease, ApiError> {
        self.frontend()?.claim_external(external).await
    }

    async fn report_external_playback(
        &self,
        report: api::ExternalPlaybackReport,
    ) -> Result<(), ApiError> {
        self.frontend()?.report_external(report).await
    }

    async fn release_external_playback(&self, lease_id: String) -> Result<(), ApiError> {
        self.frontend()?.release_external(&lease_id).await
    }

    async fn start_ytdlp(&self, request: api::YtdlpRequest) -> Result<api::JobRef, ApiError> {
        let (Some(runner), Some(config), Some(library)) = (&self.jobs, &self.config, &self.library)
        else {
            return Err(ApiError::unsupported("yt-dlp services are unavailable"));
        };
        crate::ytdlp::spawn(
            runner.clone(),
            config.clone(),
            library.clone(),
            self.session.clone(),
            request,
        )
    }

    async fn catalog(&self, continuation: Option<String>) -> Result<api::CatalogPage, ApiError> {
        self.frontend()?.catalog(continuation.as_deref()).await
    }

    async fn catalog_detail(
        &self,
        request: api::CatalogDetailRequest,
    ) -> Result<api::CatalogDetail, ApiError> {
        self.frontend()?.catalog_detail(request).await
    }

    async fn radio_stations(&self) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        Ok(self.frontend()?.radio_stations().await)
    }

    async fn track_radio(&self, key: String) -> Result<Vec<api::TrackInfo>, ApiError> {
        self.frontend()?.track_radio(&key).await
    }

    async fn playlist_radio(&self, id: String) -> Result<Vec<api::TrackInfo>, ApiError> {
        self.frontend()?.playlist_radio(&id).await
    }

    async fn search_radio(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        self.frontend()?.search_radio(&query, limit).await
    }

    async fn radio_registries(&self) -> Result<Vec<api::RadioRegistryInfo>, ApiError> {
        Ok(self.frontend()?.radio_registries().await)
    }

    async fn add_radio_registry(&self, url: String) -> Result<(), ApiError> {
        self.frontend()?.add_radio_registry(&url).await
    }

    async fn remove_radio_registry(&self, url: String) -> Result<(), ApiError> {
        self.frontend()?.remove_radio_registry(&url).await
    }

    async fn set_radio_registry_enabled(&self, url: String, enabled: bool) -> Result<(), ApiError> {
        self.frontend()?
            .set_radio_registry_enabled(&url, enabled)
            .await
    }

    async fn pin_radio_station(
        &self,
        station: api::RadioStationInfo,
        pinned: bool,
    ) -> Result<(), ApiError> {
        self.frontend()?.pin_station(station, pinned).await
    }

    async fn update_track_metadata(
        &self,
        patch: api::TrackMetadataPatch,
    ) -> Result<api::TrackInfo, ApiError> {
        self.frontend()?.update_track_metadata(patch).await
    }

    async fn delete_tracks(&self, keys: Vec<String>, from_disk: bool) -> Result<(), ApiError> {
        self.frontend()?.delete_tracks(&keys, from_disk).await
    }

    async fn delete_album(&self, id: String, from_disk: bool) -> Result<(), ApiError> {
        self.frontend()?.delete_album(&id, from_disk).await
    }

    async fn upload_artwork(&self, upload: api::ArtworkUpload) -> Result<(), ApiError> {
        self.frontend()?.upload_artwork(upload).await
    }

    async fn remove_artwork(&self, target: api::ArtworkTarget) -> Result<(), ApiError> {
        self.frontend()?.remove_artwork(target).await
    }

    async fn artwork(&self, request: api::ArtworkRequest) -> Result<api::ArtworkData, ApiError> {
        use crate::artwork::ArtworkEntity;
        let service = self
            .artwork
            .as_ref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without artwork"))?;
        let entity = match request
            .entity
            .as_ref()
            .ok_or_else(|| ApiError::invalid_input("artwork entity is required"))?
        {
            api::ArtworkEntity::Track { key } => ArtworkEntity::Track(key),
            api::ArtworkEntity::Album { id } => ArtworkEntity::Album(id),
            api::ArtworkEntity::Artist { name } => ArtworkEntity::Artist(name),
            api::ArtworkEntity::Playlist { id } => ArtworkEntity::Playlist(id),
        };
        let payload = service.fetch(entity, request.hq).await?;
        Ok(api::ArtworkData {
            content_type: payload.content_type.to_string(),
            data: payload.bytes,
        })
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

    async fn cancel_download_item(&self, key: String) -> Result<(), ApiError> {
        match &self.downloads {
            Some(service) => service.cancel_item(&key),
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
