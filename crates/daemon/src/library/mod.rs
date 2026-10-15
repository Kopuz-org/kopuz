//! LibraryService: database-backed track reads and queue materialization.
//!
//! First slice of the daemon's library ownership: read-only queries plus the
//! [`QueueMaterializer`] impl, so "play this album" resolves inside the daemon
//! and the track list never round-trips through a client. Scan, sync, and
//! write paths move in with the job runner.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use api::{ApiError, JobKind, JobRef, Page, QueueContext, Table, TrackFilter, TrackPage};
use reader::Track;
use server::source::RadioPage;
use tokio::sync::watch;

use crate::jobs::{JobCtx, JobRunner, Trigger};
use crate::session::{QueueMaterializer, SessionHandle};

pub struct LibraryService {
    db: db::Db,
    source: config::Source,
    /// Swapped whole when the radio service rebuilds it from config, so a
    /// registry toggle takes effect without a restart.
    station_registry: std::sync::RwLock<Arc<radio::registry::StationRegistry>>,
    cover_cache: PathBuf,
    /// The cover fetcher's Last.fm key.
    credentials: crate::CredentialStore,
    config_rx: OnceLock<watch::Receiver<config::Settings>>,
    sources_rx: OnceLock<watch::Receiver<Arc<server::sources::Sources>>>,
    session: OnceLock<SessionHandle>,
    catalog: OnceLock<Arc<dyn crate::catalog::CatalogQueue>>,
    transient: std::sync::Mutex<TransientTracks>,
    /// Artists a photo lookup is out for, so a grid re-asking mid-batch does not send the same lookups again.
    artwork_in_flight: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// When each source's photos were last swept for, by source.
    photo_sweeps: std::sync::Mutex<HashMap<String, std::time::Instant>>,
}

