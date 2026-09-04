use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{ApiError, ErrorCode};
use server::source::SourceError;

use crate::{ConfigService, LibraryService, SessionHandle};
mod catalog;
mod external;
mod integrations;
mod library;
mod mutations;
mod playlists;
mod radio_ops;
mod sources;

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
