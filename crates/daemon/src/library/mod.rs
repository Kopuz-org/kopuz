//! LibraryService: database-backed track reads and queue materialization.
//!
//! First slice of the daemon's library ownership: read-only queries plus the
//! [`QueueMaterializer`] impl, so "play this album" resolves inside the daemon
//! and the track list never round-trips through a client. Scan, sync, and
//! write paths move in with the job runner.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use api::{ApiError, ApiEvent, JobKind, JobRef, Page, QueueContext, Table, TrackFilter, TrackPage};
use reader::Track;
use tokio::sync::watch;

use crate::error::db as db_error;
use crate::jobs::{JobCtx, JobRunner};
use crate::session::{QueueMaterializer, SessionHandle};

pub struct LibraryService {
    db: db::Db,
    source: config::Source,
    station_registry: std::sync::RwLock<Arc<radio::registry::StationRegistry>>,
    cover_cache: PathBuf,
    config_rx: OnceLock<watch::Receiver<config::AppConfig>>,
    session: OnceLock<SessionHandle>,
    transient: std::sync::Mutex<TransientTracks>,
}

#[derive(Default)]
struct TransientTracks {
    by_uid: HashMap<String, Track>,
    order: VecDeque<String>,
}

const MAX_TRANSIENT_TRACKS: usize = 4096;

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

fn map_sort(sort: Option<&str>) -> db::TrackSort {
    match sort {
        Some(encoded) if encoded.starts_with("fields:") => encoded
            .strip_prefix("fields:")
            .and_then(|json| serde_json::from_str(json).ok())
            .map(db::TrackSort::Fields)
            .unwrap_or_default(),
        Some("title") => db::TrackSort::Title,
        Some("artist") => db::TrackSort::Artist,
        Some("album") => db::TrackSort::Album,
        Some("date_added") => db::TrackSort::DateAdded,
        Some("play_count") => db::TrackSort::PlayCount,
        _ => db::TrackSort::ArtistAlbum,
    }
}

