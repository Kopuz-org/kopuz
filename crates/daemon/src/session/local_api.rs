//! `LocalApi`: the in-process implementation of [`api::KopuzApi`].

use super::*;
use crate::jobs::Trigger;

/// In-process implementation of [`api::KopuzApi`] over a running session.
pub struct LocalApi {
    pub(super) session: SessionHandle,
    /// When this surface came up, which is as close to the daemon's start as it has.
    pub(super) started: std::time::Instant,
    pub(super) library: Option<Arc<crate::library::LibraryService>>,
    pub(super) config: Option<Arc<crate::config_service::ConfigService>>,
    pub(super) prefs: Option<Arc<crate::prefs::PrefsService>>,
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
    pub(super) downloader: Option<Arc<crate::url_download::UrlDownloadService>>,
    pub(super) spotify: Option<Arc<crate::spotify::SpotifySink>>,
    pub(super) video: Option<Arc<crate::video::VideoService>>,
}

impl LocalApi {
    fn resync_after_account_switch(&self) {
        let Some(runner) = self.jobs.clone() else {
            return;
        };
        let (library, playlists, favorites) = (
            self.library.clone(),
            self.playlists.clone(),
            self.favorites.clone(),
        );
        let start = move |kind: api::JobKind| {
            let started = match kind {
                api::JobKind::LibrarySync => library
                    .as_ref()
                    .map(|library| library.spawn_remote_sync(&runner, Trigger::User)),
                api::JobKind::PlaylistSync => playlists
                    .as_ref()
                    .map(|playlists| playlists.spawn_sync(&runner, Trigger::User)),
                api::JobKind::FavoritesSync => favorites
                    .as_ref()
                    .map(|favorites| favorites.spawn_sync(&runner, Trigger::User)),
                _ => None,
            };
            started.unwrap_or_else(|| Err(ApiError::unsupported("no service for this sync")))
        };
        start_syncs(start, self.session.subscribe());
    }

    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            started: std::time::Instant::now(),
            library: None,
            config: None,
            prefs: None,
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
            downloader: None,
            spotify: None,
            video: None,
        }
    }

    pub fn with_video(mut self, video: Arc<crate::video::VideoService>) -> Self {
        self.video = Some(video);
        self
    }

    pub fn with_library(mut self, library: Arc<crate::library::LibraryService>) -> Self {
        self.library = Some(library);
        self
    }

    pub fn with_config(mut self, config: Arc<crate::config_service::ConfigService>) -> Self {
        self.config = Some(config);
        self
    }

    pub fn with_prefs(mut self, prefs: Arc<crate::prefs::PrefsService>) -> Self {
        self.prefs = Some(prefs);
        self
    }

    fn prefs(&self) -> Result<&crate::prefs::PrefsService, ApiError> {
        self.prefs
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a prefs service"))
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

    fn favorites_service(&self) -> Result<&crate::favorites::FavoritesService, ApiError> {
        self.favorites
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a favorites service"))
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

    async fn recovering<T, A>(&self, mut attempt: impl FnMut() -> A) -> Result<T, ApiError>
    where
        A: std::future::Future<Output = Result<T, ApiError>>,
    {
        match &self.sources {
            Some(sources) => sources.recovering(attempt).await,
            None => attempt().await,
        }
    }

    /// Someone is using the app: while the internet is down, see now whether it is back.
    fn acted(&self) {
        if let Some(sources) = &self.sources {
            sources.check_network_soon();
        }
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

    pub fn with_downloader(
        mut self,
        downloader: Arc<crate::url_download::UrlDownloadService>,
    ) -> Self {
        self.downloader = Some(downloader);
        self
    }

    fn config_service(&self) -> Result<&crate::config_service::ConfigService, ApiError> {
        self.config
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a config service"))
    }

    fn downloader(&self) -> Result<&crate::url_download::UrlDownloadService, ApiError> {
        self.downloader
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a downloader"))
    }

    pub fn with_spotify(mut self, spotify: Arc<crate::spotify::SpotifySink>) -> Self {
        self.spotify = Some(spotify);
        self
    }

    /// The sink that plays for a source. Only Spotify plays itself today, so a
    /// source that is not one is a client asking for something that is not
    /// there -- which its capabilities already said.
    async fn external_sink(
        &self,
        source_id: &str,
    ) -> Result<&Arc<crate::spotify::SpotifySink>, ApiError> {
        let sources = self
            .sources
            .as_ref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without sources"))?;
        if !sources
            .source_info(source_id)
            .await?
            .capabilities
            .external_devices
        {
            return Err(ApiError::unsupported(
                "this source plays no devices of its own",
            ));
        }
        self.spotify
            .as_ref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without Spotify playback"))
    }
}

#[async_trait::async_trait]
impl api::PlaylistApi for LocalApi {
    async fn playlists(&self) -> Result<api::PlaylistCatalog, ApiError> {
        self.playlists()?.catalog().await
    }

    async fn create_playlist(&self, name: String, keys: Vec<String>) -> Result<String, ApiError> {
        let (name, keys) = (&name, &keys);
        self.recovering(|| async move { self.playlists()?.create(name, keys).await })
            .await
    }

    async fn rename_playlist(&self, id: String, name: String) -> Result<(), ApiError> {
        let (id, name) = (&id, &name);
        self.recovering(|| async move { self.playlists()?.rename(id, name).await })
            .await
    }

    async fn edit_playlist(&self, id: String, edit: api::PlaylistEdit) -> Result<(), ApiError> {
        let (id, edit) = (&id, &edit);
        self.recovering(|| async move { self.playlists()?.edit(id, edit.clone()).await })
            .await
    }

    async fn delete_playlist(&self, id: String) -> Result<(), ApiError> {
        let id = &id;
        self.recovering(|| async move { self.playlists()?.delete(id).await })
            .await
    }

    async fn add_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        let (id, keys) = (&id, &keys);
        self.recovering(|| async move { self.playlists()?.add_tracks(id, keys).await })
            .await
    }

    async fn remove_playlist_track(&self, id: String, index: u32) -> Result<(), ApiError> {
        let id = &id;
        self.recovering(|| async move { self.playlists()?.remove_track(id, index).await })
            .await
    }

    async fn reorder_playlist(
        &self,
        id: String,
        reorder: api::PlaylistReorder,
    ) -> Result<(), ApiError> {
        let id = &id;
        self.recovering(|| async move { self.playlists()?.reorder(id, reorder).await })
            .await
    }

    async fn refresh_playlist(&self, id: String) -> Result<(), ApiError> {
        self.playlists()?.refresh(&id).await
    }

    async fn create_playlist_folder(&self, name: String) -> Result<String, ApiError> {
        let name = &name;
        self.recovering(|| async move { self.playlists()?.create_folder(name).await })
            .await
    }

    async fn rename_playlist_folder(&self, id: String, name: String) -> Result<(), ApiError> {
        let (id, name) = (&id, &name);
        self.recovering(|| async move { self.playlists()?.rename_folder(id, name).await })
            .await
    }

    async fn delete_playlist_folder(&self, id: String) -> Result<(), ApiError> {
        let id = &id;
        self.recovering(|| async move { self.playlists()?.delete_folder(id).await })
            .await
    }

    async fn move_playlist(
        &self,
        playlist_id: String,
        folder_id: Option<String>,
    ) -> Result<(), ApiError> {
        let (playlist_id, folder_id) = (&playlist_id, folder_id.as_deref());
        self.recovering(|| async move {
            self.playlists()?
                .move_playlist(playlist_id, folder_id)
                .await
        })
        .await
    }
}

#[async_trait::async_trait]
impl api::PlayerApi for LocalApi {
    async fn player_state(&self) -> Result<PlayerState, ApiError> {
        Ok(self.session.state())
    }

    async fn player_command(&self, command: PlayerCommand) -> Result<CommandAck, ApiError> {
        self.acted();
        self.session.player_command(command).await
    }

    async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        self.session.queue_window(page).await
    }

    async fn queue_snapshot(&self) -> Result<api::QueueSnapshot, ApiError> {
        let mirror = self.session.queue_mirror().await;
        Ok(api::QueueSnapshot {
            rev: self.session.state().queue.rev,
            items: mirror
                .tracks
                .iter()
                .map(|track| crate::wire::track_info(track, self.session.offline()))
                .collect(),
            shuffle_order: mirror
                .shuffle_order
                .iter()
                .map(|index| *index as u32)
                .collect(),
            position: (!mirror.tracks.is_empty()).then_some(mirror.position as u32),
            shuffle: mirror.shuffle,
            context: mirror.context,
        })
    }

    async fn set_queue(&self, request: SetQueueRequest) -> Result<CommandAck, ApiError> {
        self.acted();
        self.session.set_queue(request).await
    }

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError> {
        self.acted();
        self.session.queue_edit(edit).await
    }

    async fn external_devices(
        &self,
        source_id: String,
    ) -> Result<Vec<api::ExternalDevice>, ApiError> {
        self.external_sink(&source_id).await?.devices().await
    }

    async fn select_external_device(
        &self,
        source_id: String,
        device_id: Option<String>,
    ) -> Result<(), ApiError> {
        self.external_sink(&source_id)
            .await?
            .select_device(device_id)
            .await
    }

    async fn video(&self, request: api::VideoRequest) -> Result<api::VideoChunk, ApiError> {
        self.video
            .as_deref()
            .ok_or_else(|| ApiError::unsupported("this daemon runs without music videos"))?
            .chunk(request)
            .await
    }
}

