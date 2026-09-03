use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{ApiError, ErrorCode};
use futures_util::{StreamExt, stream};
use server::source::{AuthOutcome, SourceError};

use crate::{ConfigService, LibraryService, SessionHandle};

pub struct FrontendService {
    db: db::Db,
    config: Arc<ConfigService>,
    library: Arc<LibraryService>,
    session: SessionHandle,
    registry: tokio::sync::RwLock<radio::registry::StationRegistry>,
    uploads: PathBuf,
    external_lease: Arc<tokio::sync::Mutex<Option<ExternalLease>>>,
}

struct ExternalLease {
    id: String,
    expires_at: Instant,
}

const EXTERNAL_LEASE_TTL: Duration = Duration::from_secs(15);

impl FrontendService {
    pub fn new(
        db: db::Db,
        config: Arc<ConfigService>,
        library: Arc<LibraryService>,
        session: SessionHandle,
        uploads: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            config,
            library,
            session,
            registry: tokio::sync::RwLock::new(radio::registry::StationRegistry::new()),
            uploads,
            external_lease: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    async fn current(&self) -> config::AppConfig {
        self.config.snapshot().await
    }

    fn track_page(
        tracks: &[reader::Track],
        page: api::Page,
        config: &config::AppConfig,
    ) -> api::TrackPage {
        api::TrackPage {
            total: tracks.len().min(u32::MAX as usize) as u32,
            offset: page.offset,
            items: tracks
                .iter()
                .skip(page.offset as usize)
                .take(page.limit as usize)
                .map(|track| crate::wire::track_info(track, config))
                .collect(),
        }
    }

    async fn source(&self) -> server::source::ActiveSource {
        let config = self.current().await;
        Arc::from(server::source::active(self.db.clone(), &config))
    }

    fn db_error(error: db::DbError) -> ApiError {
        crate::error::db(error)
    }

    fn validate_server_id(id: &str) -> Result<(), ApiError> {
        if id.trim().is_empty() {
            return Err(ApiError::invalid_input("server id is required"));
        }
        if id == "local" || id.starts_with("local:") {
            return Err(ApiError::invalid_input(
                "server id uses the reserved local source namespace",
            ));
        }
        Ok(())
    }

    fn persisted_track(value: &api::TrackInfo) -> Result<reader::Track, ApiError> {
        if value.key.trim().is_empty() {
            return Err(ApiError::invalid_input("persisted track key is required"));
        }
        let id = match value.service {
            Some(service) => reader::TrackId::Server {
                service: crate::wire::music_service_from_api(service).ok_or_else(|| {
                    ApiError::invalid_input("persisted track names an unknown media service")
                })?,
                item_id: value.key.clone(),
            },
            None => reader::TrackId::Local(PathBuf::from(&value.key)),
        };
        if !value.uid.is_empty() && value.uid != id.uid() {
            return Err(ApiError::invalid_input(
                "persisted track uid does not match its key",
            ));
        }
        Ok(crate::wire::track_from_info_parts(value, id, None))
    }

    pub async fn queue_snapshot(&self) -> Result<api::QueuePersistenceSnapshot, ApiError> {
        let snapshot = self.db.load_queue().await.map_err(Self::db_error)?;
        let config = self.current().await;
        Ok(api::QueuePersistenceSnapshot {
            tracks: snapshot
                .queue
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
            current_index: snapshot
                .current_queue_index
                .try_into()
                .map_err(|_| ApiError::internal("persisted queue index is too large"))?,
            progress_ms: snapshot.progress_secs.saturating_mul(1000),
            shuffle_order: snapshot
                .shuffle_order
                .into_iter()
                .map(|index| {
                    index
                        .try_into()
                        .map_err(|_| ApiError::internal("persisted shuffle index is too large"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            shuffle_enabled: snapshot.shuffle_enabled,
        })
    }

    pub async fn save_queue_snapshot(
        &self,
        snapshot: api::QueuePersistenceSnapshot,
    ) -> Result<(), ApiError> {
        if snapshot.tracks.len() > u32::MAX as usize {
            return Err(ApiError::invalid_input("queue snapshot is too large"));
        }
        if snapshot.tracks.is_empty() {
            if snapshot.current_index != 0 || !snapshot.shuffle_order.is_empty() {
                return Err(ApiError::invalid_input(
                    "an empty queue snapshot cannot contain positions",
                ));
            }
        } else if snapshot.current_index as usize >= snapshot.tracks.len() {
            return Err(ApiError::invalid_input(
                "current queue index is outside the snapshot",
            ));
        }
        if snapshot.shuffle_enabled
            && !snapshot.tracks.is_empty()
            && snapshot.shuffle_order.len() != snapshot.tracks.len()
        {
            return Err(ApiError::invalid_input(
                "enabled shuffle requires a complete queue permutation",
            ));
        }
        if !snapshot.shuffle_order.is_empty() {
            let mut seen = vec![false; snapshot.tracks.len()];
            for index in &snapshot.shuffle_order {
                let index = *index as usize;
                if index >= seen.len() || std::mem::replace(&mut seen[index], true) {
                    return Err(ApiError::invalid_input(
                        "shuffle order must be a queue permutation",
                    ));
                }
            }
            if seen.iter().any(|seen| !seen) {
                return Err(ApiError::invalid_input(
                    "shuffle order must include every queue position",
                ));
            }
        }

        let tracks = snapshot
            .tracks
            .iter()
            .map(|value| {
                let decoded = Self::persisted_track(value)?;
                Ok(self
                    .library
                    .transient_track_for_info(value)
                    .unwrap_or(decoded))
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        let persisted = db::QueueSnapshot {
            version: 1,
            queue: tracks,
            current_queue_index: snapshot.current_index as usize,
            progress_secs: snapshot.progress_ms / 1000,
            shuffle_order: snapshot
                .shuffle_order
                .into_iter()
                .map(|index| index as usize)
                .collect(),
            shuffle_enabled: snapshot.shuffle_enabled,
        };
        self.db.save_queue(&persisted).await.map_err(Self::db_error)
    }

    fn source_error(error: SourceError) -> ApiError {
        crate::error::source(error)
    }

    fn album_info(album: &reader::Album) -> api::AlbumInfo {
        api::AlbumInfo {
            id: album.id.clone(),
            title: album.title.clone(),
            artist: album.artist.clone(),
            genre: album.genre.clone(),
            year: u32::from(album.year),
            artwork: album.cover_path.as_ref().map(|_| album.id.clone()),
            manual_artwork: album.manual_cover,
        }
    }

    fn playlist_catalog(store: &reader::PlaylistStore) -> api::PlaylistCatalog {
        api::PlaylistCatalog {
            playlists: store
                .playlists
                .iter()
                .map(|playlist| api::PlaylistInfo {
                    id: playlist.id.clone(),
                    name: playlist.name.clone(),
                    track_count: playlist.tracks.len().min(u32::MAX as usize) as u32,
                    track_keys: playlist.tracks.clone(),
                    artwork: (playlist.cover_path.is_some() || playlist.image_tag.is_some())
                        .then(|| playlist.id.clone()),
                    manual_artwork: playlist.cover_path.is_some(),
                })
                .collect(),
            folders: store
                .folders
                .iter()
                .map(|folder| api::PlaylistFolderInfo {
                    id: folder.id.clone(),
                    name: folder.name.clone(),
                    playlist_ids: folder.playlist_ids.clone(),
                })
                .collect(),
        }
    }

    async fn playlist(
        &self,
        config: &config::AppConfig,
        id: &str,
    ) -> Result<reader::models::Playlist, ApiError> {
        self.db
            .load_playlists(&config.active_source)
            .await
            .map_err(Self::db_error)?
            .playlists
            .into_iter()
            .find(|playlist| playlist.id == id)
            .ok_or_else(|| ApiError::not_found("playlist not found"))
    }

    fn capabilities(value: server::source::Capabilities) -> api::SourceCapabilities {
        api::SourceCapabilities {
            edit_tags: value.edit_tags,
            delete_from_disk: value.delete_from_disk,
            scan_folders: value.scan_folders,
            folders: value.folders,
            sync: value.sync,
            downloads: value.downloads,
            discover: value.discover,
            track_radio: value.radio.track,
            playlist_radio: value.radio.playlist,
            playlists: match value.playlists {
                server::source::PlaylistOps::None => api::PlaylistCapability::None,
                server::source::PlaylistOps::AddRemove => api::PlaylistCapability::AddRemove,
                server::source::PlaylistOps::Reorder => api::PlaylistCapability::Reorder,
            },
            artists: match value.artist_view {
                server::source::ArtistView::Library => api::ArtistPresentation::Library,
                server::source::ArtistView::Remote => api::ArtistPresentation::Remote,
            },
            albums: match value.albums {
                server::source::AlbumType::Standard => api::AlbumPresentation::Standard,
                server::source::AlbumType::YtMusic => api::AlbumPresentation::Remote,
            },
            favorites_sync: match value.favorites_sync {
                server::source::FavoritesSync::Instant => api::FavoritesSyncMode::Instant,
                server::source::FavoritesSync::Paginated => api::FavoritesSyncMode::Paginated,
            },
        }
    }

    async fn source_for(
        &self,
        id: &str,
    ) -> Result<(config::AppConfig, server::source::ActiveSource), ApiError> {
        let mut config = self.current().await;
        let source = config::Source::from_column(id);
        if let config::Source::LocalLibrary(local_id) = &source {
            if !config
                .local_sources
                .iter()
                .any(|saved| saved.id == *local_id)
            {
                return Err(ApiError::not_found("local source not found"));
            }
            config.set_active_local_source(source.clone());
        } else if let config::Source::Server(server_id) = &source {
            let server = self
                .db
                .load_server(server_id)
                .await
                .map_err(Self::db_error)?
                .ok_or_else(|| ApiError::not_found("server not found"))?;
            config.set_active_server_snapshot(server);
        } else {
            config.set_active_local_source(config::Source::Local);
        }
        let active = Arc::from(server::source::active(self.db.clone(), &config));
        Ok((config, active))
    }

    async fn source_info_for(&self, id: &str) -> Result<api::SourceInfo, ApiError> {
        let current = self.current().await;
        let (resolved, source) = self.source_for(id).await?;
        let source_key = resolved.active_source.clone();
        let (name, kind, service, authenticated, url, browser, anonymous, storefront, language) =
            match &source_key {
                config::Source::Local => (
                    "Local Library".to_string(),
                    api::SourceKind::Local,
                    None,
                    true,
                    None,
                    None,
                    false,
                    None,
                    None,
                ),
                config::Source::LocalLibrary(local_id) => {
                    let saved = resolved
                        .local_sources
                        .iter()
                        .find(|saved| saved.id == *local_id)
                        .ok_or_else(|| ApiError::not_found("local source not found"))?;
                    (
                        saved.name.clone(),
                        api::SourceKind::LocalLibrary,
                        None,
                        true,
                        None,
                        None,
                        false,
                        None,
                        None,
                    )
                }
                config::Source::Server(_) => {
                    let server = resolved
                        .server
                        .as_ref()
                        .ok_or_else(|| ApiError::not_found("server not found"))?;
                    (
                        server.name.clone(),
                        api::SourceKind::Server,
                        Some(crate::wire::music_service_to_api(server.service)),
                        server.access_token.is_some() || server.yt_anonymous,
                        Some(server.url.clone()),
                        server.yt_browser.map(|browser| browser.id().to_string()),
                        server.yt_anonymous,
                        Some(server.apple_music_storefront.clone()),
                        Some(server.apple_music_language.clone()),
                    )
                }
            };
        let directories = match &source_key {
            config::Source::Local => resolved
                .music_directory
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            config::Source::LocalLibrary(local_id) => resolved
                .local_sources
                .iter()
                .find(|source| source.id == *local_id)
                .map(|source| {
                    source
                        .directories
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default(),
            config::Source::Server(server_id) => resolved.folders_for(server_id),
        };
        Ok(api::SourceInfo {
            id: source_key.as_str().to_string(),
            name,
            kind,
            service,
            active: current.active_source.as_str() == source_key.as_str(),
            authenticated,
            capabilities: Self::capabilities(source.capabilities()),
            url,
            browser,
            anonymous,
            storefront,
            language,
            directories,
        })
    }

    async fn publish_config(&self, updated: config::AppConfig, changed: Vec<String>) {
        self.session
            .set_active_source(Some(Arc::from(server::source::active(
                self.db.clone(),
                &updated,
            ))));
        self.session.set_config(updated, changed);
    }

    async fn reset_playback(&self) -> Result<(), ApiError> {
        *self.external_lease.lock().await = None;
        self.session.reset_playback().await?;
        Ok(())
    }

    pub(crate) async fn finish_source_change(
        &self,
        updated: config::AppConfig,
        changed: Vec<String>,
    ) -> Result<(), ApiError> {
        self.publish_config(updated, changed).await;
        self.reset_playback().await?;
        for table in [
            api::Table::Servers,
            api::Table::Tracks,
            api::Table::Albums,
            api::Table::Playlists,
            api::Table::Folders,
            api::Table::Favorites,
            api::Table::Recents,
        ] {
            self.library.invalidate(table);
        }
        Ok(())
    }

    pub async fn albums(
        &self,
        filter: api::AlbumFilter,
        page: api::Page,
    ) -> Result<api::AlbumPage, ApiError> {
        let config = self.current().await;
        let mut albums = self
            .db
            .albums(&config.active_source)
            .await
            .map_err(Self::db_error)?;
        if let Some(search) = filter
            .search
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let search = search.to_lowercase();
            albums.retain(|album| {
                album.title.to_lowercase().contains(&search)
                    || album.artist.to_lowercase().contains(&search)
            });
        }
        if let Some(artist) = filter.artist.as_deref() {
            albums.retain(|album| album.artist.eq_ignore_ascii_case(artist));
        }
        if let Some(genre) = filter.genre.as_deref() {
            albums.retain(|album| {
                album
                    .genre
                    .split(['/', ';', ','])
                    .any(|value| value.trim().eq_ignore_ascii_case(genre))
            });
        }
        match filter.sort.as_deref() {
            Some("title") => albums.sort_by_key(|album| album.title.to_lowercase()),
            Some("year") => albums.sort_by_key(|album| album.year),
            Some("genre") => albums.sort_by_key(|album| album.genre.to_lowercase()),
            _ => albums
                .sort_by_key(|album| (album.artist.to_lowercase(), album.title.to_lowercase())),
        }
        let total = albums.len().min(u32::MAX as usize) as u32;
        let items = albums
            .iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .map(Self::album_info)
            .collect();
        Ok(api::AlbumPage {
            total,
            offset: page.offset,
            items,
        })
    }

    pub async fn album(&self, id: &str) -> Result<api::AlbumInfo, ApiError> {
        let config = self.current().await;
        self.db
            .album(&config.active_source, id)
            .await
            .map_err(Self::db_error)?
            .as_ref()
            .map(Self::album_info)
            .ok_or_else(|| ApiError::not_found("album not found"))
    }

    pub async fn artists(&self, page: api::Page) -> Result<api::ArtistPage, ApiError> {
        let config = self.current().await;
        let artists = self
            .db
            .artists(&config.active_source)
            .await
            .map_err(Self::db_error)?;
        let albums = self
            .db
            .albums(&config.active_source)
            .await
            .map_err(Self::db_error)?;
        let images = self.db.artist_images().await.map_err(Self::db_error)?;
        let total = artists.len().min(u32::MAX as usize) as u32;
        let items = artists
            .iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .map(|(name, count)| {
                let normalized = utils::artist::normalize_artist_key(name);
                api::ArtistInfo {
                    name: name.clone(),
                    track_count: *count,
                    album_count: albums
                        .iter()
                        .filter(|album| album.artist.eq_ignore_ascii_case(name))
                        .count()
                        .min(u32::MAX as usize) as u32,
                    artwork: (images.0.contains_key(&normalized)
                        || images.1.contains_key(&normalized))
                    .then(|| name.clone()),
                    manual_artwork: images.0.contains_key(&normalized),
                }
            })
            .collect();
        Ok(api::ArtistPage {
            total,
            offset: page.offset,
            items,
        })
    }

    pub async fn refresh_artist_artwork(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<String>, ApiError> {
        const MISS_KIND: &str = "artist_photo_miss";
        const MISS_TTL_SECS: i64 = 86_400;

        let mut names: Vec<String> = names
            .into_iter()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        let source = self.source().await;
        let mut changed = false;
        match source.capabilities().artist_view {
            server::source::ArtistView::Library if source.capabilities().sync => {
                let images = source
                    .fetch_artist_images()
                    .await
                    .map_err(Self::source_error)?;
                for (name, url) in images {
                    source
                        .set_artist_image(
                            &utils::artist::normalize_artist_key(&name),
                            "server",
                            Some(&url),
                        )
                        .await
                        .map_err(Self::source_error)?;
                    changed = true;
                }
            }
            server::source::ArtistView::Remote => {
                let fresh_misses: std::collections::HashSet<String> = self
                    .db
                    .meta_keys_since(MISS_KIND, MISS_TTL_SECS)
                    .await
                    .map_err(Self::db_error)?
                    .into_iter()
                    .collect();
                let images = self.db.artist_images().await.map_err(Self::db_error)?;
                let pending: Vec<String> = names
                    .iter()
                    .filter(|name| {
                        let normalized = utils::artist::normalize_artist_key(name);
                        !images.0.contains_key(&normalized)
                            && !images.1.contains_key(&normalized)
                            && !fresh_misses.contains(&normalized)
                    })
                    .cloned()
                    .collect();
                let outcomes: Vec<bool> = stream::iter(pending.into_iter().map(|name| {
                    let source = source.clone();
                    async move {
                        let normalized = utils::artist::normalize_artist_key(&name);
                        match source.fetch_artist_image(&name).await {
                            Ok(Some(url)) => source
                                .set_artist_image(&normalized, "server", Some(&url))
                                .await
                                .is_ok(),
                            Ok(None) => {
                                let _ = source.set_meta(&normalized, MISS_KIND, "").await;
                                false
                            }
                            Err(_) => false,
                        }
                    }
                }))
                .buffer_unordered(6)
                .collect()
                .await;
                changed = outcomes.into_iter().any(|hit| hit);
            }
            server::source::ArtistView::Library => {}
        }
        if changed {
            self.library.invalidate(api::Table::Tracks);
        }
        let (overrides, photos) = self.db.artist_images().await.map_err(Self::db_error)?;
        Ok(names
            .into_iter()
            .filter(|name| {
                let normalized = utils::artist::normalize_artist_key(name);
                overrides.contains_key(&normalized) || photos.contains_key(&normalized)
            })
            .collect())
    }

    pub async fn genres(&self) -> Result<Vec<String>, ApiError> {
        let config = self.current().await;
        self.db
            .genres(&config.active_source)
            .await
            .map_err(Self::db_error)
    }

    pub async fn recent_tracks(&self, page: api::Page) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let keys = self
            .db
            .recently_played(&config.active_source, u32::MAX)
            .await
            .map_err(Self::db_error)?;
        let total = keys.len().min(u32::MAX as usize) as u32;
        let page_keys: Vec<String> = keys
            .into_iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .collect();
        let tracks = self
            .db
            .tracks_by_keys(&config.active_source, &page_keys)
            .await
            .map_err(Self::db_error)?;
        Ok(api::TrackPage {
            total,
            offset: page.offset,
            items: tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn album_tracks(
        &self,
        id: &str,
        page: api::Page,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .album_tracks(&config.active_source, id)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::track_page(&tracks, page, &config))
    }

    pub async fn artist_tracks(
        &self,
        name: &str,
        page: api::Page,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .artist_tracks(&config.active_source, name, None)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::track_page(&tracks, page, &config))
    }

    pub async fn genre_tracks(
        &self,
        name: &str,
        page: api::Page,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .genre_tracks(&config.active_source, name)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::track_page(&tracks, page, &config))
    }

    pub async fn artist_sample_tracks(&self, page: api::Page) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .artist_sample_tracks(&config.active_source, u32::MAX)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::track_page(&tracks, page, &config))
    }

    pub async fn tracks_by_keys(&self, keys: &[String]) -> Result<Vec<api::TrackInfo>, ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .tracks_by_keys(&config.active_source, keys)
            .await
            .map_err(Self::db_error)?;
        Ok(tracks
            .iter()
            .map(|track| crate::wire::track_info(track, &config))
            .collect())
    }

    pub async fn track_web_url(&self, key: &str) -> Result<Option<String>, ApiError> {
        let config = self.current().await;
        let track = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(Self::db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("track not found"))?;
        Ok(self.source().await.web_url(&track))
    }

    pub async fn album_web_url(&self, id: &str) -> Result<Option<String>, ApiError> {
        if id.trim().is_empty() {
            return Err(ApiError::invalid_input("album id is required"));
        }
        let service = self.current().await.server.map(|server| server.service);
        Ok(match service {
            Some(config::MusicService::Spotify) => {
                Some(format!("https://open.spotify.com/album/{id}"))
            }
            Some(config::MusicService::YtMusic) => {
                Some(format!("https://music.youtube.com/browse/{id}"))
            }
            _ => None,
        })
    }

    pub async fn top_genre(&self) -> Result<Option<String>, ApiError> {
        let config = self.current().await;
        self.db
            .top_genre(&config.active_source)
            .await
            .map_err(Self::db_error)
    }

    pub async fn search(&self, query: &str) -> Result<api::SearchResults, ApiError> {
        let config = self.current().await;
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), &config));
        let (tracks, albums) = source.search(query).await.map_err(Self::source_error)?;
        self.library.register_transient(&tracks);
        Ok(api::SearchResults {
            tracks: tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
            albums: albums.iter().map(Self::album_info).collect(),
        })
    }

    pub async fn playlists(&self) -> Result<api::PlaylistCatalog, ApiError> {
        let config = self.current().await;
        let store = self
            .db
            .load_playlists(&config.active_source)
            .await
            .map_err(Self::db_error)?;
        Ok(Self::playlist_catalog(&store))
    }

    pub async fn playlist_tracks(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current().await;
        let playlist = self.playlist(&config, &request.id).await?;
        let total = playlist.tracks.len().min(u32::MAX as usize) as u32;
        let keys: Vec<String> = playlist
            .tracks
            .iter()
            .skip(request.page.offset as usize)
            .take(request.page.limit as usize)
            .cloned()
            .collect();
        let tracks = self
            .db
            .tracks_by_keys(&config.active_source, &keys)
            .await
            .map_err(Self::db_error)?;
        Ok(api::TrackPage {
            total,
            offset: request.page.offset,
            items: tracks
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub async fn refresh_playlist(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<api::TrackPage, ApiError> {
        if request.id.trim().is_empty() {
            return Err(ApiError::invalid_input("playlist id is required"));
        }
        let config = self.current().await;
        let source = self.source().await;
        if source.capabilities().sync {
            let mut cursor = None;
            let mut tracks = Vec::new();
            let mut seen = std::collections::HashSet::new();
            loop {
                let page = source
                    .fetch_playlist_entries_page(&request.id, cursor)
                    .await
                    .map_err(Self::source_error)?;
                let next = page.next;
                tracks.extend(page.tracks.into_iter().filter(|track| {
                    let key = track.id.key().into_owned();
                    !key.is_empty() && seen.insert(key)
                }));
                match next {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            for chunk in tracks.chunks(100) {
                self.db
                    .upsert_tracks(&config.active_source, chunk)
                    .await
                    .map_err(Self::db_error)?;
            }
            let keys: Vec<String> = tracks
                .iter()
                .map(|track| track.id.key().into_owned())
                .collect();
            self.db
                .set_playlist_tracks(&config.active_source, &request.id, &keys)
                .await
                .map_err(Self::db_error)?;
            self.library.invalidate(api::Table::Tracks);
            self.library.invalidate(api::Table::Playlists);
        }
        self.playlist_tracks(request).await
    }

    pub async fn create_playlist(&self, name: &str, keys: &[String]) -> Result<String, ApiError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ApiError::invalid_input("playlist name is required"));
        }
        let id = self
            .source()
            .await
            .create_playlist(name, keys)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Playlists);
        Ok(id)
    }

    pub async fn rename_playlist(&self, id: &str, name: &str) -> Result<(), ApiError> {
        let name = name.trim();
        if id.is_empty() || name.is_empty() {
            return Err(ApiError::invalid_input("playlist id and name are required"));
        }
        let config = self.current().await;
        let playlist = self.playlist(&config, id).await?;
        self.db
            .upsert_playlist_meta(
                &config.active_source,
                id,
                name,
                playlist.cover_path.as_deref().and_then(Path::to_str),
                playlist.image_tag.as_deref(),
            )
            .await
            .map_err(Self::db_error)?;
        self.library.invalidate(api::Table::Playlists);
        Ok(())
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<(), ApiError> {
        if id.is_empty() {
            return Err(ApiError::invalid_input("playlist id is required"));
        }
        self.source()
            .await
            .delete_playlist(id)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Playlists);
        self.library.invalidate(api::Table::Folders);
        Ok(())
    }

    pub async fn add_playlist_tracks(&self, id: &str, keys: &[String]) -> Result<(), ApiError> {
        self.source()
            .await
            .add_to_playlist(id, keys)
            .await
            .map(|_| ())
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Playlists);
        Ok(())
    }

    pub async fn remove_playlist_tracks(&self, id: &str, keys: &[String]) -> Result<(), ApiError> {
        let config = self.current().await;
        let playlist = self.playlist(&config, id).await?;
        let source = self.source().await;
        let mut remaining = playlist.tracks.clone();
        for key in keys {
            let Some(position) = remaining.iter().position(|item| item == key) else {
                continue;
            };
            let Some(track) = self
                .db
                .tracks_by_keys(&config.active_source, std::slice::from_ref(key))
                .await
                .map_err(Self::db_error)?
                .into_iter()
                .next()
            else {
                continue;
            };
            source
                .remove_from_playlist(id, &track, position)
                .await
                .map_err(Self::source_error)?;
            remaining.remove(position);
        }
        self.library.invalidate(api::Table::Playlists);
        Ok(())
    }

    pub async fn reorder_playlist_tracks(&self, id: &str, keys: &[String]) -> Result<(), ApiError> {
        let config = self.current().await;
        let playlist = self.playlist(&config, id).await?;
        let mut current_keys = playlist.tracks.clone();
        let mut requested_keys = keys.to_vec();
        current_keys.sort_unstable();
        requested_keys.sort_unstable();
        if current_keys != requested_keys {
            return Err(ApiError::invalid_input(
                "a reorder must contain every playlist track",
            ));
        }
        let Some(first) = keys
            .iter()
            .enumerate()
            .find(|(index, key)| playlist.tracks.get(*index) != Some(*key))
            .map(|(index, _)| index)
        else {
            return Ok(());
        };
        let last = keys
            .iter()
            .enumerate()
            .rfind(|(index, key)| playlist.tracks.get(*index) != Some(*key))
            .map(|(index, _)| index)
            .unwrap_or(first);
        let moved_later = playlist.tracks[first] == keys[last]
            && playlist.tracks[first + 1..=last] == keys[first..last];
        let moved_earlier = playlist.tracks[last] == keys[first]
            && playlist.tracks[first..last] == keys[first + 1..=last];
        let (moved_key, new_index) = if moved_later {
            (&keys[last], last)
        } else if moved_earlier {
            (&keys[first], first)
        } else {
            return Err(ApiError::invalid_input(
                "a reorder must describe one moved playlist track",
            ));
        };
        let track = self
            .db
            .tracks_by_keys(&config.active_source, std::slice::from_ref(moved_key))
            .await
            .map_err(Self::db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("moved track not found"))?;
        self.source()
            .await
            .reorder_playlist(id, keys, &track, new_index)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Playlists);
        Ok(())
    }

    pub async fn create_folder(&self, name: &str) -> Result<String, ApiError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ApiError::invalid_input("folder name is required"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.source()
            .await
            .create_folder(&id, name)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Folders);
        Ok(id)
    }

    pub async fn rename_folder(&self, id: &str, name: &str) -> Result<(), ApiError> {
        let name = name.trim();
        if id.is_empty() || name.is_empty() {
            return Err(ApiError::invalid_input("folder id and name are required"));
        }
        self.source()
            .await
            .rename_folder(id, name)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Folders);
        Ok(())
    }