fn lyrics_view(lyrics: utils::lyrics::Lyrics) -> api::LyricsView {
    let to_ms = |seconds: f64| (seconds.max(0.0) * 1000.0) as u64;
    match lyrics {
        utils::lyrics::Lyrics::Plain(text) => api::LyricsView {
            plain: Some(text),
            synced: Vec::new(),
        },
        utils::lyrics::Lyrics::Synced(lines) => api::LyricsView {
            plain: None,
            synced: lines
                .into_iter()
                .map(|line| api::LyricLineView {
                    start_ms: to_ms(line.start_time),
                    end_ms: line.end_time.map(to_ms),
                    text: line.text,
                    chunks: line
                        .chunks
                        .into_iter()
                        .map(|chunk| api::LyricChunkView {
                            start_ms: to_ms(chunk.start_time),
                            text: chunk.text,
                        })
                        .collect(),
                    parent_line_index: line.parent_line_index.map(|index| index as u32),
                    background: line.background,
                    opposite_turn: line.opposite_turn,
                })
                .collect(),
        },
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
            station_registry: std::sync::RwLock::new(station_registry),
            cover_cache,
            config_rx: OnceLock::new(),
            session: OnceLock::new(),
            transient: std::sync::Mutex::new(TransientTracks::default()),
        }
    }

    /// Late-bound session wiring (the session needs the materializer first):
    /// gives the service live config and the event stream for invalidations.
    pub fn attach_session(&self, session: SessionHandle) {
        let _ = self.config_rx.set(session.config_watch());
        let _ = self.session.set(session);
    }

    pub fn set_station_registry(&self, registry: Arc<radio::registry::StationRegistry>) {
        if let Ok(mut current) = self.station_registry.write() {
            *current = registry.clone();
        }
        if let Some(session) = self.session.get() {
            session.set_station_registry(registry);
        }
    }

    pub(crate) fn register_transient(&self, tracks: &[Track]) {
        let Ok(mut cache) = self.transient.lock() else {
            return;
        };
        for track in tracks {
            let uid = track.id.uid();
            cache.order.retain(|saved| saved != &uid);
            cache.order.push_back(uid.clone());
            cache.by_uid.insert(uid, track.clone());
        }
        while cache.order.len() > MAX_TRANSIENT_TRACKS {
            if let Some(uid) = cache.order.pop_front() {
                cache.by_uid.remove(&uid);
            }
        }
    }

    pub(crate) fn transient_track(&self, key: &str) -> Option<Track> {
        let service = self.current_config().server.map(|server| server.service)?;
        let uid = reader::TrackId::Server {
            service,
            item_id: key.to_string(),
        }
        .uid();
        self.transient.lock().ok()?.by_uid.get(&uid).cloned()
    }

    pub(crate) fn transient_track_for_info(&self, value: &api::TrackInfo) -> Option<Track> {
        let service = value
            .service
            .and_then(crate::wire::music_service_from_api)?;
        let uid = reader::TrackId::Server {
            service,
            item_id: value.key.clone(),
        }
        .uid();
        self.transient.lock().ok()?.by_uid.get(&uid).cloned()
    }

    pub(crate) fn track_from_info(value: &api::TrackInfo) -> Result<Track, ApiError> {
        if value.key.trim().is_empty() {
            return Err(ApiError::invalid_input("inline track key is required"));
        }
        let service = value
            .service
            .and_then(crate::wire::music_service_from_api)
            .ok_or_else(|| ApiError::invalid_input("inline tracks must name a media service"))?;
        let id = reader::TrackId::Server {
            service,
            item_id: value.key.clone(),
        };
        if !value.uid.is_empty() && value.uid != id.uid() {
            return Err(ApiError::invalid_input(
                "inline track uid does not match its service and key",
            ));
        }
        Ok(crate::wire::track_from_info_parts(value, id, None))
    }

    fn current_config(&self) -> config::AppConfig {
        self.config_rx
            .get()
            .map(|rx| rx.borrow().clone())
            .unwrap_or_default()
    }

    pub(crate) fn invalidate(&self, table: Table) {
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

    pub async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError> {
        let config = self.current_config();
        let (total, rows) = self.tracks_raw(filter, page).await?;
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: rows
                .iter()
                .map(|track| crate::wire::track_info(track, &config))
                .collect(),
        })
    }

    pub(crate) async fn tracks_raw(
        &self,
        filter: TrackFilter,
        page: Page,
    ) -> Result<(u32, Vec<Track>), ApiError> {
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
            return Ok((total, items));
        }

        let db_filter = db::TrackFilter {
            source: self.query_source(),
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
        Ok((total, items))
    }

    pub async fn folder_tracks(
        &self,
        prefix: &str,
        page: Page,
    ) -> Result<api::TrackPage, ApiError> {
        let config = self.current_config();
        let rows = self
            .db
            .folder_tracks(&self.query_source(), prefix)
            .await
            .map_err(db_error)?;
        let total = rows.len() as u32;
        let items = rows
            .iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .map(|track| crate::wire::track_info(track, &config))
            .collect();
        Ok(api::TrackPage {
            total,
            offset: page.offset,
            items,
        })
    }

    pub fn stats(&self) -> api::StatsView {
        api::StatsView {
            listen_counts: self.current_config().listen_counts.clone(),
        }
    }

    /// Lyrics for one library track, through the app's full provider chain
    /// (local .lrc, server lyrics API, synced fallbacks, lrclib) with its
    /// process cache. Radio has no lyrics by construction.
    async fn lyrics_request(&self, key: &str) -> Result<utils::lyrics::LyricsRequest, ApiError> {
        let config = self.current_config();
        let track = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(db_error)?
            .into_iter()
            .next()
            .or_else(|| self.transient_track(key))
            .ok_or_else(|| ApiError::not_found("unknown track key"))?;
        if track.duration == u64::MAX {
            return Err(ApiError::invalid_input("radio streams have no lyrics"));
        }

        let mut request = utils::lyrics::LyricsRequest::new(
            &track.artist,
            &track.title,
            &track.album,
            track.duration,
            track.id.uid(),
        )
        .prefer_local(config.prefer_local_lyrics)
        .enable_musixmatch(config.enable_musixmatch_lyrics);
        if let Some(server) = &config.server {
            request = request.with_server(
                Some(&server.url),
                server.access_token.as_deref(),
                server.user_id.as_deref(),
            );
            if utils::lyrics::cached_lyrics_for_request(&request).is_some() {
                return Ok(request);
            }
            if server.service == config::MusicService::AppleMusic
                && let Some(token) = server.access_token.as_ref()
            {
                match server::applemusic::auth::get_bearer_token().await {
                    Ok(bearer_token) => {
                        request = request.apple_music_auth(utils::lyrics::AppleMusicLyricsAuth {
                            token: token.clone(),
                            bearer_token,
                            storefront: server.apple_music_storefront.clone(),
                            language: server.apple_music_language.clone(),
                            catalog_id: track.id.key().into_owned(),
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Apple Music lyrics authorization failed");
                    }
                }
            }
        }
        Ok(request)
    }

    pub async fn lyrics(&self, key: &str) -> Result<api::LyricsView, ApiError> {
        let request = self.lyrics_request(key).await?;

        let lyrics = match utils::lyrics::cached_lyrics_for_request(&request) {
            Some(cached) => cached,
            None => utils::lyrics::fetch_lyrics_for_request(&request).await,
        };
        lyrics
            .map(lyrics_view)
            .ok_or_else(|| ApiError::not_found("no lyrics found"))
    }

    pub fn lyrics_stream(self: &Arc<Self>, key: String) -> api::LyricsStream {
        use futures_util::StreamExt as _;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let service = self.clone();
        tokio::spawn(async move {
            let cancel = tx.clone();
            let request = match tokio::select! {
                biased;
                () = cancel.closed() => return,
                request = service.lyrics_request(&key) => request,
            } {
                Ok(request) => request,
                Err(error) => {
                    let _ = tx.send(Err(error));
                    return;
                }
            };
            if let Some(cached) = utils::lyrics::cached_lyrics_for_request(&request) {
                let result = cached
                    .map(lyrics_view)
                    .ok_or_else(|| ApiError::not_found("no lyrics found"));
                let _ = tx.send(result);
                return;
            }
            let mut last = None;
            let final_lyrics = {
                let fetch =
                    utils::lyrics::fetch_lyrics_progressive_for_request(&request, |lyrics| {
                        let view = lyrics_view(lyrics);
                        if last.as_ref() != Some(&view) {
                            let _ = tx.send(Ok(view.clone()));
                            last = Some(view);
                        }
                    });
                tokio::pin!(fetch);
                tokio::select! {
                    biased;
                    () = cancel.closed() => return,
                    lyrics = &mut fetch => lyrics.map(lyrics_view),
                }
            };
            match final_lyrics {
                Some(view) if last.as_ref() != Some(&view) => {
                    let _ = tx.send(Ok(view));
                }
                Some(_) => {}
                None if last.is_none() => {
                    let _ = tx.send(Err(ApiError::not_found("no lyrics found")));
                }
                None => {}
            }
        });
        futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        })
        .boxed()
    }

    /// Synthetic radio track, seeded from the manifest so no client ever sees
    /// raw ids while the first metadata update is in flight. The `u64::MAX`
    /// duration sentinel is translated to `TrackKind::Radio` at the wire.
    fn radio_track(&self, station_id: &str, stream_id: &str) -> Track {
        let registry = self
            .station_registry
            .read()
            .map(|registry| registry.clone())
            .unwrap_or_else(|_| Arc::new(radio::registry::StationRegistry::default()));
        let station = registry.get(station_id);
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
    async fn probe_local_files(keys: Vec<String>, cover_cache: PathBuf) -> Vec<Track> {
        if keys.is_empty() {
            return Vec::new();
        }
        tokio::task::spawn_blocking(move || {
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

mod jobs;

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
                let mut local_missing = Vec::new();
                for key in missing {
                    if let Some(track) = self.transient_track(&key) {
                        by_key.insert(key, track);
                    } else {
                        local_missing.push(key);
                    }
                }
                for track in Self::probe_local_files(local_missing, self.cover_cache.clone()).await
                {
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
                .tracks_raw(
                    filter.clone(),
                    Page {
                        offset: 0,
                        limit: u32::MAX,
                    },
                )
                .await?
                .1),
            QueueContext::Radio {
                station_id,
                stream_id,
            } => Ok(vec![self.radio_track(station_id, stream_id)]),
            QueueContext::InlineTracks { tracks } => {
                let expected = self
                    .current_config()
                    .server
                    .map(|server| server.service)
                    .ok_or_else(|| {
                        ApiError::invalid_input("inline tracks require an active remote source")
                    })?;
                tracks
                    .iter()
                    .map(|value| {
                        let service = value
                            .service
                            .and_then(crate::wire::music_service_from_api)
                            .ok_or_else(|| {
                                ApiError::invalid_input("inline tracks must name a media service")
                            })?;
                        if service != expected {
                            return Err(ApiError::invalid_input(
                                "inline track does not belong to the active source",
                            ));
                        }
                        let decoded = Self::track_from_info(value)?;
                        Ok(self.transient_track_for_info(value).unwrap_or(decoded))
                    })
                    .collect::<Result<Vec<_>, _>>()
            }
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

    #[tokio::test]
    async fn broad_track_queries_follow_the_session_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("source.db")).await.expect("db");
        database
            .upsert_tracks(&config::Source::Local, &[track(0, "Local")])
            .await
            .expect("seed local");
        database
            .upsert_tracks(
                &config::Source::Server("server-b".into()),
                &[track(1, "Remote")],
            )
            .await
            .expect("seed server");
        let library = Arc::new(LibraryService::new(
            database,
            config::Source::Local,
            Arc::new(radio::registry::StationRegistry::default()),
            dir.path().join("covers"),
        ));
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("player");
        let services = crate::session::PlaybackServices {
            config: config::AppConfig {
                active_source: config::Source::Server("server-b".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let session =
            crate::session::SessionHandle::spawn_with_player(library.clone(), player, services);
        library.attach_session(session);

        let page = library
            .tracks(TrackFilter::default(), Page::default())
            .await
            .expect("tracks");
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].artist, "Remote");
    }

    #[tokio::test]
    async fn inline_tracks_preserve_qualified_transient_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("inline.db")).await.expect("db");
        let library = Arc::new(LibraryService::new(
            database,
            config::Source::Local,
            Arc::new(radio::registry::StationRegistry::default()),
            dir.path().join("covers"),
        ));
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("player");
        let mut remote = config::MusicServer::new("YT Music".into(), String::new());
        remote.id = Some("yt".into());
        remote.service = config::MusicService::YtMusic;
        let services = crate::session::PlaybackServices {
            config: config::AppConfig {
                active_source: config::Source::Server("yt".into()),
                server: Some(remote),
                ..Default::default()
            },
            ..Default::default()
        };
        let session =
            crate::session::SessionHandle::spawn_with_player(library.clone(), player, services);
        library.attach_session(session);
        let original = Track {
            id: reader::TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: "same-id".into(),
            },
            cover: Some("https://cover.test/image".into()),
            album_id: "album".into(),
            title: "Original".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration: 180,
            khz: 44,
            bitrate: 320,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: Vec::new(),
        };
        library.register_transient(std::slice::from_ref(&original));
        let info = api::TrackInfo {
            key: "same-id".into(),
            uid: "ytmusic:same-id".into(),
            title: "Wire".into(),
            duration_ms: Some(180_000),
            service: Some(api::MusicService::YtMusic),
            ..Default::default()
        };
        let materialized = library
            .materialize(&QueueContext::InlineTracks {
                tracks: vec![info.clone()],
            })
            .await
            .expect("inline track");
        assert_eq!(materialized, vec![original]);

        let error = library
            .materialize(&QueueContext::InlineTracks {
                tracks: vec![api::TrackInfo {
                    service: Some(api::MusicService::Spotify),
                    ..info
                }],
            })
            .await
            .expect_err("cross-source track rejected");
        assert_eq!(error.code, api::ErrorCode::InvalidInput);

        let error = library
            .materialize(&QueueContext::InlineTracks {
                tracks: vec![api::TrackInfo {
                    uid: "spotify:same-id".into(),
                    ..api::TrackInfo {
                        key: "same-id".into(),
                        service: Some(api::MusicService::YtMusic),
                        ..Default::default()
                    }
                }],
            })
            .await
            .expect_err("inconsistent uid rejected");
        assert_eq!(error.code, api::ErrorCode::InvalidInput);
    }
}