/// Tracks that exist but the database has never seen: a browse shelf, a
/// search hit from a remote catalog, a radio mix. Keeping them here is what
/// lets the rest of the API stay key-addressed -- a client queues, hearts or
/// asks for the artwork of a catalog row exactly as it would a library one.
///
/// Bounded and least-recently-registered-out, because the ceiling is however
/// much browsing someone does in one session.
#[derive(Default)]
struct TransientTracks {
    by_key: HashMap<String, Track>,
    order: std::collections::VecDeque<String>,
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

fn no_such(what: &str) -> ApiError {
    ApiError::not_found(format!("{what} not found"))
}

/// A source that cannot look an id up has nothing by that id.
fn missing_as(what: &'static str) -> impl Fn(ApiError) -> ApiError {
    move |error| match error.code {
        api::ErrorCode::Unsupported => no_such(what),
        _ => error,
    }
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

fn map_sort(sort: api::TrackSort) -> db::TrackSort {
    match sort {
        api::TrackSort::Default => db::TrackSort::ArtistAlbum,
        api::TrackSort::Title => db::TrackSort::Title,
        api::TrackSort::Artist => db::TrackSort::Artist,
        api::TrackSort::Album => db::TrackSort::Album,
        api::TrackSort::DateAdded => db::TrackSort::DateAdded,
        api::TrackSort::PlayCount => db::TrackSort::PlayCount,
        api::TrackSort::Fields(fields) => db::TrackSort::Fields(fields),
    }
}

fn db_page(page: Page) -> db::Page {
    db::Page {
        offset: page.offset,
        limit: page.limit,
    }
}

fn album_query(source: config::Source, query: api::AlbumQuery) -> db::AlbumQuery {
    db::AlbumQuery {
        source,
        search: query.search.unwrap_or_default(),
        genre: query.genre,
        year_from: query.year_from,
        year_to: query.year_to,
        artist_key: query.artist_key,
        downloaded: query.downloaded,
        ids: query.ids,
        sort: query.sort,
    }
}

fn artist_query(source: config::Source, query: api::ArtistQuery) -> db::ArtistQuery {
    db::ArtistQuery {
        source,
        search: query.search.unwrap_or_default(),
        downloaded: query.downloaded,
        sort: query.sort,
    }
}

fn lyrics_view(lyrics: server::lyrics::Lyrics) -> api::LyricsView {
    let to_ms = |seconds: f64| (seconds.max(0.0) * 1000.0) as u64;
    match lyrics {
        server::lyrics::Lyrics::Plain(text) => api::LyricsView {
            plain: Some(text),
            synced: Vec::new(),
        },
        server::lyrics::Lyrics::Synced(lines) => api::LyricsView {
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

impl LibraryService {
    pub fn new(
        db: db::Db,
        source: config::Source,
        station_registry: Arc<radio::registry::StationRegistry>,
        cover_cache: PathBuf,
        credentials: crate::CredentialStore,
    ) -> Self {
        Self {
            db,
            source,
            station_registry: std::sync::RwLock::new(station_registry),
            cover_cache,
            credentials,
            config_rx: OnceLock::new(),
            sources_rx: OnceLock::new(),
            session: OnceLock::new(),
            catalog: OnceLock::new(),
            transient: std::sync::Mutex::new(TransientTracks::default()),
            artwork_in_flight: Arc::default(),
            photo_sweeps: std::sync::Mutex::default(),
        }
    }

    /// Remember rows that came from the network, so a later request naming
    /// one by key can still resolve it.
    pub fn register_transient(&self, tracks: &[Track]) {
        let Ok(mut cache) = self.transient.lock() else {
            return;
        };
        for track in tracks {
            let key = track.id.key().into_owned();
            cache.order.retain(|saved| saved != &key);
            cache.order.push_back(key.clone());
            cache.by_key.insert(key, track.clone());
        }
        while cache.order.len() > MAX_TRANSIENT_TRACKS {
            if let Some(key) = cache.order.pop_front() {
                cache.by_key.remove(&key);
            }
        }
    }

    pub fn transient_track(&self, key: &str) -> Option<Track> {
        self.transient.lock().ok()?.by_key.get(key).cloned()
    }

    fn catalog_service(&self) -> Result<&dyn crate::catalog::CatalogQueue, ApiError> {
        self.catalog
            .get()
            .map(Arc::as_ref)
            .ok_or_else(|| ApiError::unsupported("this daemon runs without a catalog service"))
    }

    /// Late-bound, because the catalog service needs this one to register
    /// what it fetches. Without it, a mix seeded by a track cannot be built,
    /// nor a queue from a catalog id the library does not hold.
    pub fn attach_catalog(&self, catalog: Arc<dyn crate::catalog::CatalogQueue>) {
        let _ = self.catalog.set(catalog);
    }

    /// Adopt a rebuilt station registry, and hand it to the session so a
    /// stream URL resolves against the same one at load time.
    pub fn set_station_registry(&self, registry: Arc<radio::registry::StationRegistry>) {
        if let Ok(mut current) = self.station_registry.write() {
            *current = registry.clone();
        }
        if let Some(session) = self.session.get() {
            session.set_station_registry(registry);
        }
    }

    /// Late-bound session wiring (the session needs the materializer first):
    /// gives the service live config and the event stream for invalidations.
    pub fn attach_session(&self, session: SessionHandle) {
        let _ = self.config_rx.set(session.settings_watch());
        let _ = self.sources_rx.set(session.sources_watch());
        let _ = self.session.set(session);
    }

    fn current_config(&self) -> config::Settings {
        self.config_rx
            .get()
            .map(|rx| rx.borrow().clone())
            .unwrap_or_default()
    }

    /// The live sources once the session is attached, the construction-time source before that.
    fn current_sources(&self) -> Arc<server::sources::Sources> {
        match self.sources_rx.get() {
            Some(rx) => rx.borrow().clone(),
            None => Arc::new(server::sources::Sources {
                active: self.source.clone(),
                ..Default::default()
            }),
        }
    }

    fn offline(&self) -> crate::offline::OfflineIndex {
        self.session
            .get()
            .map(|session| session.offline().clone())
            .unwrap_or_default()
    }

    fn invalidate(&self, table: Table) {
        if let Some(session) = self.session.get() {
            session.invalidate(table);
        }
    }

    /// The source library reads run against: the live active source once the
    /// session is attached, the construction-time source before that.
    fn query_source(&self) -> config::Source {
        self.sources_rx
            .get()
            .map(|rx| rx.borrow().active.clone())
            .unwrap_or_else(|| self.source.clone())
    }

    fn scan_roots(config: &config::Settings) -> Vec<(config::Source, Vec<PathBuf>)> {
        config
            .library
            .local_sources
            .iter()
            .map(|source| {
                (
                    config::Source::LocalLibrary(source.id.clone()),
                    source.directories.clone(),
                )
            })
            .collect()
    }

    pub async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError> {
        let offline = self.offline();
        let (total, rows) = self.tracks_raw(filter, page).await?;
        Ok(TrackPage {
            total,
            offset: page.offset,
            items: rows
                .iter()
                .map(|track| crate::wire::track_info(track, &offline))
                .collect(),
        })
    }

    pub(crate) async fn tracks_raw(
        &self,
        filter: TrackFilter,
        page: Page,
    ) -> Result<(u32, Vec<Track>), ApiError> {
        let sort = match filter.sort {
            api::TrackSort::Default if filter.album.is_some() => db::TrackSort::TrackNumber,
            sort => map_sort(sort),
        };
        let db_filter = db::TrackFilter {
            source: self.query_source(),
            sort,
            search: filter.search.unwrap_or_default(),
            album: filter.album,
            genre: filter.genre,
            favorite: filter.favorite,
            downloaded: filter.downloaded,
            year_from: filter.year_from,
            year_to: filter.year_to,
            reverse: filter.reverse,
        };
        let items = self
            .db
            .tracks_page(&db_filter, db_page(page))
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
        let offline = self.offline();
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
            .map(|track| crate::wire::track_info(track, &offline))
            .collect();
        Ok(api::TrackPage {
            total,
            offset: page.offset,
            items,
        })
    }

    pub async fn stats(&self) -> Result<api::StatsView, ApiError> {
        let own = self.query_source().listen_count_key("");
        let listen_counts = self
            .db
            .listen_counts()
            .await
            .map_err(db_error)?
            .into_iter()
            .filter_map(|(key, count)| {
                if own.is_empty() {
                    (!key.starts_with("local:")).then_some((key, count))
                } else {
                    key.strip_prefix(&own).map(|uid| (uid.to_owned(), count))
                }
            })
            .collect();
        Ok(api::StatsView { listen_counts })
    }

    /// Lyrics for one library track, through the app's full provider chain
    /// (local .lrc, server lyrics API, synced fallbacks, lrclib) with its
    /// process cache. Radio has no lyrics by construction.
    pub async fn lyrics(&self, key: &str) -> Result<api::LyricsView, ApiError> {
        let config = self.current_config();
        let sources = self.current_sources();
        let track = match self
            .db
            .tracks_by_keys(&sources.active, &[key.to_string()])
            .await
            .map_err(db_error)?
            .into_iter()
            .next()
        {
            Some(track) => track,
            // A row played straight from a browse listing has no database
            // entry, and is exactly the one someone wants the words to.
            None => self
                .transient_track(key)
                .ok_or_else(|| ApiError::not_found("unknown track key"))?,
        };
        if track.duration == u64::MAX {
            return Err(ApiError::invalid_input("radio streams have no lyrics"));
        }

        let mut request = server::lyrics::LyricsRequest::new(
            &track.artist,
            &track.title,
            &track.album,
            track.duration,
            track.id.uid(),
        )
        .prefer_local(config.lyrics.prefer_local_lyrics)
        .enable_musixmatch(config.lyrics.enable_musixmatch_lyrics);
        if let Some(server) = &sources.server {
            request = request.with_server(
                Some(&server.url),
                server.access_token.as_deref(),
                server.user_id.as_deref(),
            );
            // Apple Music's own words need the account's token and a bearer
            // fetched for the session -- both credentials, so both are here.
            if server.service == config::MusicService::AppleMusic
                && let Some(token) = server.access_token.clone()
                && let Some(catalog_id) = key.strip_prefix("applemusic:")
            {
                let bearer_token = server::applemusic::auth::get_bearer_token()
                    .await
                    .unwrap_or_default();
                request = request.apple_music_auth(server::lyrics::AppleMusicLyricsAuth {
                    token,
                    bearer_token,
                    storefront: server.apple_music_storefront.clone(),
                    language: server.apple_music_language.clone(),
                    catalog_id: catalog_id.to_string(),
                });
            }
            if let Some(auth) = youtube_music_lyrics_auth(server) {
                request = request.youtube_music_auth(auth);
            }
        }

        // Three layers, cheapest first: this process's cache, the library's
        // stored answer, then the providers -- whose answer is stored so the
        // next open, in any frontend, skips the network.
        let cache_key = request.cache_key();
        let lyrics = match server::lyrics::cached_lyrics_for_request(&request) {
            Some(cached) => cached,
            None => match self.persisted_lyrics(&cache_key).await {
                Some(persisted) => {
                    server::lyrics::prime(&cache_key, persisted.clone());
                    persisted
                }
                None => {
                    let fetched = server::lyrics::fetch_lyrics_for_request(&request).await;
                    if fetched.conclusive {
                        self.persist_lyrics(&cache_key, &fetched.lyrics).await;
                    }
                    fetched.lyrics
                }
            },
        };
        lyrics
            .map(lyrics_view)
            .ok_or_else(|| ApiError::not_found("no lyrics found"))
    }

    /// Synthetic radio track, seeded from the manifest so no client ever sees
    /// raw ids while the first metadata update is in flight. The `u64::MAX`
    /// duration sentinel is translated to `TrackKind::Radio` at the wire.
    fn radio_track(&self, station_id: &str, stream_id: &str) -> Track {
        let registry = match self.station_registry.read() {
            Ok(registry) => registry.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
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
        let cover = station.and_then(|station| match &station.metadata {
            Some(radio::manifest::MetadataSourceDef::Static(meta)) => {
                meta.resolve(stream_id).2.map(str::to_string)
            }
            _ => None,
        });

        Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(format!(
                "radio:{station_id}:{stream_id}"
            ))),
            cover,
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
            credits: Vec::new(),
            artists: vec![],
            replay_gain: config::ReplayGainInfo::default(),
            explicit: false,
            plays: None,
            counterpart: None,
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

mod artist_art;
mod jobs;
mod lyrics_cache;
mod reads;

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
                // A key the database does not hold is either a row from a
                // live listing this session saw, or a file on disk.
                let missing: Vec<String> = keys
                    .iter()
                    .filter(|key| !by_key.contains_key(*key))
                    .cloned()
                    .collect();
                let mut on_disk = Vec::new();
                for key in missing {
                    match self.transient_track(&key) {
                        Some(track) => {
                            by_key.insert(key, track);
                        }
                        None => on_disk.push(key),
                    }
                }
                for track in Self::probe_local_files(on_disk, self.cover_cache.clone()).await {
                    by_key.insert(track.id.key().to_string(), track);
                }
                // Anything left was never listed this session; only the source can name it.
                let unresolved: Vec<&String> = keys
                    .iter()
                    .filter(|key| !by_key.contains_key(*key))
                    .collect();
                let mut unknown = Vec::new();
                for key in unresolved {
                    let found = match self.catalog_service() {
                        Ok(catalog) => catalog.track(key).await?,
                        Err(_) => None,
                    };
                    match found {
                        Some(track) => {
                            by_key.insert(key.clone(), track);
                        }
                        None => unknown.push(key.as_str()),
                    }
                }
                if !unknown.is_empty() {
                    return Err(ApiError::not_found(format!(
                        "no track has the key {}",
                        unknown.join(", ")
                    )));
                }
                Ok(keys.iter().filter_map(|key| by_key.remove(key)).collect())
            }
            QueueContext::Album { id } => {
                let tracks = self
                    .db
                    .album_tracks(&self.query_source(), id)
                    .await
                    .map_err(db_error)?;
                if !tracks.is_empty() {
                    return Ok(tracks);
                }
                let catalog = self.catalog_service().map_err(|_| no_such("album"))?;
                catalog.album_tracks(id).await.map_err(missing_as("album"))
            }
            QueueContext::Artist { artist } => {
                let row = self.artist_row(artist).await?;
                self.db
                    .artist_tracks(&self.query_source(), &row.key, None)
                    .await
                    .map_err(db_error)
            }
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
                // A playlist the library does not hold is one the source lists.
                let Some(playlist) = store.playlists.iter().find(|playlist| playlist.id == *id)
                else {
                    let catalog = self.catalog_service().map_err(|_| no_such("playlist"))?;
                    return catalog
                        .playlist_tracks(id)
                        .await
                        .map_err(missing_as("playlist"));
                };
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
            } => {
                // A station that came from the public directory gets its play
                // reported back to it, which is how that directory ranks.
                if stream_id == radio::browser::BROWSER_STREAM_ID {
                    radio::browser::count_click(station_id);
                }
                Ok(vec![self.radio_track(station_id, stream_id)])
            }
            QueueContext::TrackRadio { .. } | QueueContext::PlaylistRadio { .. } => {
                Ok(self.materialize_queue(context).await?.tracks)
            }
        }
    }

    async fn materialize_queue(&self, context: &QueueContext) -> Result<RadioPage, ApiError> {
        match context {
            QueueContext::TrackRadio { key } => self.catalog_service()?.track_radio(key).await,
            QueueContext::PlaylistRadio { id } => self.catalog_service()?.playlist_radio(id).await,
            _ => Ok(RadioPage::last(self.materialize(context).await?)),
        }
    }

    async fn more_radio(&self, cursor: &str) -> Result<RadioPage, ApiError> {
        self.catalog_service()?.more_radio(cursor).await
    }

    fn register_restored(&self, tracks: &[Track]) {
        self.register_transient(tracks);
    }
}

fn youtube_music_lyrics_auth(
    server: &config::MusicServer,
) -> Option<server::lyrics::YouTubeMusicLyricsAuth> {
    if server.service != config::MusicService::YtMusic || server.yt_anonymous {
        return None;
    }
    let cookies = server.access_token.clone().filter(|c| !c.is_empty())?;
    Some(server::lyrics::YouTubeMusicLyricsAuth {
        cookies,
        account: server.account.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_music_lyrics_carry_the_chosen_brand_account() {
        let mut server = config::MusicServer::new_with_service(
            "YouTube Music".into(),
            String::new(),
            config::MusicService::YtMusic,
        );
        server.access_token = Some("SAPISID=abc".into());
        server.account = Some("4242".into());
        let auth = youtube_music_lyrics_auth(&server).expect("signed in");
        assert_eq!(auth.cookies, "SAPISID=abc");
        assert_eq!(auth.account.as_deref(), Some("4242"));

        server.account = None;
        let own = youtube_music_lyrics_auth(&server).expect("signed in");
        assert_eq!(own.account, None);

        server.access_token = Some(String::new());
        assert_eq!(youtube_music_lyrics_auth(&server), None);

        let mut jellyfin = server.clone();
        jellyfin.service = config::MusicService::Jellyfin;
        jellyfin.access_token = Some("token".into());
        assert_eq!(youtube_music_lyrics_auth(&jellyfin), None);
    }

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
            credits: Vec::new(),
            artists: vec![],
            replay_gain: config::ReplayGainInfo::default(),
            explicit: false,
            plays: None,
            counterpart: None,
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
        let credentials = crate::CredentialStore::load(&database)
            .await
            .expect("credentials");
        let service = LibraryService::new(
            database,
            source,
            Arc::new(radio::registry::StationRegistry::default()),
            cover_cache,
            credentials,
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

        let ada = library
            .artists(api::ArtistQuery::default(), Page::default())
            .await
            .expect("artist grid")
            .artists
            .into_iter()
            .find(|artist| artist.name == "Ada")
            .expect("Ada is listed");
        let page = library
            .artist_tracks(&ada.key, Page::default())
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
                keys: vec!["/lib/2.flac".into(), "/lib/0.flac".into()],
            })
            .await
            .expect("keys context");
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "song 2");
        assert_eq!(tracks[1].title, "song 0");

        let missing = library
            .materialize(&QueueContext::Album { id: "ghost".into() })
            .await
            .expect_err("unknown album");
        assert_eq!(missing.code, api::ErrorCode::NotFound);

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

    /// A row from a live listing is not in the database and is not a file on
    /// disk, so without the transient cache it would silently vanish from a
    /// queue built by key -- which is every queue the API can build.
    #[tokio::test]
    async fn materialize_resolves_a_track_the_database_has_never_seen() {
        let (_dir, library) = seeded_library().await;
        let remote = Track {
            id: reader::TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: "vid-1".into(),
            },
            cover: Some("https://example.com/art.jpg".into()),
            album_id: String::new(),
            title: "from the catalog".into(),
            artist: "Someone".into(),
            album: String::new(),
            duration: 200,
            khz: 44,
            bitrate: 128,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            credits: Vec::new(),
            artists: vec![],
            replay_gain: config::ReplayGainInfo::default(),
            explicit: false,
            plays: None,
            counterpart: None,
        };

        let unregistered = library
            .materialize(&QueueContext::Tracks {
                keys: vec!["vid-1".into()],
            })
            .await
            .expect_err("nothing can name an unregistered key without a catalog");
        assert_eq!(unregistered.code, api::ErrorCode::NotFound);

        library.register_transient(std::slice::from_ref(&remote));
        let tracks = library
            .materialize(&QueueContext::Tracks {
                keys: vec!["vid-1".into(), "/lib/0.flac".into()],
            })
            .await
            .expect("keys context");
        assert_eq!(tracks.len(), 2, "the catalog row joins the library one");
        assert_eq!(tracks[0].title, "from the catalog");
        assert_eq!(tracks[1].title, "song 0");
    }

    /// Skip explicit filters a queue built from stored rows, not only fetched ones.
    #[tokio::test]
    async fn a_stored_explicit_track_is_skipped_when_queued_from_the_library() {
        let (_dir, library) = seeded_library().await;
        let mut explicit = track(9, "Ada");
        explicit.explicit = true;
        library
            .db
            .upsert_tracks(&config::Source::default(), std::slice::from_ref(&explicit))
            .await
            .expect("store the explicit track");

        let tracks = library
            .materialize(&QueueContext::Tracks {
                keys: vec!["/lib/0.flac".into(), "/lib/9.flac".into()],
            })
            .await
            .expect("keys context");
        assert_eq!(
            tracks.iter().map(|t| t.explicit).collect::<Vec<_>>(),
            [false, true],
            "the flag survives the database"
        );
        let (kept, _) = crate::queue_model::without_explicit(tracks, None);
        assert_eq!(
            kept.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["song 0"]
        );
    }

    fn remote(id: &str) -> Track {
        Track {
            id: reader::TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: id.into(),
            },
            title: format!("remote {id}"),
            ..track(0, "Someone")
        }
    }

