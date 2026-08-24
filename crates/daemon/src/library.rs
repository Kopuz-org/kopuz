//! LibraryService: database-backed track reads and queue materialization.
//!
//! First slice of the daemon's library ownership: read-only queries plus the
//! [`QueueMaterializer`] impl, so "play this album" resolves inside the daemon
//! and the track list never round-trips through a client. Scan, sync, and
//! write paths move in with the job runner.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use api::{ApiError, ApiEvent, JobKind, JobRef, Page, QueueContext, Table, TrackFilter, TrackPage};
use reader::Track;
use tokio::sync::watch;

use crate::jobs::{JobCtx, JobRunner};
use crate::session::{QueueMaterializer, SessionHandle};

pub struct LibraryService {
    db: db::Db,
    source: config::Source,
    station_registry: Arc<radio::registry::StationRegistry>,
    cover_cache: PathBuf,
    config_rx: OnceLock<watch::Receiver<config::AppConfig>>,
    session: OnceLock<SessionHandle>,
}

fn normalize_album_id(id: &str) -> String {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() >= 2
        && (parts[0] == "subsonic" || parts[0] == "custom" || parts[0] == "jellyfin")
    {
        format!("{}:{}", parts[0], parts[1])
    } else {
        id.to_string()
    }
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

fn map_sort(sort: Option<&str>) -> db::TrackSort {
    match sort {
        Some("title") => db::TrackSort::Title,
        Some("artist") => db::TrackSort::Artist,
        Some("album") => db::TrackSort::Album,
        Some("date_added") => db::TrackSort::DateAdded,
        Some("play_count") => db::TrackSort::PlayCount,
        _ => db::TrackSort::ArtistAlbum,
    }
}

fn matches_search(track: &Track, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    [&track.title, &track.artist, &track.album]
        .into_iter()
        .any(|field| field.to_lowercase().contains(&needle))
}

impl LibraryService {
    pub fn new(
        db: db::Db,
        source: config::Source,
        station_registry: Arc<radio::registry::StationRegistry>,
        cover_cache: PathBuf,
    ) -> Self {
        Self {
            db,
            source,
            station_registry,
            cover_cache,
            config_rx: OnceLock::new(),
            session: OnceLock::new(),
        }
    }

    /// Late-bound session wiring (the session needs the materializer first):
    /// gives the service live config and the event stream for invalidations.
    pub fn attach_session(&self, session: SessionHandle) {
        let _ = self.config_rx.set(session.config_watch());
        let _ = self.session.set(session);
    }

    fn current_config(&self) -> config::AppConfig {
        self.config_rx
            .get()
            .map(|rx| rx.borrow().clone())
            .unwrap_or_default()
    }

    fn invalidate(&self, table: Table) {
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        if let Some(session) = self.session.get() {
            session.emit_event(ApiEvent::LibraryInvalidated {
                table,
                generation: GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
            });
        }
    }

    /// The source library reads run against: the live active source once the
    /// session is attached, the construction-time source before that.
    fn query_source(&self) -> config::Source {
        self.config_rx
            .get()
            .map(|rx| rx.borrow().active_source.clone())
            .unwrap_or_else(|| self.source.clone())
    }

    fn scan_roots(config: &config::AppConfig) -> Vec<(config::Source, Vec<PathBuf>)> {
        std::iter::once((config::Source::Local, config.music_directory.clone()))
            .chain(config.local_sources.iter().map(|source| {
                (
                    config::Source::LocalLibrary(source.id.clone()),
                    source.directories.clone(),
                )
            }))
            .collect()
    }

    pub fn spawn_scan(self: &Arc<Self>, runner: &JobRunner) -> Result<JobRef, ApiError> {
        let service = self.clone();
        runner.start(JobKind::Scan, move |ctx| async move {
            let config = service.current_config();
            service.run_scan(&ctx, &config).await
        })
    }

    pub fn spawn_remote_sync(self: &Arc<Self>, runner: &JobRunner) -> Result<JobRef, ApiError> {
        let service = self.clone();
        runner.start(JobKind::LibrarySync, move |ctx| async move {
            let config = service.current_config();
            service.run_remote_sync(&ctx, &config).await
        })
    }

    /// Local filesystem scan, ported from the app's rescan effect: DB-seeded
    /// working set, per-root scan, retain-by-root, chunked upserts, prune,
    /// local artist images with self-heal, then cover indexing and (when
    /// enabled) network cover fetching. The job runner's single-flight
    /// replaces the app's epoch supersession; cancellation is checked between
    /// phases and chunks.
    pub async fn run_scan(&self, ctx: &JobCtx, config: &config::AppConfig) -> Result<(), ApiError> {
        let db_error = |error: db::DbError| ApiError::internal(format!("database error: {error}"));
        for (source, configured_dirs) in Self::scan_roots(config) {
            if ctx.cancelled() {
                return Ok(());
            }
            let scannable_dirs: Vec<PathBuf> = configured_dirs
                .iter()
                .filter(|dir| dir.exists())
                .cloned()
                .collect();

            if configured_dirs.is_empty() {
                self.db
                    .prune_source(&source, &[], &[])
                    .await
                    .map_err(db_error)?;
                self.invalidate(Table::Tracks);
                self.invalidate(Table::Albums);
                continue;
            }

            ctx.progress("seeding", None, None, None);
            let mut seed_tracks: Vec<Track> = Vec::new();
            let mut seen_keys = HashSet::new();
            for dir in &configured_dirs {
                let mut prefix = dir.to_string_lossy().into_owned();
                if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
                    prefix.push(std::path::MAIN_SEPARATOR);
                }
                let found = self
                    .db
                    .folder_tracks(&source, &prefix)
                    .await
                    .map_err(db_error)?;
                for track in found {
                    if seen_keys.insert(track.id.key().into_owned()) {
                        seed_tracks.push(track);
                    }
                }
            }
            let seed_albums = self.db.albums(&source).await.map_err(db_error)?;
            let mut library = reader::Library {
                root_paths: configured_dirs.clone(),
                tracks: seed_tracks,
                albums: seed_albums,
                ..Default::default()
            };

            let progress_ctx = ctx.clone();
            let progress: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |file: String| {
                progress_ctx.progress_throttled("scanning", Some(file));
            });
            for dir in &scannable_dirs {
                if ctx.cancelled() {
                    return Ok(());
                }
                let _ = reader::scan_directory(
                    dir.clone(),
                    self.cover_cache.clone(),
                    &mut library,
                    progress.clone(),
                )
                .await;
            }

            library.tracks.retain(|track| {
                let in_configured_root = configured_dirs.iter().any(|dir| {
                    track
                        .id
                        .local_path()
                        .is_some_and(|path| path.starts_with(dir))
                });
                let in_scannable_root = scannable_dirs.iter().any(|dir| {
                    track
                        .id
                        .local_path()
                        .is_some_and(|path| path.starts_with(dir))
                });
                in_configured_root
                    && (!in_scannable_root
                        || track.id.local_path().is_some_and(|path| path.exists()))
            });
            let valid_album_ids: HashSet<_> = library
                .tracks
                .iter()
                .map(|track| track.album_id.clone())
                .collect();
            library
                .albums
                .retain(|album| valid_album_ids.contains(&album.id));

            let total = library.tracks.len() as u64;
            let mut done = 0u64;
            for chunk in library.tracks.chunks(100) {
                if ctx.cancelled() {
                    return Ok(());
                }
                self.db
                    .upsert_tracks(&source, chunk)
                    .await
                    .map_err(db_error)?;
                done += chunk.len() as u64;
                ctx.progress("persisting", Some(done), Some(total), None);
                self.invalidate(Table::Tracks);
            }
            self.db
                .upsert_albums(&source, &library.albums)
                .await
                .map_err(db_error)?;
            let keep_keys: Vec<String> = library
                .tracks
                .iter()
                .map(|track| track.id.key().into_owned())
                .collect();
            let keep_albums: Vec<String> = library
                .albums
                .iter()
                .map(|album| album.id.clone())
                .collect();
            self.db
                .prune_source(&source, &keep_keys, &keep_albums)
                .await
                .map_err(db_error)?;
            for (artist, image) in &library.local_artist_images {
                let path = image.to_string_lossy().into_owned();
                let _ = self.db.set_artist_image(artist, "local", Some(&path)).await;
            }
            if let Ok((_, photos)) = self.db.artist_images().await {
                for (artist, photo) in photos {
                    if let reader::ArtistImageRef::Local(path) = photo
                        && !path.exists()
                    {
                        let _ = self.db.set_artist_image(&artist, "local", None).await;
                    }
                }
            }
            self.invalidate(Table::Tracks);
            self.invalidate(Table::Albums);

            ctx.progress("indexing covers", None, None, None);
            let missing_local = reader::missing_cover_ids(&library);
            let _ = reader::index_local_covers(
                &mut library,
                self.cover_cache.clone(),
                progress.clone(),
            )
            .await;
            self.persist_resolved_covers(ctx, &source, &library.albums, &missing_local)
                .await;

            if config.auto_fetch_covers && !ctx.cancelled() {
                ctx.progress("fetching covers", None, None, None);
                let lastfm_key = {
                    let key = config.lastfm_api_key.trim().to_owned();
                    (!key.is_empty()).then_some(key)
                };
                let fetcher = reader::cover_fetcher::CoverFetcher::new(
                    self.cover_cache.clone(),
                    config.cover_fetch_strategy,
                    lastfm_key,
                    progress.clone(),
                );
                let missing_before = reader::missing_cover_ids(&library);
                let _ = fetcher.fetch_missing_covers(&mut library).await;
                self.persist_resolved_covers(ctx, &source, &library.albums, &missing_before)
                    .await;
            }
        }
        Ok(())
    }

    async fn persist_resolved_covers(
        &self,
        ctx: &JobCtx,
        source: &config::Source,
        albums: &[reader::Album],
        missing_ids: &HashSet<String>,
    ) {
        let mut changed = false;
        for album in albums {
            if ctx.cancelled() {
                break;
            }
            if !missing_ids.contains(&album.id) {
                continue;
            }
            let Some(cover) = album.cover_path.as_ref() else {
                continue;
            };
            let path = cover.to_string_lossy().into_owned();
            match self
                .db
                .update_album_cover_if_not_manual(source, &album.id, &path)
                .await
            {
                Ok(written) => changed |= written,
                Err(error) => {
                    tracing::warn!(album_id = %album.id, %error, "cover persist failed");
                }
            }
        }
        if changed {
            self.invalidate(Table::Albums);
        }
    }

    /// Remote library pull, ported from `sync_server_library`: fetch the
    /// snapshot, merge manual covers, chunked upserts with invalidations,
    /// artist images, then prune what the server dropped.
    pub async fn run_remote_sync(
        &self,
        ctx: &JobCtx,
        config: &config::AppConfig,
    ) -> Result<(), ApiError> {
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), config));
        if !source.capabilities().sync {
            return Err(ApiError::unsupported(
                "the active source has no library sync",
            ));
        }
        let src = source.source().clone();
        let existing_albums = self
            .db
            .albums(&src)
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
        let merge_cover = |mut album: reader::Album| -> reader::Album {
            if let Some(old) = existing_albums
                .iter()
                .find(|existing| normalize_album_id(&existing.id) == normalize_album_id(&album.id))
            {
                if album.cover_path.is_none() || old.manual_cover {
                    album.cover_path = old.cover_path.clone();
                }
                if old.manual_cover {
                    album.manual_cover = true;
                }
            }
            album
        };

        ctx.progress("fetching library", None, None, None);
        let snapshot = source
            .fetch_library()
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?;

        let merged_albums: Vec<reader::Album> =
            snapshot.albums.into_iter().map(merge_cover).collect();
        let total = (merged_albums.len() + snapshot.tracks.len()) as u64;
        let mut done = 0u64;
        for chunk in merged_albums.chunks(100) {
            if ctx.cancelled() {
                return Ok(());
            }
            source
                .upsert_albums(chunk)
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            done += chunk.len() as u64;
            ctx.progress("persisting", Some(done), Some(total), None);
            self.invalidate(Table::Albums);
        }
        for chunk in snapshot.tracks.chunks(100) {
            if ctx.cancelled() {
                return Ok(());
            }
            source
                .upsert_tracks(chunk)
                .await
                .map_err(|error| ApiError::internal(error.to_string()))?;
            done += chunk.len() as u64;
            ctx.progress("persisting", Some(done), Some(total), None);
            self.invalidate(Table::Tracks);
        }
        for (name, url) in &snapshot.artist_images {
            let _ = source.set_artist_image(name, "server", Some(url)).await;
        }
        let keep_keys: Vec<String> = snapshot
            .tracks
            .iter()
            .map(|track| track.id.key().into_owned())
            .collect();
        let keep_albums: Vec<String> = merged_albums.iter().map(|album| album.id.clone()).collect();
        let _ = source.prune(&keep_keys, &keep_albums).await;
        self.invalidate(Table::Tracks);
        self.invalidate(Table::Albums);
        Ok(())
    }

    pub async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError> {
        if filter.favorite.is_some() {
            return Err(ApiError::unsupported(
                "favorite filtering lands with the favorites service",
            ));
        }

        let narrowed = if let Some(album) = filter.album.as_deref() {
            Some(
                self.db
                    .album_tracks(&self.query_source(), album)
                    .await
                    .map_err(db_error)?,
            )
        } else if let Some(artist) = filter.artist.as_deref() {
            Some(
                self.db
                    .artist_tracks(&self.query_source(), artist, None)
                    .await
                    .map_err(db_error)?,
            )
        } else if let Some(genre) = filter.genre.as_deref() {
            Some(
                self.db
                    .genre_tracks(&self.query_source(), genre)
                    .await
                    .map_err(db_error)?,
            )
        } else {
            None
        };

        if let Some(mut rows) = narrowed {
            if let Some(search) = filter.search.as_deref().filter(|s| !s.is_empty()) {
                rows.retain(|track| matches_search(track, search));
            }
            let total = rows.len() as u32;
            let items = rows
                .into_iter()
                .skip(page.offset as usize)
                .take(page.limit as usize)
                .collect();
            return Ok(TrackPage {
                total,
                offset: page.offset,
                items,
            });
        }

        let db_filter = db::TrackFilter {
            source: self.source.clone(),
            sort: map_sort(filter.sort.as_deref()),
            search: filter.search.unwrap_or_default(),
        };
        let items = self
            .db
            .tracks_page(
                &db_filter,
                db::Page {
                    offset: page.offset,
                    limit: page.limit,
                },
            )
            .await
            .map_err(db_error)?;
        let total = self.db.tracks_count(&db_filter).await.map_err(db_error)?;
        Ok(TrackPage {
            total,
            offset: page.offset,
            items,
        })
    }

    /// Synthetic radio track, seeded from the manifest so no client ever sees
    /// raw ids while the first metadata update is in flight. The `u64::MAX`
    /// duration sentinel is translated to `TrackKind::Radio` at the wire.
    fn radio_track(&self, station_id: &str, stream_id: &str) -> Track {
        let station = self.station_registry.get(station_id);
        let title = station
            .map(|station| station.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| stream_id.to_string());
        let artist = station
            .and_then(|station| match &station.metadata {
                Some(radio::manifest::MetadataSourceDef::Static(meta)) => {
                    Some(meta.resolve(stream_id).1.to_string())
                }
                _ => None,
            })
            .or_else(|| {
                station
                    .and_then(|station| {
                        station.streams.iter().find(|stream| stream.id == stream_id)
                    })
                    .map(|stream| stream.name.clone())
            })
            .filter(|artist| !artist.trim().is_empty())
            .unwrap_or_else(|| "Live Radio".to_string());

        Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(format!(
                "radio:{station_id}:{stream_id}"
            ))),
            cover: None,
            album_id: String::new(),
            title,
            artist,
            album: "Live Radio".to_string(),
            duration: u64::MAX,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
        }
    }

    /// Keys the database does not know but that exist as local audio files are
    /// probed directly, so ad-hoc file playback keeps working alongside the
    /// library.
    async fn probe_local_files(keys: Vec<String>) -> Vec<Track> {
        if keys.is_empty() {
            return Vec::new();
        }
        tokio::task::spawn_blocking(move || {
            let cover_cache = std::env::temp_dir();
            let mut library = reader::Library::default();
            keys.iter()
                .filter_map(|key| {
                    let path = Path::new(key);
                    path.is_file()
                        .then(|| reader::read(path, &cover_cache, &mut library))
                        .flatten()
                })
                .collect()
        })
        .await
        .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl QueueMaterializer for LibraryService {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        match context {
            QueueContext::Tracks { keys } => {
                let known = self
                    .db
                    .tracks_by_keys(&self.query_source(), keys)
                    .await
                    .map_err(db_error)?;
                let mut by_key: HashMap<String, Track> = known
                    .into_iter()
                    .map(|track| (track.id.key().to_string(), track))
                    .collect();
                let missing: Vec<String> = keys
                    .iter()
                    .filter(|key| !by_key.contains_key(*key))
                    .cloned()
                    .collect();
                for track in Self::probe_local_files(missing).await {
                    by_key.insert(track.id.key().to_string(), track);
                }
                Ok(keys.iter().filter_map(|key| by_key.remove(key)).collect())
            }
            QueueContext::Album { id } => self
                .db
                .album_tracks(&self.query_source(), id)
                .await
                .map_err(db_error),
            QueueContext::Artist { name } => self
                .db
                .artist_tracks(&self.query_source(), name, None)
                .await
                .map_err(db_error),
            QueueContext::Genre { name } => self
                .db
                .genre_tracks(&self.query_source(), name)
                .await
                .map_err(db_error),
            QueueContext::Playlist { id } => {
                let store = self
                    .db
                    .load_playlists(&self.query_source())
                    .await
                    .map_err(db_error)?;
                let playlist = store
                    .playlists
                    .iter()
                    .find(|playlist| playlist.id == *id)
                    .ok_or_else(|| ApiError::not_found("playlist not found"))?;
                self.db
                    .tracks_by_keys(&self.query_source(), &playlist.tracks)
                    .await
                    .map_err(db_error)
            }
            QueueContext::Filter { filter } => Ok(self
                .tracks(
                    filter.clone(),
                    Page {
                        offset: 0,
                        limit: u32::MAX,
                    },
                )
                .await?
                .items),
            QueueContext::Radio {
                station_id,
                stream_id,
            } => Ok(vec![self.radio_track(station_id, stream_id)]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(n: usize, artist: &str) -> Track {
        Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(format!("/lib/{n}.flac"))),
            cover: None,
            album_id: format!("album-{}", n % 2),
            title: format!("song {n}"),
            artist: artist.to_string(),
            album: format!("album {}", n % 2),
            duration: 60,
            khz: 44,
            bitrate: 320,
            track_number: Some(n as u32),
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
        }
    }

    async fn seeded_library() -> (tempfile::TempDir, LibraryService) {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("test.db"))
            .await
            .expect("db init");
        let source = config::Source::default();
        let tracks: Vec<Track> = (0..5)
            .map(|n| track(n, if n < 3 { "Ada" } else { "Boris" }))
            .collect();
        database
            .upsert_tracks(&source, &tracks)
            .await
            .expect("seed tracks");
        let cover_cache = dir.path().join("covers");
        let service = LibraryService::new(
            database,
            source,
            Arc::new(radio::registry::StationRegistry::default()),
            cover_cache,
        );
        (dir, service)
    }

    #[tokio::test]
    async fn tracks_pages_and_searches_the_database() {
        let (_dir, library) = seeded_library().await;

        let page = library
            .tracks(
                TrackFilter::default(),
                Page {
                    offset: 0,
                    limit: 2,
                },
            )
            .await
            .expect("page");
        assert_eq!(page.total, 5);
        assert_eq!(page.items.len(), 2);

        let page = library
            .tracks(
                TrackFilter {
                    search: Some("song 4".into()),
                    ..Default::default()
                },
                Page::default(),
            )
            .await
            .expect("search");
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].title, "song 4");

        let page = library
            .tracks(
                TrackFilter {
                    artist: Some("Ada".into()),
                    ..Default::default()
                },
                Page::default(),
            )
            .await
            .expect("artist listing");
        assert_eq!(page.total, 3);
    }

    #[tokio::test]
    async fn materialize_resolves_database_contexts() {
        let (_dir, library) = seeded_library().await;

        let tracks = library
            .materialize(&QueueContext::Album {
                id: "album-1".into(),
            })
            .await
            .expect("album context");
        assert_eq!(tracks.len(), 2);

        let tracks = library
            .materialize(&QueueContext::Tracks {
                keys: vec!["/lib/2.flac".into(), "/lib/0.flac".into(), "/nope".into()],
            })
            .await
            .expect("keys context");
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "song 2");
        assert_eq!(tracks[1].title, "song 0");

        let missing = library
            .materialize(&QueueContext::Playlist { id: "ghost".into() })
            .await
            .expect_err("unknown playlist");
        assert_eq!(missing.code, api::ErrorCode::NotFound);

        let radio = library
            .materialize(&QueueContext::Radio {
                station_id: "st".into(),
                stream_id: "hi".into(),
            })
            .await
            .expect("radio context");
        assert_eq!(radio[0].duration, u64::MAX);
        assert_eq!(radio[0].title, "hi");
    }
}