#[async_trait::async_trait]
impl api::LibraryApi for LocalApi {
    async fn tracks(
        &self,
        filter: api::TrackFilter,
        page: Page,
    ) -> Result<api::TrackPage, ApiError> {
        self.acted();
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

    async fn albums(&self, query: api::AlbumQuery, page: Page) -> Result<api::AlbumPage, ApiError> {
        self.acted();
        self.library()?.albums(query, page).await
    }

    async fn albums_recently_added(&self, page: Page) -> Result<api::AlbumPage, ApiError> {
        self.library()?.albums_recently_added(page).await
    }

    async fn album(&self, id: String) -> Result<Option<api::AlbumInfo>, ApiError> {
        self.library()?.album(&id).await
    }

    async fn album_tracks(&self, id: String, page: Page) -> Result<api::TrackPage, ApiError> {
        self.library()?.album_tracks(&id, page).await
    }

    async fn artists(
        &self,
        query: api::ArtistQuery,
        page: Page,
    ) -> Result<api::ArtistPage, ApiError> {
        self.acted();
        self.library()?.artists(query, page).await
    }

    async fn catalog(&self, continuation: Option<String>) -> Result<api::CatalogPage, ApiError> {
        let continuation = continuation.as_deref();
        self.recovering(|| async move { self.catalog()?.catalog(continuation).await })
            .await
    }

    async fn catalog_detail(
        &self,
        request: api::CatalogDetailRequest,
    ) -> Result<api::CatalogDetail, ApiError> {
        let request = &request;
        self.recovering(|| async move { self.catalog()?.detail(request.clone()).await })
            .await
    }

    async fn for_you(&self) -> Result<api::ForYou, ApiError> {
        self.recovering(|| async move { self.catalog()?.for_you().await })
            .await
    }

    async fn refresh_for_you(&self) -> Result<api::ForYou, ApiError> {
        self.recovering(|| async move { self.catalog()?.refresh_for_you().await })
            .await
    }

    async fn discover_rows(&self) -> Result<api::DiscoverRows, ApiError> {
        self.recovering(|| async move { self.catalog()?.discover_rows().await })
            .await
    }

    async fn refresh_discover_rows(&self) -> Result<api::DiscoverRows, ApiError> {
        self.recovering(|| async move { self.catalog()?.refresh_discover_rows().await })
            .await
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
        let keys = &keys;
        self.recovering(|| async move { self.mutations()?.delete_tracks(keys, from_disk).await })
            .await
    }

    async fn delete_album(&self, id: String, from_disk: bool) -> Result<(), ApiError> {
        let id = &id;
        self.recovering(|| async move { self.mutations()?.delete_album(id, from_disk).await })
            .await
    }

    async fn upload_artwork(&self, upload: api::ArtworkUpload) -> Result<(), ApiError> {
        let upload = &upload;
        self.recovering(|| async move { self.mutations()?.upload_artwork(upload.clone()).await })
            .await
    }

    async fn remove_artwork(&self, target: api::ArtworkTarget) -> Result<(), ApiError> {
        let target = &target;
        self.recovering(|| async move { self.mutations()?.remove_artwork(target.clone()).await })
            .await
    }

    async fn artist_tracks(&self, artist: String, page: Page) -> Result<api::TrackPage, ApiError> {
        self.library()?.artist_tracks(&artist, page).await
    }

    async fn artist(&self, artist: String) -> Result<api::ArtistDetail, ApiError> {
        self.acted();
        self.library()?.artist(&artist).await
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

    async fn search(&self, request: api::SearchRequest) -> Result<api::SearchResults, ApiError> {
        self.acted();
        match (&request.filter, &request.continuation) {
            (None, None) => self.library()?.search(&request.query).await,
            _ => self.catalog()?.search(request).await,
        }
    }

    async fn search_suggestions(
        &self,
        query: String,
    ) -> Result<Vec<api::SearchSuggestion>, ApiError> {
        self.catalog()?.search_suggestions(&query).await
    }

    async fn track_web_url(&self, key: String) -> Result<Option<String>, ApiError> {
        self.library()?.track_web_url(&key).await
    }

    async fn album_web_url(&self, id: String) -> Result<Option<String>, ApiError> {
        self.library()?.album_web_url(&id).await
    }

    async fn refresh_artist_artwork(&self, artists: Vec<String>) -> Result<(), ApiError> {
        self.library()?.refresh_artist_artwork(artists).await
    }

    async fn favorites(&self) -> Result<api::FavoritesView, ApiError> {
        self.recovering(|| async move { self.favorites_service()?.list().await })
            .await
    }

    async fn set_favorite(&self, key: String, favorite: bool) -> Result<(), ApiError> {
        let key = &key;
        self.recovering(|| async move { self.favorites_service()?.set(key, favorite).await })
            .await
    }

    async fn dont_recommend(&self, key: String) -> Result<(), ApiError> {
        let key = &key;
        self.recovering(|| async move { self.favorites_service()?.dont_recommend(key).await })
            .await
    }

    async fn rate(&self, item_ref: String, rating: api::Rating) -> Result<(), ApiError> {
        let item_ref = &item_ref;
        self.recovering(|| async move { self.favorites_service()?.rate(item_ref, rating).await })
            .await
    }

    async fn follow(&self, artist_ref: String, follow: bool) -> Result<(), ApiError> {
        let artist_ref = &artist_ref;
        self.recovering(|| async move { self.catalog()?.follow(artist_ref, follow).await })
            .await
    }

    async fn save(&self, item_ref: String, saved: bool) -> Result<(), ApiError> {
        let item_ref = &item_ref;
        self.recovering(|| async move { self.catalog()?.save(item_ref, saved).await })
            .await
    }

    async fn remove_from_history(&self, token: String) -> Result<(), ApiError> {
        let token = &token;
        self.recovering(|| async move { self.catalog()?.remove_from_history(token).await })
            .await
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
            Some(library) => library.stats().await,
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

    async fn artwork_settings(&self) -> Result<Vec<api::FieldSpec>, ApiError> {
        let config = self.config_service()?.snapshot().await;
        Ok(crate::artwork::settings::fields(&config))
    }

    async fn set_artwork_settings(
        &self,
        values: Vec<api::FieldValue>,
    ) -> Result<Vec<api::FieldSpec>, ApiError> {
        let service = self.config_service()?;
        let keys = crate::artwork::settings::written_keys(&values);
        service.ensure_unlocked(&keys).await?;
        let updated = service
            .mutate_state(&keys, move |config| {
                crate::artwork::settings::apply(&values, config)
            })
            .await?;
        Ok(crate::artwork::settings::fields(&updated))
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

    async fn edit_config(
        &self,
        edits: Vec<config::SettingsEdit>,
    ) -> Result<api::EditReport, ApiError> {
        self.config_service()?.edit(edits).await
    }

    async fn preview_equalizer(
        &self,
        equalizer: config::EqualizerSettings,
    ) -> Result<(), ApiError> {
        self.session.preview_equalizer(equalizer);
        Ok(())
    }

    async fn preview_replay_gain(
        &self,
        replay_gain: config::ReplayGainSettings,
    ) -> Result<(), ApiError> {
        self.session.preview_replay_gain(replay_gain);
        Ok(())
    }

    async fn daemon_status(&self) -> Result<api::DaemonStatus, ApiError> {
        Ok(api::DaemonStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: self.started.elapsed().as_secs(),
            proto_revision: api::WIRE_REVISION,
        })
    }
}

#[async_trait::async_trait]
impl api::PrefsApi for LocalApi {
    async fn frontend_prefs(&self, frontend: String) -> Result<Vec<api::FrontendPref>, ApiError> {
        self.prefs()?.get(&frontend).await
    }

    async fn set_frontend_prefs(
        &self,
        frontend: String,
        entries: Vec<api::PrefEntry>,
    ) -> Result<(), ApiError> {
        self.prefs()?.set(&frontend, entries).await
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
        let started = match kind {
            api::JobKind::Scan => match &self.library {
                Some(library) => library.spawn_scan(runner, Trigger::User),
                None => Err(ApiError::unsupported("no library service")),
            },
            api::JobKind::LibrarySync => match &self.library {
                Some(library) => library.spawn_remote_sync(runner, Trigger::User),
                None => Err(ApiError::unsupported("no library service")),
            },
            api::JobKind::FavoritesSync => match &self.favorites {
                Some(favorites) => favorites.spawn_sync(runner, Trigger::User),
                None => Err(ApiError::unsupported("no favorites service")),
            },
            api::JobKind::PlaylistSync => match &self.playlists {
                Some(playlists) => playlists.spawn_sync(runner, Trigger::User),
                None => Err(ApiError::unsupported("no playlist service")),
            },
            // These carry their own request, so they start through their own
            // method rather than by kind.
            api::JobKind::Download | api::JobKind::UrlDownload | api::JobKind::Unknown => {
                Err(ApiError::unsupported("this job kind has no direct starter"))
            }
        };
        // These kinds carry no request, so a second start is the same job and joins it.
        started.or_else(|error| match error.code {
            api::ErrorCode::Conflict => runner.running(kind).ok_or(error),
            _ => Err(error),
        })
    }

    async fn download_url(&self, url: String, format: String) -> Result<api::JobRef, ApiError> {
        let (Some(service), Some(runner)) = (&self.downloader, &self.jobs) else {
            return Err(ApiError::unsupported(
                "this daemon runs without a downloader",
            ));
        };
        service.start(runner, url, format).await
    }

    async fn search_downloads(
        &self,
        query: String,
    ) -> Result<Vec<api::DownloadCandidate>, ApiError> {
        self.downloader()?.search(&query).await
    }

    async fn download_formats(&self) -> Result<Vec<api::ChoiceOption>, ApiError> {
        Ok(self.downloader()?.formats())
    }

    async fn downloader_settings(&self) -> Result<Vec<api::FieldSpec>, ApiError> {
        Ok(self.downloader()?.settings().await)
    }

    async fn set_downloader_settings(
        &self,
        values: Vec<api::FieldValue>,
    ) -> Result<Vec<api::FieldSpec>, ApiError> {
        self.downloader()?.set_settings(values).await
    }

    async fn downloader_history(&self) -> Result<Vec<api::DownloadHistoryEntry>, ApiError> {
        self.downloader()?.history().await
    }

    async fn clear_downloader_history(&self) -> Result<(), ApiError> {
        self.downloader()?.clear_history().await
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

    async fn offline_cache_status(&self) -> Result<api::OfflineCacheStatus, ApiError> {
        Ok(self.session.offline().cache().status())
    }

    async fn clear_offline_cache(&self) -> Result<(), ApiError> {
        self.session.offline().cache().clear().await
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
    async fn storage_quota(&self, id: String) -> Result<api::StorageQuota, ApiError> {
        self.sources()?.storage_quota(&id).await
    }

    async fn upload_track(&self, upload: api::TrackUpload) -> Result<(), ApiError> {
        self.sources()?.upload_track(upload).await
    }

    async fn sources(&self) -> Result<Vec<api::SourceInfo>, ApiError> {
        self.sources()?.sources().await
    }

    async fn services(&self) -> Result<Vec<api::ServiceInfo>, ApiError> {
        Ok(self.sources()?.services().await)
    }

    async fn check_source_draft(
        &self,
        draft: api::SourceDraft,
    ) -> Result<api::DraftCheck, ApiError> {
        self.sources()?.check_source_draft(draft).await
    }

    async fn probe_source_draft(
        &self,
        draft: api::SourceDraft,
    ) -> Result<api::DraftProbe, ApiError> {
        self.sources()?.probe_source_draft(draft).await
    }

    async fn set_source_settings(
        &self,
        id: String,
        values: Vec<api::FieldValue>,
    ) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.set_source_settings(&id, values).await
    }

    async fn switch_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.switch_source(&id).await
    }

    async fn upsert_source(&self, draft: api::SourceDraft) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.upsert_source(draft).await
    }

    async fn delete_source(&self, id: String) -> Result<(), ApiError> {
        self.sources()?.delete_source(&id).await
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

    async fn browser_sessions(
        &self,
        service: String,
    ) -> Result<Vec<api::BrowserSession>, ApiError> {
        self.sources()?.browser_sessions(&service).await
    }

    async fn import_browser_session(
        &self,
        id: String,
        session: String,
    ) -> Result<api::SourceInfo, ApiError> {
        self.sources()?.import_browser_session(&id, &session).await
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

    async fn network_status(&self) -> Result<api::NetworkStatus, ApiError> {
        Ok(self.sources()?.network_status())
    }

    async fn set_offline(&self, offline: bool) -> Result<api::NetworkStatus, ApiError> {
        Ok(self.sources()?.set_offline(offline))
    }

    async fn accounts(&self, id: String) -> Result<Vec<api::SourceAccount>, ApiError> {
        self.sources()?.accounts(&id).await
    }

    async fn switch_account(
        &self,
        id: String,
        account: Option<String>,
    ) -> Result<api::SourceInfo, ApiError> {
        let (info, resync) = self.sources()?.switch_account(&id, account).await?;
        if resync {
            self.resync_after_account_switch();
        }
        Ok(info)
    }

    async fn test_source_connection(
        &self,
        id: String,
    ) -> Result<Vec<api::ConnectionCheck>, ApiError> {
        self.sources()?.test_connection(&id).await
    }

    async fn can_open_browser(&self) -> Result<bool, ApiError> {
        Ok(self.sources()?.can_open_browser().await)
    }

    async fn integrations(&self) -> Result<Vec<api::IntegrationInfo>, ApiError> {
        Ok(self.integrations()?.list().await)
    }

    async fn set_integration_settings(
        &self,
        id: String,
        values: Vec<api::FieldValue>,
    ) -> Result<api::IntegrationInfo, ApiError> {
        self.integrations()?.set_settings(&id, values).await
    }

    async fn clear_integration(&self, id: String) -> Result<(), ApiError> {
        self.integrations()?.clear(&id).await
    }

    async fn authenticate_integration(&self, id: String) -> Result<api::IntegrationInfo, ApiError> {
        self.integrations()?.authenticate(&id).await
    }

    async fn browser_profiles(&self) -> Result<Vec<api::BrowserProfile>, ApiError> {
        Ok(crate::integrations::browser_profiles())
    }

    async fn connect_integration_from_browser(
        &self,
        id: String,
        profile: String,
    ) -> Result<api::IntegrationInfo, ApiError> {
        self.integrations()?
            .connect_from_browser(&id, &profile)
            .await
    }
}

/// Start the three sync kinds. A sync already running belongs to the old
/// account, so a kind that conflicts starts again once that run finishes.
/// `events` must be subscribed before the call, or a finish that lands right
/// after a conflict is missed.
fn start_syncs(
    mut start: impl FnMut(api::JobKind) -> Result<api::JobRef, ApiError> + Send + 'static,
    mut events: broadcast::Receiver<ApiEvent>,
) -> Option<tokio::task::JoinHandle<()>> {
    let mut waiting = Vec::new();
    let mut attempt = move |kind: api::JobKind, waiting: &mut Vec<api::JobKind>| {
        waiting.retain(|queued| *queued != kind);
        match start(kind) {
            Ok(_) => {}
            Err(error) if error.code == api::ErrorCode::Conflict => waiting.push(kind),
            Err(error) => {
                tracing::debug!(%error, ?kind, "sync after an account switch not started")
            }
        }
    };
    for kind in [
        api::JobKind::LibrarySync,
        api::JobKind::PlaylistSync,
        api::JobKind::FavoritesSync,
    ] {
        attempt(kind, &mut waiting);
    }
    if waiting.is_empty() {
        return None;
    }
    Some(tokio::spawn(async move {
        while !waiting.is_empty() {
            let due = match events.recv().await {
                Ok(ApiEvent::JobFinished { kind, .. }) if waiting.contains(&kind) => vec![kind],
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => waiting.clone(),
                Err(broadcast::error::RecvError::Closed) => return,
            };
            for kind in due {
                attempt(kind, &mut waiting);
            }
        }
    }))
}

#[cfg(test)]
mod start_syncs_tests {
    use super::*;
    use std::sync::Mutex;

    fn finished(kind: api::JobKind) -> ApiEvent {
        ApiEvent::JobFinished {
            id: "job-1".into(),
            kind,
            ok: true,
            error: None,
            automatic: false,
        }
    }

    #[tokio::test]
    async fn a_sync_already_running_starts_again_when_it_finishes() {
        let running = Arc::new(Mutex::new(vec![api::JobKind::LibrarySync]));
        let started = Arc::new(Mutex::new(Vec::new()));
        let start = {
            let (running, started) = (running.clone(), started.clone());
            move |kind| {
                if running.lock().unwrap().contains(&kind) {
                    return Err(ApiError::new(api::ErrorCode::Conflict, "running"));
                }
                started.lock().unwrap().push(kind);
                Ok(api::JobRef {
                    job_id: "job-2".into(),
                })
            }
        };
        let (events, rx) = broadcast::channel(8);
        let retry = start_syncs(start, rx).expect("library sync was busy");
        assert_eq!(
            *started.lock().unwrap(),
            [api::JobKind::PlaylistSync, api::JobKind::FavoritesSync]
        );

        events.send(finished(api::JobKind::PlaylistSync)).unwrap();
        running.lock().unwrap().clear();
        events.send(finished(api::JobKind::LibrarySync)).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), retry)
            .await
            .expect("retry finishes once every kind started")
            .unwrap();
        assert_eq!(
            *started.lock().unwrap(),
            [
                api::JobKind::PlaylistSync,
                api::JobKind::FavoritesSync,
                api::JobKind::LibrarySync
            ]
        );
    }

    #[test]
    fn nothing_waits_when_every_sync_starts() {
        let (_events, rx) = broadcast::channel(8);
        let start = |_| {
            Ok(api::JobRef {
                job_id: "job-1".into(),
            })
        };
        assert!(start_syncs(start, rx).is_none());
    }
}