    /// Holds one album, playlist and track the library has never seen, and nothing else.
    struct FakeCatalog;

    #[async_trait::async_trait]
    impl crate::catalog::CatalogQueue for FakeCatalog {
        async fn album_tracks(&self, id: &str) -> Result<Vec<Track>, ApiError> {
            match id {
                "MPREalbum" => Ok(vec![remote("a1"), remote("a2")]),
                _ => Err(ApiError::unsupported("album by ref")),
            }
        }

        async fn playlist_tracks(&self, id: &str) -> Result<Vec<Track>, ApiError> {
            match id {
                "PLcommunity" => Ok(vec![remote("p1"), remote("p2"), remote("p3")]),
                _ => Err(ApiError::not_found("that playlist has no tracks to play")),
            }
        }

        async fn track(&self, key: &str) -> Result<Option<Track>, ApiError> {
            Ok((key == "unseen").then(|| remote("unseen")))
        }

        async fn track_radio(&self, _key: &str) -> Result<RadioPage, ApiError> {
            Err(ApiError::unsupported("radio"))
        }

        async fn playlist_radio(&self, _id: &str) -> Result<RadioPage, ApiError> {
            Err(ApiError::unsupported("radio"))
        }

        async fn more_radio(&self, _cursor: &str) -> Result<RadioPage, ApiError> {
            Err(ApiError::unsupported("radio"))
        }
    }