    pub async fn delete_folder(&self, id: &str) -> Result<(), ApiError> {
        if id.is_empty() {
            return Err(ApiError::invalid_input("folder id is required"));
        }
        self.source()
            .await
            .delete_folder(id)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Folders);
        Ok(())
    }

    pub async fn move_playlist(&self, id: &str, folder_id: Option<&str>) -> Result<(), ApiError> {
        self.source()
            .await
            .set_playlist_folder(id, folder_id)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Folders);
        Ok(())
    }

    pub async fn sources(&self) -> Result<Vec<api::SourceInfo>, ApiError> {
        let config = self.current().await;
        let mut ids = Vec::with_capacity(config.local_sources.len() + config.servers.len() + 1);
        ids.push("local".to_string());
        ids.extend(config.local_sources.iter().map(|source| source.id.clone()));
        ids.extend(config.servers.iter().map(|server| server.id.clone()));
        let mut sources = Vec::with_capacity(ids.len());
        for id in ids {
            sources.push(self.source_info_for(&id).await?);
        }
        Ok(sources)
    }

    pub async fn switch_source(&self, id: &str) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["active_source", "server"])?;
        let previous_source = self.current().await.active_source;
        let (target, _) = self.source_for(id).await?;
        let source = target.active_source.clone();
        let source_changed = previous_source != source;
        let updated = self
            .config
            .mutate_state(move |config| match source {
                config::Source::Local | config::Source::LocalLibrary(_) => {
                    config.set_active_local_source(source)
                }
                config::Source::Server(_) => {
                    if let Some(server) = target.server {
                        config.set_active_server_snapshot(server);
                    }
                }
            })
            .await?;
        if source_changed {
            self.finish_source_change(updated, vec!["active_source".to_string()])
                .await?;
        } else {
            self.publish_config(updated, vec!["active_source".to_string()])
                .await;
        }
        self.source_info_for(id).await
    }

    pub async fn upsert_local_source(
        &self,
        draft: api::LocalSourceDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["local_sources"])?;
        let name = draft.name.trim();
        if name.is_empty() {
            return Err(ApiError::invalid_input("local source name is required"));
        }
        if draft.directories.is_empty()
            || draft.directories.iter().any(|path| path.trim().is_empty())
        {
            return Err(ApiError::invalid_input(
                "at least one local source directory is required",
            ));
        }
        let id = draft
            .id
            .unwrap_or_else(|| format!("local:{}", uuid::Uuid::new_v4()));
        if !id.starts_with("local:") {
            return Err(ApiError::invalid_input("invalid local source id"));
        }
        let saved = config::SavedLocalSource {
            id: id.clone(),
            name: name.to_string(),
            directories: draft.directories.into_iter().map(PathBuf::from).collect(),
        };
        let updated = self
            .config
            .mutate_state({
                let saved = saved.clone();
                move |config| {
                    if let Some(existing) = config
                        .local_sources
                        .iter_mut()
                        .find(|source| source.id == saved.id)
                    {
                        *existing = saved.clone();
                    } else {
                        config.local_sources.push(saved.clone());
                    }
                }
            })
            .await?;
        self.publish_config(updated, vec!["local_sources".to_string()])
            .await;
        self.library.invalidate(api::Table::Servers);
        self.source_info_for(&id).await
    }

    pub async fn delete_local_source(&self, id: &str) -> Result<(), ApiError> {
        self.config
            .ensure_unlocked(&["active_source", "local_sources"])?;
        if id == "local" {
            return Err(ApiError::invalid_input(
                "the default local source cannot be deleted",
            ));
        }
        let current = self.current().await;
        if !current.local_sources.iter().any(|source| source.id == id) {
            return Err(ApiError::not_found("local source not found"));
        }
        let was_active = current.active_source.local_library_id() == Some(id);
        let id = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| config.remove_local_source(&id))
            .await?;
        if was_active {
            self.finish_source_change(
                updated,
                vec!["local_sources".to_string(), "active_source".to_string()],
            )
            .await?;
        } else {
            self.publish_config(updated, vec!["local_sources".to_string()])
                .await;
            self.library.invalidate(api::Table::Servers);
        }
        Ok(())
    }

    pub async fn set_source_directories(
        &self,
        id: &str,
        directories: Vec<String>,
    ) -> Result<api::SourceInfo, ApiError> {
        if directories.iter().any(|path| path.trim().is_empty()) {
            return Err(ApiError::invalid_input("source directory is empty"));
        }
        let source = config::Source::from_column(id);
        let changed = match &source {
            config::Source::Local => "music_directory",
            config::Source::LocalLibrary(_) => "local_sources",
            config::Source::Server(_) => "server_folders",
        };
        self.config.ensure_unlocked(&[changed])?;
        let current = self.current().await;
        if let config::Source::LocalLibrary(local_id) = &source
            && !current
                .local_sources
                .iter()
                .any(|saved| saved.id == *local_id)
        {
            return Err(ApiError::not_found("local source not found"));
        }
        if let config::Source::Server(server_id) = &source
            && !current.servers.iter().any(|saved| saved.id == *server_id)
        {
            return Err(ApiError::not_found("server not found"));
        }
        let id_owned = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| match source {
                config::Source::Local => {
                    config.music_directory = directories.into_iter().map(PathBuf::from).collect();
                }
                config::Source::LocalLibrary(local_id) => {
                    if let Some(saved) = config
                        .local_sources
                        .iter_mut()
                        .find(|saved| saved.id == local_id)
                    {
                        saved.directories = directories.into_iter().map(PathBuf::from).collect();
                    }
                }
                config::Source::Server(server_id) => {
                    config.set_folders_for(&server_id, directories);
                }
            })
            .await?;
        self.publish_config(updated, vec![changed.to_string()])
            .await;
        self.library.invalidate(api::Table::Servers);
        self.source_info_for(&id_owned).await
    }

    pub async fn upsert_server(
        &self,
        draft: api::ServerDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        let service = crate::wire::music_service_from_api(draft.service)
            .ok_or_else(|| ApiError::invalid_input("unknown music service"))?;
        if draft.name.trim().is_empty() {
            return Err(ApiError::invalid_input("server name is required"));
        }
        if !service.uses_browser_signin() && !draft.url.starts_with("http") {
            return Err(ApiError::invalid_input("server URL must use HTTP or HTTPS"));
        }
        let id = draft.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Self::validate_server_id(&id)?;
        let browser = match draft.browser.as_deref() {
            Some(browser) => Some(
                config::Browser::from_id(browser)
                    .ok_or_else(|| ApiError::invalid_input("unknown browser"))?,
            ),
            None => None,
        };
        let saved = config::SavedServer {
            id: id.clone(),
            name: draft.name.trim().to_string(),
            url: draft.url.trim_end_matches('/').to_string(),
            service,
            yt_browser: browser,
            yt_anonymous: draft.anonymous,
            apple_music_storefront: draft.storefront.unwrap_or_else(|| "us".to_string()),
            apple_music_language: draft.language.unwrap_or_else(|| "en".to_string()),
        };
        let current = self.current().await;
        let backend_changed = current.active_source.server_id() == Some(id.as_str())
            && current
                .server
                .as_ref()
                .is_some_and(|server| server.service != saved.service || server.url != saved.url);
        let updated = self
            .config
            .mutate_state({
                let saved = saved.clone();
                move |config| {
                    if let Some(existing) = config
                        .servers
                        .iter_mut()
                        .find(|server| server.id == saved.id)
                    {
                        *existing = saved.clone();
                    } else {
                        config.servers.push(saved.clone());
                    }
                    if config.active_source.server_id() == Some(saved.id.as_str())
                        && let Some(server) = config.server.as_mut()
                    {
                        server.name.clone_from(&saved.name);
                        server.url.clone_from(&saved.url);
                        server.service = saved.service;
                        server.yt_browser = saved.yt_browser;
                        server.yt_anonymous = saved.yt_anonymous;
                        server
                            .apple_music_storefront
                            .clone_from(&saved.apple_music_storefront);
                        server
                            .apple_music_language
                            .clone_from(&saved.apple_music_language);
                    }
                }
            })
            .await?;
        self.publish_config(updated, vec!["servers".to_string()])
            .await;
        if backend_changed {
            self.reset_playback().await?;
        }
        self.library.invalidate(api::Table::Servers);
        self.source_info_for(&id).await
    }

    pub async fn delete_server(&self, id: &str) -> Result<(), ApiError> {
        self.config
            .ensure_unlocked(&["active_source", "server", "servers"])?;
        let current = self.current().await;
        let service = current
            .servers
            .iter()
            .find(|server| server.id == id)
            .map(|server| server.service);
        let was_active = current.active_source.server_id() == Some(id);
        let id_owned = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| {
                config.remove_saved_server(&id_owned);
                if was_active {
                    config.clear_active_server();
                }
            })
            .await?;
        self.publish_config(
            updated,
            vec!["servers".to_string(), "active_source".to_string()],
        )
        .await;
        if was_active {
            self.reset_playback().await?;
        }
        self.library.invalidate(api::Table::Servers);
        match service {
            Some(config::MusicService::YtMusic) => {
                let _ = server::ytmusic::isolated_profile::delete_profile(id);
            }
            Some(config::MusicService::SoundCloud) => {
                let _ = server::soundcloud::signin::delete_profile(id);
            }
            Some(config::MusicService::AppleMusic) => {
                let _ = server::applemusic::signin::delete_profile(id);
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn provision(
        &self,
        provision: api::CredentialProvision,
    ) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        if provision.secret.is_empty() {
            return Err(ApiError::invalid_input("credential is empty"));
        }
        let mut server = self
            .db
            .load_server(&provision.server_id)
            .await
            .map_err(Self::db_error)?
            .ok_or_else(|| ApiError::not_found("server not found"))?;
        let previous_user_id = server.user_id.clone();
        server.access_token = Some(provision.secret);
        server.user_id = provision.user_id;
        if let Some(browser) = provision.browser.as_deref() {
            server.yt_browser = Some(
                config::Browser::from_id(browser)
                    .ok_or_else(|| ApiError::invalid_input("unknown browser"))?,
            );
        }
        let active = self
            .current()
            .await
            .active_source
            .server_id()
            .is_some_and(|id| id == provision.server_id);
        let access_token = server.access_token.clone();
        let user_id = server.user_id.clone();
        let saved = config::SavedServer::from_music_server(&server);
        let live_server = server.clone();
        let updated = self
            .config
            .mutate_state(move |config| {
                if let Some(existing) = config.servers.iter_mut().find(|entry| entry.id == saved.id)
                {
                    *existing = saved;
                } else {
                    config.servers.push(saved);
                }
                if active {
                    config.server = Some(live_server);
                }
            })
            .await?;
        self.db
            .set_server_credentials(
                &provision.server_id,
                access_token.as_deref(),
                user_id.as_deref(),
            )
            .await
            .map_err(Self::db_error)?;
        if active {
            self.publish_config(updated, vec!["servers".to_string()])
                .await;
            if previous_user_id != server.user_id {
                self.reset_playback().await?;
            }
        } else {
            self.session
                .set_config(updated, vec!["servers".to_string()]);
        }
        self.library.invalidate(api::Table::Servers);
        self.source_info_for(&provision.server_id).await
    }

    pub async fn login_source(
        &self,
        request: api::SourceLoginRequest,
    ) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        if request.username.trim().is_empty() || request.password.is_empty() {
            return Err(ApiError::invalid_input(
                "source username and password are required",
            ));
        }
        let current = self.current().await;
        let server = self
            .db
            .load_server(&request.server_id)
            .await
            .map_err(Self::db_error)?
            .ok_or_else(|| ApiError::not_found("server not found"))?;
        let auth =
            server::provider::ProviderClient::new(server.service, server.url, current.device_id)
                .login(request.username.trim(), &request.password)
                .await
                .map_err(|error| ApiError::new(ErrorCode::Unauthorized, error))?;
        self.provision(api::CredentialProvision {
            server_id: request.server_id,
            secret: auth.access_token,
            user_id: Some(auth.user_id),
            browser: None,
        })
        .await
    }

    pub async fn clear_credentials(&self, id: &str) -> Result<(), ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        let mut server = self
            .db
            .load_server(id)
            .await
            .map_err(Self::db_error)?
            .ok_or_else(|| ApiError::not_found("server not found"))?;
        let had_credentials = server.access_token.is_some() || server.user_id.is_some();
        server.access_token = None;
        server.user_id = None;
        let active = self
            .current()
            .await
            .active_source
            .server_id()
            .is_some_and(|server_id| server_id == id);
        self.db
            .set_server_credentials(id, None, None)
            .await
            .map_err(Self::db_error)?;
        if active {
            let updated = self
                .config
                .mutate_state(move |config| config.server = Some(server))
                .await?;
            self.publish_config(updated, vec!["servers".to_string()])
                .await;
            if had_credentials {
                self.reset_playback().await?;
            }
        }
        self.library.invalidate(api::Table::Servers);
        Ok(())
    }

    pub async fn authenticate_source(&self, id: &str) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        #[cfg(target_os = "android")]
        {
            let _ = id;
            return Err(ApiError::unsupported(
                "daemon-owned browser authentication is unavailable on Android",
            ));
        }
        #[cfg(not(target_os = "android"))]
        {
            let server = self
                .db
                .load_server(id)
                .await
                .map_err(Self::db_error)?
                .ok_or_else(|| ApiError::not_found("server not found"))?;
            let browser = server.yt_browser.unwrap_or(config::Browser::Chrome);
            let (secret, user_id) = match server.service {
                config::MusicService::YtMusic => {
                    let secret = ensure_ytmusic_signed_in(server.access_token.clone(), browser, id)
                        .await
                        .map_err(ApiError::internal)?;
                    let user_id = server::ytmusic::derive_user_id(&secret)
                        .unwrap_or_else(|| "me".to_string());
                    (secret, user_id)
                }
                config::MusicService::SoundCloud => {
                    let secret = server::soundcloud::signin::launch_signin_and_extract(
                        browser,
                        id,
                        Duration::from_secs(300),
                    )
                    .await
                    .map_err(ApiError::internal)?;
                    let user_id = server::soundcloud::derive_user_id(&secret)
                        .await
                        .unwrap_or_else(|| "me".to_string());
                    (secret, user_id)
                }
                config::MusicService::AppleMusic => {
                    let secret = server::applemusic::signin::launch_signin_and_extract(
                        browser,
                        id,
                        Duration::from_secs(300),
                    )
                    .await
                    .map_err(ApiError::internal)?;
                    (secret, "me".to_string())
                }
                config::MusicService::Spotify => {
                    let auth = server::spotify::auth::launch_signin_and_extract(server.url)
                        .await
                        .map_err(ApiError::internal)?;
                    (
                        server::spotify::auth::pack_token(&auth.access_token, &auth.refresh_token),
                        auth.user_id,
                    )
                }
                _ => {
                    return Err(ApiError::unsupported(
                        "this source uses explicit credential provisioning",
                    ));
                }
            };
            self.provision(api::CredentialProvision {
                server_id: id.to_string(),
                secret,
                user_id: Some(user_id),
                browser: Some(browser.id().to_string()),
            })
            .await
        }
    }

    pub async fn browse_source(
        &self,
        id: &str,
        path: &str,
    ) -> Result<Vec<api::SourceFolderEntry>, ApiError> {
        let server = self
            .db
            .load_server(id)
            .await
            .map_err(Self::db_error)?
            .ok_or_else(|| ApiError::not_found("server not found"))?;
        if server.service != config::MusicService::Nextcloud {
            return Err(ApiError::unsupported(
                "folder browsing is only available for Nextcloud sources",
            ));
        }
        let user_id = server
            .user_id
            .as_deref()
            .ok_or_else(|| ApiError::new(ErrorCode::SourceAuthExpired, "missing user id"))?;
        let secret = server
            .access_token
            .as_deref()
            .ok_or_else(|| ApiError::new(ErrorCode::SourceAuthExpired, "missing password"))?;
        let paths = server::nextcloud::browse_folders(&server.url, user_id, secret, path)
            .await
            .map_err(ApiError::internal)?;
        Ok(paths
            .into_iter()
            .map(|path| api::SourceFolderEntry {
                name: server::nextcloud::folder_name(&path).to_string(),
                path,
            })
            .collect())
    }

    pub async fn integration_credentials(&self) -> Vec<api::IntegrationCredentialStatus> {
        let config = self.current().await;
        vec![
            api::IntegrationCredentialStatus {
                kind: api::IntegrationKind::ListenBrainz,
                configured: !config.musicbrainz_token.trim().is_empty(),
            },
            api::IntegrationCredentialStatus {
                kind: api::IntegrationKind::LastFm,
                configured: !config.lastfm_api_key.trim().is_empty()
                    && !config.lastfm_api_secret.trim().is_empty()
                    && !config.lastfm_session_key.trim().is_empty(),
            },
            api::IntegrationCredentialStatus {
                kind: api::IntegrationKind::LibreFm,
                configured: !config.librefm_session_key.trim().is_empty(),
            },
        ]
    }

    pub async fn provision_integration(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        let kind = provision.kind;
        self.config.ensure_unlocked(match kind {
            api::IntegrationKind::ListenBrainz => &["musicbrainz_token"],
            api::IntegrationKind::LastFm => {
                &["lastfm_api_key", "lastfm_api_secret", "lastfm_session_key"]
            }
            api::IntegrationKind::LibreFm => &[
                "librefm_api_key",
                "librefm_api_secret",
                "librefm_session_key",
            ],
            api::IntegrationKind::Unknown => &[],
        })?;
        let complete = match kind {
            api::IntegrationKind::ListenBrainz => provision
                .token
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            api::IntegrationKind::LastFm => [
                provision.api_key.as_deref(),
                provision.api_secret.as_deref(),
                provision.session_key.as_deref(),
            ]
            .into_iter()
            .all(|value| value.is_some_and(|value| !value.trim().is_empty())),
            api::IntegrationKind::LibreFm => provision
                .session_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            api::IntegrationKind::Unknown => false,
        };
        if !complete {
            return Err(ApiError::invalid_input(
                "the required credentials were not provided",
            ));
        }
        let changed = match kind {
            api::IntegrationKind::ListenBrainz => vec!["musicbrainz_token".to_string()],
            api::IntegrationKind::LastFm => vec![
                "lastfm_api_key".to_string(),
                "lastfm_api_secret".to_string(),
                "lastfm_session_key".to_string(),
            ],
            api::IntegrationKind::LibreFm => vec![
                "librefm_api_key".to_string(),
                "librefm_api_secret".to_string(),
                "librefm_session_key".to_string(),
            ],
            api::IntegrationKind::Unknown => {
                return Err(ApiError::invalid_input("unknown integration"));
            }
        };
        let updated = self
            .config
            .mutate_state(move |config| match kind {
                api::IntegrationKind::ListenBrainz => {
                    config.musicbrainz_token = provision.token.unwrap_or_default();
                }
                api::IntegrationKind::LastFm => {
                    config.lastfm_api_key = provision.api_key.unwrap_or_default();
                    config.lastfm_api_secret = provision.api_secret.unwrap_or_default();
                    config.lastfm_session_key = provision.session_key.unwrap_or_default();
                }
                api::IntegrationKind::LibreFm => {
                    config.librefm_api_key = provision
                        .api_key
                        .unwrap_or_else(|| scrobble::librefm::API_KEY.to_string());
                    config.librefm_api_secret = provision
                        .api_secret
                        .unwrap_or_else(|| scrobble::librefm::API_SECRET.to_string());
                    config.librefm_session_key = provision.session_key.unwrap_or_default();
                }
                api::IntegrationKind::Unknown => {}
            })
            .await?;
        self.session.set_config(updated, changed);
        let configured = self
            .integration_credentials()
            .await
            .into_iter()
            .find(|status| status.kind == kind)
            .is_some_and(|status| status.configured);
        Ok(api::IntegrationCredentialStatus { kind, configured })
    }

    pub async fn clear_integration(&self, kind: api::IntegrationKind) -> Result<(), ApiError> {
        self.config.ensure_unlocked(match kind {
            api::IntegrationKind::ListenBrainz => &["musicbrainz_token"],
            api::IntegrationKind::LastFm => {
                &["lastfm_api_key", "lastfm_api_secret", "lastfm_session_key"]
            }
            api::IntegrationKind::LibreFm => &[
                "librefm_api_key",
                "librefm_api_secret",
                "librefm_session_key",
            ],
            api::IntegrationKind::Unknown => &[],
        })?;
        let provision = api::IntegrationCredentialProvision {
            kind,
            ..Default::default()
        };
        let changed = match kind {
            api::IntegrationKind::ListenBrainz => vec!["musicbrainz_token".to_string()],
            api::IntegrationKind::LastFm => vec![
                "lastfm_api_key".to_string(),
                "lastfm_api_secret".to_string(),
                "lastfm_session_key".to_string(),
            ],
            api::IntegrationKind::LibreFm => vec![
                "librefm_api_key".to_string(),
                "librefm_api_secret".to_string(),
                "librefm_session_key".to_string(),
            ],
            api::IntegrationKind::Unknown => {
                return Err(ApiError::invalid_input("unknown integration"));
            }
        };
        let updated = self
            .config
            .mutate_state(move |config| match provision.kind {
                api::IntegrationKind::ListenBrainz => config.musicbrainz_token.clear(),
                api::IntegrationKind::LastFm => {
                    config.lastfm_api_key.clear();
                    config.lastfm_api_secret.clear();
                    config.lastfm_session_key.clear();
                }
                api::IntegrationKind::LibreFm => {
                    config.librefm_api_key.clear();
                    config.librefm_api_secret.clear();
                    config.librefm_session_key.clear();
                }
                api::IntegrationKind::Unknown => {}
            })
            .await?;
        self.session.set_config(updated, changed);
        Ok(())
    }

    #[cfg(target_os = "android")]
    pub async fn authenticate_integration(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        let _ = provision;
        Err(ApiError::unsupported(
            "daemon-owned browser authentication is unavailable on Android",
        ))
    }

    #[cfg(not(target_os = "android"))]
    pub async fn authenticate_integration(
        &self,
        mut provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        self.config.ensure_unlocked(match provision.kind {
            api::IntegrationKind::ListenBrainz => &["musicbrainz_token"],
            api::IntegrationKind::LastFm => {
                &["lastfm_api_key", "lastfm_api_secret", "lastfm_session_key"]
            }
            api::IntegrationKind::LibreFm => &[
                "librefm_api_key",
                "librefm_api_secret",
                "librefm_session_key",
            ],
            api::IntegrationKind::Unknown => &[],
        })?;
        let (api_key, api_secret, token, url) = match provision.kind {
            api::IntegrationKind::LastFm => {
                let key = provision
                    .api_key
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| ApiError::invalid_input("Last.fm API key is required"))?;
                let secret = provision
                    .api_secret
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| ApiError::invalid_input("Last.fm API secret is required"))?;
                let token = scrobble::lastfm::get_auth_token(&key)
                    .await
                    .map_err(|error| ApiError::internal(format!("Last.fm auth failed: {error}")))?;
                let url = scrobble::lastfm::auth_url(&key, &token);
                (key, secret, token, url)
            }
            api::IntegrationKind::LibreFm => {
                let key = scrobble::librefm::API_KEY.to_string();
                let secret = scrobble::librefm::API_SECRET.to_string();
                let token = scrobble::librefm::get_auth_token(&key)
                    .await
                    .map_err(|error| {
                        ApiError::internal(format!("Libre.fm auth failed: {error}"))
                    })?;
                let url = scrobble::librefm::auth_url(&key, &token);
                (key, secret, token, url)
            }
            api::IntegrationKind::ListenBrainz => {
                return self.provision_integration(provision).await;
            }
            api::IntegrationKind::Unknown => {
                return Err(ApiError::invalid_input("unknown integration"));
            }
        };
        webbrowser::open(&url)
            .map_err(|error| ApiError::internal(format!("could not open browser: {error}")))?;
        let mut session_key = None;
        for _ in 0..150 {
            let result = match provision.kind {
                api::IntegrationKind::LastFm => {
                    scrobble::lastfm::get_session_key(&api_key, &api_secret, &token).await
                }
                api::IntegrationKind::LibreFm => {
                    scrobble::librefm::get_session_key(&api_key, &api_secret, &token).await
                }
                _ => return Err(ApiError::invalid_input("unknown integration")),
            };
            if let Ok(key) = result {
                session_key = Some(key);
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        provision.api_key = Some(api_key);
        provision.api_secret = Some(api_secret);
        provision.session_key = Some(session_key.ok_or_else(|| {
            ApiError::new(ErrorCode::SourceAuthExpired, "authorization timed out")
        })?);
        self.provision_integration(provision).await
    }

    pub async fn validate_source(&self, id: &str) -> Result<api::SourceState, ApiError> {
        let (_, source) = self.source_for(id).await?;
        Ok(match source.validate().await {
            AuthOutcome::Valid => api::SourceState::Online,
            AuthOutcome::Expired => api::SourceState::AuthExpired,
            AuthOutcome::Unreachable => api::SourceState::Offline,
        })
    }

    pub async fn external_access(&self, kind: &str) -> Result<api::ExternalAccess, ApiError> {
        if kind != "spotify" {
            return Err(ApiError::unsupported("unsupported external playback kind"));
        }
        let config = self.current().await;
        let server = config
            .server
            .filter(|server| server.service == config::MusicService::Spotify)
            .ok_or_else(|| ApiError::not_found("Spotify is not the active source"))?;
        let packed = server.access_token.ok_or_else(|| {
            ApiError::new(ErrorCode::SourceAuthExpired, "Spotify is not signed in")
        })?;
        let client_id = server.url.clone();
        let (access_token, refresh_token) = server::spotify::auth::unpack_token(&packed);
        if refresh_token.is_empty() {
            return Ok(api::ExternalAccess {
                kind: kind.to_string(),
                access_token,
                client_id: Some(client_id),
            });
        }
        let refreshed = server::spotify::auth::refresh_packed(&packed, client_id.clone())
            .await
            .map_err(|error| {
                ApiError::new(
                    ErrorCode::SourceAuthExpired,
                    format!("Spotify credential refresh failed: {error}"),
                )
            })?;
        let server_id = server
            .id
            .ok_or_else(|| ApiError::internal("active Spotify server has no id"))?;
        self.db
            .set_server_credentials(&server_id, Some(&refreshed), server.user_id.as_deref())
            .await
            .map_err(Self::db_error)?;
        let expected = packed;
        let refreshed_for_config = refreshed.clone();
        let updated = self
            .config
            .mutate_state(move |config| {
                if config.active_source.server_id() == Some(server_id.as_str())
                    && let Some(server) = config.server.as_mut()
                    && server.access_token.as_deref() == Some(expected.as_str())
                {
                    server.access_token = Some(refreshed_for_config);
                }
            })
            .await?;
        self.session
            .set_config(updated, vec!["servers".to_string()]);
        let (access_token, _) = server::spotify::auth::unpack_token(&refreshed);
        Ok(api::ExternalAccess {
            kind: kind.to_string(),
            access_token,
            client_id: Some(client_id),
        })
    }

    pub fn set_external(&self, external: Option<api::ExternalPlayback>) {
        self.session.set_external(external);
    }

    pub async fn claim_external(
        &self,
        external: api::ExternalPlayback,
    ) -> Result<api::ExternalPlaybackLease, ApiError> {
        if external.kind.trim().is_empty() {
            return Err(ApiError::invalid_input(
                "external playback kind is required",
            ));
        }
        if external.kind != "spotify" {
            return Err(ApiError::unsupported("unsupported external playback kind"));
        }
        if !self.current().await.server.is_some_and(|server| {
            server.service == config::MusicService::Spotify
                && server
                    .access_token
                    .as_deref()
                    .is_some_and(|token| !token.is_empty())
        }) {
            return Err(ApiError::new(
                ErrorCode::SourceAuthExpired,
                "Spotify must be the authenticated active source",
            ));
        }
        let mut lease = self.external_lease.lock().await;
        if lease
            .as_ref()
            .is_some_and(|current| current.expires_at > Instant::now())
        {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback is owned by another frontend",
            ));
        }
        if lease.take().is_some() {
            self.session.set_external(None);
        }
        let lease_id = uuid::Uuid::new_v4().to_string();
        *lease = Some(ExternalLease {
            id: lease_id.clone(),
            expires_at: Instant::now() + EXTERNAL_LEASE_TTL,
        });
        drop(lease);
        self.session.set_external(Some(external));
        self.spawn_external_expiry(lease_id.clone());
        Ok(api::ExternalPlaybackLease {
            lease_id,
            expires_in_ms: EXTERNAL_LEASE_TTL.as_millis() as u64,
        })
    }

    fn spawn_external_expiry(&self, lease_id: String) {
        let external_lease = self.external_lease.clone();
        let session = self.session.clone();
        tokio::spawn(async move {
            loop {
                let deadline = {
                    let lease = external_lease.lock().await;
                    lease
                        .as_ref()
                        .filter(|current| current.id == lease_id)
                        .map(|current| current.expires_at)
                };
                let Some(deadline) = deadline else {
                    return;
                };
                tokio::time::sleep_until(deadline.into()).await;
                let mut lease = external_lease.lock().await;
                let expired = lease.as_ref().is_some_and(|current| {
                    current.id == lease_id && current.expires_at <= Instant::now()
                });
                if expired {
                    *lease = None;
                    drop(lease);
                    session.set_external(None);
                    tracing::info!("external playback lease expired");
                    return;
                }
            }
        });
    }

    pub async fn report_external(
        &self,
        report: api::ExternalPlaybackReport,
    ) -> Result<(), ApiError> {
        let mut lease = self.external_lease.lock().await;
        let Some(current) = lease.as_mut() else {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback is not claimed",
            ));
        };
        if current.expires_at <= Instant::now() {
            *lease = None;
            drop(lease);
            self.session.set_external(None);
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback lease expired",
            ));
        }
        if current.id != report.lease_id {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback lease does not match",
            ));
        }
        current.expires_at = Instant::now() + EXTERNAL_LEASE_TTL;
        drop(lease);
        let track = report
            .track
            .as_ref()
            .map(|value| -> Result<reader::Track, ApiError> {
                let decoded = LibraryService::track_from_info(value)?;
                Ok(self
                    .library
                    .transient_track_for_info(value)
                    .unwrap_or(decoded))
            })
            .transpose()?;
        if track
            .as_ref()
            .is_some_and(|track| track.id.service() != Some(config::MusicService::Spotify))
        {
            return Err(ApiError::invalid_input(
                "external Spotify playback requires a Spotify track",
            ));
        }
        if let Some(track) = track.as_ref() {
            self.library.register_transient(std::slice::from_ref(track));
        }
        self.session
            .report_external(
                track,
                report.position_ms,
                report.playing,
                report.completed,
                report.device,
            )
            .await
    }

    pub async fn release_external(&self, lease_id: &str) -> Result<(), ApiError> {
        let mut lease = self.external_lease.lock().await;
        let Some(current) = lease.as_ref() else {
            return Ok(());
        };
        if current.id != lease_id {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback lease does not match",
            ));
        }
        *lease = None;
        drop(lease);
        self.session.set_external(None);
        Ok(())
    }

    fn catalog_page(
        &self,
        home: server::ytmusic::discover::DiscoverHome,
        config: &config::AppConfig,
    ) -> api::CatalogPage {
        use server::ytmusic::discover::DiscoverItem;
        let transient: Vec<reader::Track> = home
            .shelves
            .iter()
            .flat_map(|shelf| shelf.items.iter())
            .filter_map(|item| match item {
                DiscoverItem::Song(track) => Some((**track).clone()),
                _ => None,
            })
            .collect();
        self.library.register_transient(&transient);
        api::CatalogPage {
            shelves: home
                .shelves
                .into_iter()
                .map(|shelf| api::CatalogShelf {
                    title: shelf.title,
                    strapline: shelf.strapline,
                    more_ref: shelf.more_browse_id,
                    list: shelf.is_song_list,
                    items: shelf
                        .items
                        .into_iter()
                        .map(|item| match item {
                            DiscoverItem::Song(track) => api::CatalogItem {
                                kind: api::CatalogItemKind::Track,
                                id: track.id.key().into_owned(),
                                title: track.title.clone(),
                                subtitle: Some(track.artist.clone()),
                                artwork: track.cover.clone(),
                                track: Some(crate::wire::track_info(&track, config)),
                            },
                            DiscoverItem::Playlist {
                                playlist_id,
                                title,
                                subtitle,
                                thumbnail,
                            } => api::CatalogItem {
                                kind: api::CatalogItemKind::Playlist,
                                id: playlist_id,
                                title,
                                subtitle: Some(subtitle),
                                artwork: thumbnail,
                                track: None,
                            },
                            DiscoverItem::Album {
                                browse_id,
                                title,
                                subtitle,
                                thumbnail,
                            } => api::CatalogItem {
                                kind: api::CatalogItemKind::Album,
                                id: browse_id,
                                title,
                                subtitle: Some(subtitle),
                                artwork: thumbnail,
                                track: None,
                            },
                            DiscoverItem::Artist {
                                channel_id,
                                name,
                                thumbnail,
                            } => api::CatalogItem {
                                kind: api::CatalogItemKind::Artist,
                                id: channel_id,
                                title: name,
                                subtitle: None,
                                artwork: thumbnail,
                                track: None,
                            },
                            DiscoverItem::Mood {
                                browse_id,
                                title,
                                thumbnail,
                            } => api::CatalogItem {
                                kind: api::CatalogItemKind::Mood,
                                id: browse_id,
                                title,
                                subtitle: None,
                                artwork: thumbnail,
                                track: None,
                            },
                        })
                        .collect(),
                })
                .collect(),
            continuation: home.continuation,
        }
    }

    pub async fn catalog(&self, continuation: Option<&str>) -> Result<api::CatalogPage, ApiError> {
        let config = self.current().await;
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), &config));
        let home = match continuation {
            Some(token) => source.discover_continuation(token).await,
            None => source.discover_home().await,
        }
        .map_err(Self::source_error)?;
        Ok(self.catalog_page(home, &config))
    }

    pub async fn catalog_detail(
        &self,
        request: api::CatalogDetailRequest,
    ) -> Result<api::CatalogDetail, ApiError> {
        if request.id.trim().is_empty() {
            return Err(ApiError::invalid_input("catalog id is required"));
        }
        let config = self.current().await;
        let source = self.source().await;
        match request.kind {
            api::CatalogItemKind::Album => {
                let album = match source
                    .fetch_album_by_ref(&request.id)
                    .await
                    .map_err(Self::source_error)?
                {
                    Some(album) => album,
                    None => source
                        .fetch_album(&request.id)
                        .await
                        .map_err(Self::source_error)?,
                };
                self.library.register_transient(&album.tracks);
                Ok(api::CatalogDetail {
                    kind: api::CatalogItemKind::Album,
                    id: album.browse_id,
                    title: album.title,
                    subtitle: album.artist,
                    description: None,
                    artwork: album.thumbnail,
                    playback_id: album.audio_playlist_id,
                    year: album.year,
                    tracks: album
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, &config))
                        .collect(),
                    shelves: Vec::new(),
                    continuation: None,
                })
            }
            api::CatalogItemKind::Playlist => {
                let page = source
                    .fetch_playlist_entries_page(&request.id, request.continuation)
                    .await
                    .map_err(Self::source_error)?;
                self.library.register_transient(&page.tracks);
                Ok(api::CatalogDetail {
                    kind: api::CatalogItemKind::Playlist,
                    id: request.id.clone(),
                    title: request.id,
                    tracks: page
                        .tracks
                        .iter()
                        .map(|track| crate::wire::track_info(track, &config))
                        .collect(),
                    continuation: page.next,
                    ..Default::default()
                })
            }
            api::CatalogItemKind::Artist => {
                let channel_id = if request.id.starts_with("UC") {
                    request.id
                } else {
                    source
                        .resolve_artist_channel_id(request.id.trim())
                        .await
                        .map_err(Self::source_error)?
                        .ok_or_else(|| ApiError::not_found("catalog artist not found"))?
                };
                let artist = source
                    .fetch_artist(&channel_id)
                    .await
                    .map_err(Self::source_error)?;
                let page = self.catalog_page(
                    server::ytmusic::discover::DiscoverHome {
                        shelves: artist.sections,
                        continuation: None,
                    },
                    &config,
                );
                Ok(api::CatalogDetail {
                    kind: api::CatalogItemKind::Artist,
                    id: artist.channel_id,
                    title: artist.name,
                    subtitle: artist.subscribers,
                    description: artist.description,
                    artwork: artist.banner_thumbnail,
                    playback_id: artist.shuffle_playlist_id,
                    tracks: Vec::new(),
                    shelves: page.shelves,
                    continuation: page.continuation,
                    year: None,
                })
            }
            api::CatalogItemKind::Track
            | api::CatalogItemKind::Mood
            | api::CatalogItemKind::Unknown => {
                Err(ApiError::unsupported("catalog detail kind is unsupported"))
            }
        }
    }

    fn station_info(
        manifest: &radio::manifest::StationManifest,
        pinned: bool,
    ) -> api::RadioStationInfo {
        api::RadioStationInfo {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            icon: manifest.icon.clone(),
            artwork: match manifest.metadata.as_ref() {
                Some(radio::manifest::MetadataSourceDef::Static(metadata)) => {
                    metadata.cover_url.clone()
                }
                _ => None,
            },
            tags: manifest.tags.clone(),
            streams: manifest
                .streams
                .iter()
                .map(|stream| api::RadioStreamInfo {
                    id: stream.id.clone(),
                    name: stream.name.clone(),
                    url: stream.url.clone(),
                    icon: stream.icon.clone(),
                })
                .collect(),
            pinned,
        }
    }

    pub async fn reload_radio(&self) -> Result<(), ApiError> {
        let config = self.current().await;
        let mut registry = radio::registry::StationRegistry::new();
        for entry in config.radio_registries.iter().filter(|entry| entry.enabled) {
            if let Err(error) = registry.import_registry(&entry.url).await {
                tracing::warn!(url = %entry.url, %error, "radio registry import failed");
            }
        }
        for json in &config.pinned_stations {
            match serde_json::from_str(json) {
                Ok(manifest) => registry.pin_manifest(manifest),
                Err(error) => tracing::warn!(%error, "pinned radio station is invalid"),
            }
        }
        *self.registry.write().await = registry.clone();
        self.library.set_station_registry(Arc::new(registry));
        Ok(())
    }

    pub async fn adopt_radio_registry(&self, registry: radio::registry::StationRegistry) {
        *self.registry.write().await = registry.clone();
        self.library.set_station_registry(Arc::new(registry));
    }

    pub async fn radio_stations(&self) -> Vec<api::RadioStationInfo> {
        let registry = self.registry.read().await;
        registry
            .all_stations()
            .into_iter()
            .map(|station| {
                let pinned = registry.is_registry_station(&station.id);
                Self::station_info(station, pinned)
            })
            .collect()
    }

    pub async fn track_radio(&self, key: &str) -> Result<Vec<api::TrackInfo>, ApiError> {
        if key.trim().is_empty() {
            return Err(ApiError::invalid_input("radio seed key is required"));
        }
        let config = self.current().await;
        let mut tracks = self
            .source()
            .await
            .start_radio(key)
            .await
            .map_err(Self::source_error)?;
        if !tracks.is_empty() {
            let seed = if tracks.iter().any(|track| track.id.key().as_ref() == key) {
                None
            } else {
                Some(
                    self.db
                        .tracks_by_keys(&config.active_source, &[key.to_string()])
                        .await
                        .map_err(Self::db_error)?
                        .into_iter()
                        .next()
                        .or_else(|| self.library.transient_track(key))
                        .ok_or_else(|| ApiError::not_found("radio seed track not found"))?,
                )
            };
            tracks = Self::pin_radio_seed(key, seed, tracks);
        }
        self.library.register_transient(&tracks);
        Ok(tracks
            .iter()
            .map(|track| crate::wire::track_info(track, &config))
            .collect())
    }

    fn pin_radio_seed(
        key: &str,
        fallback: Option<reader::Track>,
        tracks: Vec<reader::Track>,
    ) -> Vec<reader::Track> {
        if tracks.is_empty() {
            return tracks;
        }
        let (seed_rows, mut rest): (Vec<_>, Vec<_>) = tracks
            .into_iter()
            .partition(|track| track.id.key().as_ref() == key);
        if let Some(seed) = seed_rows.into_iter().next().or(fallback) {
            rest.insert(0, seed);
        }
        rest
    }

    pub async fn playlist_radio(&self, id: &str) -> Result<Vec<api::TrackInfo>, ApiError> {
        if id.trim().is_empty() {
            return Err(ApiError::invalid_input("playlist radio seed is required"));
        }
        let config = self.current().await;
        let tracks = self
            .source()
            .await
            .start_playlist_radio(id)
            .await
            .map_err(Self::source_error)?;
        self.library.register_transient(&tracks);
        Ok(tracks
            .iter()
            .map(|track| crate::wire::track_info(track, &config))
            .collect())
    }

    pub async fn search_radio(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        let stations = if query.trim().is_empty() {
            radio::browser::top_stations(limit).await
        } else {
            radio::browser::search(query, limit).await
        }
        .map_err(|error| ApiError::new(ErrorCode::SourceUnreachable, error.to_string()))?;
        let mut registry = self.registry.write().await;
        let mut result = Vec::with_capacity(stations.len());
        for station in stations {
            let manifest = radio::browser::to_manifest(&station);
            let pinned = registry.is_registry_station(&manifest.id);
            result.push(Self::station_info(&manifest, pinned));
            registry.insert_manifest(manifest);
        }
        let snapshot = Arc::new(registry.clone());
        drop(registry);
        self.library.set_station_registry(snapshot);
        Ok(result)
    }

    pub async fn radio_registries(&self) -> Vec<api::RadioRegistryInfo> {
        self.current()
            .await
            .radio_registries
            .iter()
            .map(|entry| api::RadioRegistryInfo {
                url: entry.url.clone(),
                enabled: entry.enabled,
                built_in: entry.is_default,
            })
            .collect()
    }

    async fn mutate_radio_config(
        &self,
        key: &'static str,
        mutate: impl FnOnce(&mut config::AppConfig),
    ) -> Result<(), ApiError> {
        self.config.ensure_unlocked(&[key])?;
        let updated = self.config.mutate_state(mutate).await?;
        self.session.set_config(updated, vec![key.to_string()]);
        self.reload_radio().await
    }

    pub async fn add_radio_registry(&self, url: &str) -> Result<(), ApiError> {
        let mut probe = radio::registry::StationRegistry::new();
        probe
            .import_registry(url)
            .await
            .map_err(|error| ApiError::invalid_input(error.to_string()))?;
        let url = url.to_string();
        self.mutate_radio_config("radio_registries", move |config| {
            if !config.radio_registries.iter().any(|entry| entry.url == url) {
                config.radio_registries.push(config::RegistryEntry {
                    url,
                    enabled: true,
                    is_default: false,
                });
            }
        })
        .await
    }

    pub async fn remove_radio_registry(&self, url: &str) -> Result<(), ApiError> {
        let url = url.to_string();
        self.mutate_radio_config("radio_registries", move |config| {
            config
                .radio_registries
                .retain(|entry| entry.url != url || entry.is_default);
        })
        .await
    }

    pub async fn set_radio_registry_enabled(
        &self,
        url: &str,
        enabled: bool,
    ) -> Result<(), ApiError> {
        let url = url.to_string();
        self.mutate_radio_config("radio_registries", move |config| {
            if let Some(entry) = config
                .radio_registries
                .iter_mut()
                .find(|entry| entry.url == url)
            {
                entry.enabled = enabled;
            }
        })
        .await
    }

    pub async fn pin_station(
        &self,
        station: api::RadioStationInfo,
        pinned: bool,
    ) -> Result<(), ApiError> {
        let manifest = {
            let registry = self.registry.read().await;
            registry
                .get(&station.id)
                .cloned()
                .unwrap_or_else(|| radio::manifest::StationManifest {
                    schema_version: "1.0".to_string(),
                    id: station.id.clone(),
                    name: station.name.clone(),
                    description: station.description.clone(),
                    icon: station.icon.clone(),
                    tags: station.tags.clone(),
                    streams: station
                        .streams
                        .iter()
                        .map(|stream| radio::manifest::StreamDef {
                            id: stream.id.clone(),
                            name: stream.name.clone(),
                            url: stream.url.clone(),
                            codec: None,
                            bitrate: None,
                            icon: stream.icon.clone(),
                        })
                        .collect(),
                    metadata: Some(radio::manifest::MetadataSourceDef::Static(
                        radio::manifest::StaticSourceDef {
                            title: station.name.clone(),
                            artist: "Live Radio".to_string(),
                            cover_url: station.artwork.clone(),
                            stream_overrides: std::collections::HashMap::new(),
                        },
                    )),
                })
        };
        manifest
            .validate()
            .map_err(|error| ApiError::invalid_input(error.to_string()))?;
        let id = manifest.id.clone();
        let json = serde_json::to_string(&manifest)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        self.mutate_radio_config("pinned_stations", move |config| {
            config.pinned_stations.retain(|existing| {
                serde_json::from_str::<radio::manifest::StationManifest>(existing)
                    .map(|station| station.id != id)
                    .unwrap_or(true)
            });
            if pinned {
                config.pinned_stations.push(json);
            }
        })
        .await
    }

    pub async fn update_track_metadata(
        &self,
        patch: api::TrackMetadataPatch,
    ) -> Result<api::TrackInfo, ApiError> {
        let config = self.current().await;
        let mut track = self
            .db
            .tracks_by_keys(&config.active_source, std::slice::from_ref(&patch.key))
            .await
            .map_err(Self::db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("track not found"))?;
        let path = track
            .id
            .local_path()
            .map(Path::to_owned)
            .ok_or_else(|| ApiError::unsupported("only local track tags are editable"))?;
        let title = patch.title.unwrap_or_else(|| track.title.clone());
        let artist = patch.artist.unwrap_or_else(|| track.artist.clone());
        let album = patch.album.unwrap_or_else(|| track.album.clone());
        let track_number = if patch.clear_track_number {
            None
        } else {
            patch.track_number.or(track.track_number)
        };
        let disc_number = if patch.clear_disc_number {
            None
        } else {
            patch.disc_number.or(track.disc_number)
        };
        let edits = reader::TrackEdits {
            title: title.clone(),
            artist: artist.clone(),
            album: album.clone(),
            track_number,
            disc_number,
            cover: reader::CoverChange::Keep,
        };
        tokio::task::spawn_blocking(move || reader::write_tags(&path, &edits))
            .await
            .map_err(|error| ApiError::internal(format!("tag writer task failed: {error}")))?
            .map_err(ApiError::internal)?;
        track.title = title.trim().to_string();
        track.artist = artist.trim().to_string();
        track.artists = artist
            .split([';', ','])
            .map(str::trim)
            .filter(|artist| !artist.is_empty())
            .map(str::to_string)
            .collect();
        track.album = album.trim().to_string();
        track.album_id = reader::metadata::make_album_id(&track.album, &track.artist);
        track.track_number = track_number;
        track.disc_number = disc_number;
        self.db
            .upsert_tracks(&config.active_source, std::slice::from_ref(&track))
            .await
            .map_err(Self::db_error)?;
        self.library.invalidate(api::Table::Tracks);
        Ok(crate::wire::track_info(&track, &config))
    }

    fn allowed_local_path(config: &config::AppConfig, path: &Path) -> bool {
        let Ok(path) = path.canonicalize() else {
            return false;
        };
        config
            .music_directory
            .iter()
            .chain(
                config
                    .local_sources
                    .iter()
                    .flat_map(|source| source.directories.iter()),
            )
            .filter_map(|root| root.canonicalize().ok())
            .any(|root| path.starts_with(root))
    }

    pub async fn delete_tracks(&self, keys: &[String], from_disk: bool) -> Result<(), ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .tracks_by_keys(&config.active_source, keys)
            .await
            .map_err(Self::db_error)?;
        if from_disk {
            let mut paths = Vec::with_capacity(tracks.len());
            for track in &tracks {
                let path = track.id.local_path().ok_or_else(|| {
                    ApiError::unsupported("server tracks cannot be deleted from local storage")
                })?;
                if !Self::allowed_local_path(&config, path) {
                    return Err(ApiError::invalid_input(
                        "track path is outside configured library roots",
                    ));
                }
                paths.push(path.to_owned());
            }
            tokio::task::spawn_blocking(move || {
                for path in paths {
                    if let Err(error) = std::fs::remove_file(path)
                        && error.kind() != std::io::ErrorKind::NotFound
                    {
                        return Err(error);
                    }
                }
                Ok::<(), std::io::Error>(())
            })
            .await
            .map_err(|error| ApiError::internal(format!("delete task failed: {error}")))?
            .map_err(|error| ApiError::internal(format!("file delete failed: {error}")))?;
        }
        self.source()
            .await
            .delete_tracks(keys)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Tracks);
        Ok(())
    }

    pub async fn delete_album(&self, id: &str, from_disk: bool) -> Result<(), ApiError> {
        let config = self.current().await;
        let tracks = self
            .db
            .album_tracks(&config.active_source, id)
            .await
            .map_err(Self::db_error)?;
        let keys: Vec<String> = tracks
            .iter()
            .map(|track| track.id.key().into_owned())
            .collect();
        if from_disk {
            self.delete_tracks(&keys, true).await?;
        }
        self.source()
            .await
            .delete_album(id)
            .await
            .map_err(Self::source_error)?;
        self.library.invalidate(api::Table::Albums);
        Ok(())
    }

    pub async fn upload_artwork(&self, upload: api::ArtworkUpload) -> Result<(), ApiError> {
        const MAX_BYTES: usize = 32 * 1024 * 1024;
        if upload.data.is_empty() || upload.data.len() > MAX_BYTES {
            return Err(ApiError::invalid_input(
                "artwork must be between 1 byte and 32 MiB",
            ));
        }
        let mut image = image::ImageReader::new(std::io::Cursor::new(&upload.data))
            .with_guessed_format()
            .map_err(|error| ApiError::invalid_input(format!("invalid image: {error}")))?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        image.limits(limits);
        image
            .decode()
            .map_err(|error| ApiError::invalid_input(format!("invalid image: {error}")))?;
        let target = upload
            .target
            .ok_or_else(|| ApiError::invalid_input("artwork target is required"))?;
        if let api::ArtworkTarget::Track { key } = target {
            return self
                .update_track_artwork(&key, reader::CoverChange::Set(upload.data))
                .await;
        }
        let config = self.current().await;
        let previous = match &target {
            api::ArtworkTarget::Track { .. } => None,
            api::ArtworkTarget::Album { id } => {
                self.db
                    .album(&config.active_source, id)
                    .await
                    .map_err(Self::db_error)?
                    .ok_or_else(|| ApiError::not_found("album not found"))?
                    .cover_path
            }
            api::ArtworkTarget::Artist { name } => self
                .db
                .artist_images()
                .await
                .map_err(Self::db_error)?
                .0
                .get(&utils::artist::normalize_artist_key(name))
                .cloned(),
            api::ArtworkTarget::Playlist { id } => self.playlist(&config, id).await?.cover_path,
        };
        let extension = match upload.content_type.as_str() {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/webp" => "webp",
            _ => return Err(ApiError::invalid_input("unsupported artwork content type")),
        };
        let path = self
            .uploads
            .join(format!("{}.{extension}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&self.uploads)
            .map_err(|error| ApiError::internal(format!("artwork directory failed: {error}")))?;
        tokio::fs::write(&path, &upload.data)
            .await
            .map_err(|error| ApiError::internal(format!("artwork write failed: {error}")))?;
        let path_string = path.to_string_lossy().into_owned();
        let result: Result<(), ApiError> = match target {
            api::ArtworkTarget::Track { .. } => unreachable!(),
            api::ArtworkTarget::Album { id } => {
                async {
                    self.source()
                        .await
                        .update_album_cover(&id, Some(&path_string), true)
                        .await
                        .map_err(Self::source_error)?;
                    self.library.invalidate(api::Table::Albums);
                    Ok(())
                }
                .await
            }
            api::ArtworkTarget::Artist { name } => {
                async {
                    self.source()
                        .await
                        .set_artist_image(
                            &utils::artist::normalize_artist_key(&name),
                            "custom",
                            Some(&path_string),
                        )
                        .await
                        .map_err(Self::source_error)?;
                    self.library.invalidate(api::Table::Tracks);
                    Ok(())
                }
                .await
            }
            api::ArtworkTarget::Playlist { id } => {
                async {
                    let config = self.current().await;
                    let playlist = self.playlist(&config, &id).await?;
                    self.source()
                        .await
                        .set_playlist_cover(
                            &id,
                            &playlist.name,
                            &path,
                            playlist.image_tag.as_deref(),
                        )
                        .await
                        .map_err(Self::source_error)?;
                    self.library.invalidate(api::Table::Playlists);
                    Ok(())
                }
                .await
            }
        };
        if result.is_err() {
            let _ = tokio::fs::remove_file(path).await;
        } else if let Some(previous) = previous
            && previous.starts_with(&self.uploads)
            && previous != path
        {
            let _ = tokio::fs::remove_file(previous).await;
        }
        result
    }

    pub async fn remove_artwork(&self, target: api::ArtworkTarget) -> Result<(), ApiError> {
        let config = self.current().await;
        let previous = match target {
            api::ArtworkTarget::Track { key } => {
                self.update_track_artwork(&key, reader::CoverChange::Remove)
                    .await?;
                None
            }
            api::ArtworkTarget::Album { id } => {
                let album = self
                    .db
                    .album(&config.active_source, &id)
                    .await
                    .map_err(Self::db_error)?
                    .ok_or_else(|| ApiError::not_found("album not found"))?;
                self.source()
                    .await
                    .update_album_cover(&id, None, false)
                    .await
                    .map_err(Self::source_error)?;
                self.library.invalidate(api::Table::Albums);
                album.cover_path
            }
            api::ArtworkTarget::Artist { name } => {
                let normalized = utils::artist::normalize_artist_key(&name);
                let previous = self
                    .db
                    .artist_images()
                    .await
                    .map_err(Self::db_error)?
                    .0
                    .get(&normalized)
                    .cloned();
                self.source()
                    .await
                    .set_artist_image(&normalized, "custom", None)
                    .await
                    .map_err(Self::source_error)?;
                self.library.invalidate(api::Table::Tracks);
                previous
            }
            api::ArtworkTarget::Playlist { id } => {
                let playlist = self.playlist(&config, &id).await?;
                let previous = playlist.cover_path.clone();
                self.db
                    .upsert_playlist_meta(
                        &config.active_source,
                        &id,
                        &playlist.name,
                        None,
                        playlist.image_tag.as_deref(),
                    )
                    .await
                    .map_err(Self::db_error)?;
                self.library.invalidate(api::Table::Playlists);
                previous
            }
        };
        if let Some(path) = previous
            && path.starts_with(&self.uploads)
        {
            let _ = tokio::fs::remove_file(path).await;
        }
        Ok(())
    }

    async fn update_track_artwork(
        &self,
        key: &str,
        cover: reader::CoverChange,
    ) -> Result<(), ApiError> {
        let config = self.current().await;
        let track = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(Self::db_error)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("track not found"))?;
        let path = track
            .id
            .local_path()
            .map(Path::to_owned)
            .ok_or_else(|| ApiError::unsupported("only local track artwork is editable"))?;
        if !Self::allowed_local_path(&config, &path) {
            return Err(ApiError::invalid_input(
                "track path is outside configured library roots",
            ));
        }
        let edits = reader::TrackEdits {
            title: track.title,
            artist: track.artist,
            album: track.album,
            track_number: track.track_number,
            disc_number: track.disc_number,
            cover,
        };
        tokio::task::spawn_blocking(move || reader::write_tags(&path, &edits))
            .await
            .map_err(|error| ApiError::internal(format!("tag writer task failed: {error}")))?
            .map_err(ApiError::internal)?;
        self.library.invalidate(api::Table::Tracks);
        Ok(())
    }
}