    async fn library_with_catalog() -> (tempfile::TempDir, LibraryService) {
        let (dir, library) = seeded_library().await;
        library.attach_catalog(Arc::new(FakeCatalog));
        (dir, library)
    }

    fn titles(tracks: &[Track]) -> Vec<&str> {
        tracks.iter().map(|track| track.title.as_str()).collect()
    }

    #[tokio::test]
    async fn a_catalog_album_plays_by_its_id() {
        let (_dir, library) = library_with_catalog().await;

        let tracks = library
            .materialize(&QueueContext::Album {
                id: "MPREalbum".into(),
            })
            .await
            .expect("album the library does not hold");
        assert_eq!(titles(&tracks), ["remote a1", "remote a2"]);

        let held = library
            .materialize(&QueueContext::Album {
                id: "album-1".into(),
            })
            .await
            .expect("album the library holds");
        assert_eq!(held.len(), 2, "the library's own album is not looked up");

        let missing = library
            .materialize(&QueueContext::Album { id: "nope".into() })
            .await
            .expect_err("a source that cannot look it up has no such album");
        assert_eq!(missing.code, api::ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn an_unsaved_playlist_plays_by_its_id() {
        let (_dir, library) = library_with_catalog().await;

        let tracks = library
            .materialize(&QueueContext::Playlist {
                id: "PLcommunity".into(),
            })
            .await
            .expect("playlist the library does not hold");
        assert_eq!(titles(&tracks), ["remote p1", "remote p2", "remote p3"]);

        let missing = library
            .materialize(&QueueContext::Playlist { id: "ghost".into() })
            .await
            .expect_err("unknown playlist");
        assert_eq!(missing.code, api::ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn an_unseen_key_is_looked_up_or_refused() {
        let (_dir, library) = library_with_catalog().await;

        let tracks = library
            .materialize(&QueueContext::Tracks {
                keys: vec!["unseen".into(), "/lib/0.flac".into()],
            })
            .await
            .expect("the source names the unseen key");
        assert_eq!(titles(&tracks), ["remote unseen", "song 0"]);

        let refused = library
            .materialize(&QueueContext::Tracks {
                keys: vec!["/lib/0.flac".into(), "nobody".into()],
            })
            .await
            .expect_err("a key no one can name");
        assert_eq!(refused.code, api::ErrorCode::NotFound);
        assert!(refused.message.contains("nobody"), "{}", refused.message);
    }
}