/// Accept `seed` cookies if they still validate, else try one keepalive
/// rotation before giving up on them.
#[cfg(not(target_os = "android"))]
async fn try_resume_ytmusic(seed: Option<String>) -> Option<String> {
    let cookies = seed?;
    if server::provider::validate_ytmusic_cookies(&cookies).await {
        return Some(cookies);
    }
    if let Ok(Some(rotated)) = server::ytmusic::verify_session_keepalive::tick(&cookies).await
        && server::provider::validate_ytmusic_cookies(&rotated).await
    {
        return Some(rotated);
    }
    None
}

/// The old settings-actions flow: resume from stored cookies, then from the
/// isolated browser profile, and only then force a full browser sign-in,
/// which must validate before it is trusted. Skipping the resume steps would
/// wipe the profile and demand password/2FA on every transient error.
#[cfg(not(target_os = "android"))]
async fn ensure_ytmusic_signed_in(
    config_cookies: Option<String>,
    browser: config::Browser,
    server_id: &str,
) -> Result<String, String> {
    if let Some(cookies) = try_resume_ytmusic(config_cookies).await {
        return Ok(cookies);
    }

    let profile = server::ytmusic::isolated_profile::profile_dir(server_id);
    if profile.is_dir() {
        let from_profile = server::ytmusic::cookies::extract_from(browser, &profile)
            .await
            .ok();
        if let Some(cookies) = try_resume_ytmusic(from_profile).await {
            return Ok(cookies);
        }
    }

    let cookies = server::ytmusic::isolated_profile::launch_signin_and_extract(
        browser,
        server_id,
        Duration::from_secs(300),
    )
    .await?;
    if !server::provider::validate_ytmusic_cookies(&cookies).await {
        return Err("sign-in completed but YouTube Music validation still failed".to_string());
    }
    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use super::FrontendService;

    #[test]
    fn server_ids_cannot_use_the_local_source_namespace() {
        for id in ["", "local", "local:library"] {
            let error = FrontendService::validate_server_id(id).expect_err("reserved id");
            assert_eq!(error.code, api::ErrorCode::InvalidInput);
        }
        assert!(FrontendService::validate_server_id("server-id").is_ok());
    }

    fn server_track(key: &str, title: &str) -> reader::Track {
        reader::Track {
            id: reader::TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: key.to_string(),
            },
            cover: None,
            album_id: String::new(),
            title: title.to_string(),
            artist: String::new(),
            album: String::new(),
            duration: 1,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: Vec::new(),
        }
    }

    #[test]
    fn track_radio_pins_the_seed_without_duplicates() {
        let fallback = server_track("seed", "fallback seed");
        let queue = FrontendService::pin_radio_seed(
            "seed",
            Some(fallback),
            vec![
                server_track("next-1", "next 1"),
                server_track("seed", "provider seed"),
                server_track("next-2", "next 2"),
                server_track("seed", "duplicate seed"),
            ],
        );

        let keys: Vec<_> = queue
            .iter()
            .map(|track| track.id.key().into_owned())
            .collect();
        assert_eq!(keys, ["seed", "next-1", "next-2"]);
        assert_eq!(queue[0].title, "provider seed");
    }

    #[test]
    fn track_radio_inserts_a_missing_seed_but_keeps_empty_radio_empty() {
        let fallback = server_track("seed", "fallback seed");
        let queue = FrontendService::pin_radio_seed(
            "seed",
            Some(fallback.clone()),
            vec![server_track("next", "next")],
        );
        assert_eq!(queue[0].id.key().as_ref(), "seed");
        assert_eq!(queue[1].id.key().as_ref(), "next");

        assert!(FrontendService::pin_radio_seed("seed", Some(fallback), Vec::new()).is_empty());
    }

    #[test]
    fn local_path_must_be_inside_a_configured_root() {
        let directory = tempfile::tempdir().expect("temp directory");
        let root = directory.path().join("library");
        let outside = directory.path().join("outside.flac");
        std::fs::create_dir(&root).expect("create library root");
        std::fs::write(root.join("inside.flac"), b"audio").expect("create inside file");
        std::fs::write(&outside, b"audio").expect("create outside file");
        let config = config::AppConfig {
            music_directory: vec![root.clone()],
            ..Default::default()
        };

        assert!(FrontendService::allowed_local_path(
            &config,
            &root.join("inside.flac")
        ));
        assert!(!FrontendService::allowed_local_path(&config, &outside));
    }

    #[cfg(unix)]
    #[test]
    fn local_path_rejects_a_symlink_that_escapes_the_root() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temp directory");
        let root = directory.path().join("library");
        let outside = directory.path().join("outside.flac");
        let link = root.join("escaped.flac");
        std::fs::create_dir(&root).expect("create library root");
        std::fs::write(&outside, b"audio").expect("create outside file");
        symlink(&outside, &link).expect("create symlink");
        let config = config::AppConfig {
            music_directory: vec![root],
            ..Default::default()
        };

        assert!(!FrontendService::allowed_local_path(&config, &link));
    }
}
