use percent_encoding::NON_ALPHANUMERIC;
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
struct LrcLibResponse {
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricChunk {
    /// A timed lyric chunk. Most providers use whole words; Apple Music can
    /// return smaller syllable-level chunks.
    pub start_time: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub start_time: f64,
    pub end_time: Option<f64>,
    pub text: String,
    pub chunks: Vec<LyricChunk>,
    pub parent_line_index: Option<usize>,
    pub background: bool,
    pub opposite_turn: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lyrics {
    Synced(Vec<LyricLine>),
    Plain(String),
}

const LYRICS_CACHE_CAPACITY: usize = 256;
const MUSIXMATCH_ROOT_URL: &str = "https://apic-desktop.musixmatch.com/ws/1.1/";
const PAXSENIX_ROOT_URL: &str = "https://lyrics.paxsenix.org";
const ITUNES_SEARCH_ROOT_URL: &str = "https://itunes.apple.com/search";
const MUSIXMATCH_TIMEOUT: Duration = Duration::from_secs(3);
const PAXSENIX_TIMEOUT: Duration = Duration::from_secs(5);
const PAXSENIX_APPLE_LYRICS_TIMEOUT: Duration = Duration::from_secs(10);
const PAXSENIX_YOUTUBE_LYRICS_TIMEOUT: Duration = Duration::from_secs(3);
const LRCLIB_TIMEOUT: Duration = Duration::from_secs(5);
const SERVER_LYRICS_TIMEOUT: Duration = Duration::from_secs(5);
const LYRICS_INFLIGHT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const LYRICS_INFLIGHT_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

static LYRICS_CACHE: OnceLock<Mutex<LyricsCache>> = OnceLock::new();
static LYRICS_INFLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static MUSIXMATCH_TOKEN: OnceLock<Mutex<Option<MusixmatchToken>>> = OnceLock::new();

macro_rules! lyrics_debug {
    ($($arg:tt)*) => {
        if lyrics_terminal_debug_enabled() {
            eprintln!("[lyrics] {}", format_args!($($arg)*));
        }
    };
}

struct LyricsCache {
    entries: HashMap<String, Option<Lyrics>>,
    order: VecDeque<String>,
    capacity: usize,
}

struct LyricsInflightGuard {
    key: String,
}

struct MusixmatchToken {
    value: String,
    expires_at_ms: u128,
}

impl Drop for LyricsInflightGuard {
    fn drop(&mut self) {
        if let Ok(mut inflight) = lyrics_inflight().lock() {
            inflight.remove(&self.key);
        }
    }
}

impl LyricsCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    fn get_cloned(&mut self, key: &str) -> Option<Option<Lyrics>> {
        let value = self.entries.get(key).cloned()?;
        self.touch(key);
        Some(value)
    }

    fn put(&mut self, key: String, value: Option<Lyrics>) {
        if self.entries.contains_key(&key) {
            self.entries.insert(key.clone(), value);
            self.touch(&key);
            return;
        }

        if self.entries.len() >= self.capacity {
            while let Some(oldest) = self.order.pop_front() {
                if self.entries.remove(&oldest).is_some() {
                    break;
                }
            }
        }

        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|existing| existing == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key.to_string());
    }
}

// --- Jellyfin lyrics types ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct JellyfinLyricsResponse {
    #[serde(default)]
    lyrics: Vec<JellyfinLyricLine>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct JellyfinLyricLine {
    text: String,
    start: Option<i64>,
}

// --- Subsonic lyrics types (getLyricsBySongId - OpenSubsonic extension) ---

#[derive(Debug, Deserialize)]
struct SubsonicSongLyricsEnvelope {
    #[serde(rename = "subsonic-response")]
    response: SubsonicSongLyricsResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubsonicSongLyricsResponse {
    status: String,
    #[serde(default)]
    lyrics_list: Option<SubsonicLyricsList>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubsonicLyricsList {
    #[serde(default)]
    structured_lyrics: Vec<SubsonicStructuredLyric>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubsonicStructuredLyric {
    synced: bool,
    #[serde(default)]
    line: Vec<SubsonicLyricLine>,
}

#[derive(Debug, Deserialize)]
struct SubsonicLyricLine {
    #[serde(default)]
    start: Option<i64>,
    value: String,
}

// --- Subsonic lyrics types (getLyrics - basic endpoint) ---

#[derive(Debug, Deserialize)]
struct SubsonicPlainLyricsEnvelope {
    #[serde(rename = "subsonic-response")]
    response: SubsonicPlainLyricsResponse,
}

#[derive(Debug, Deserialize)]
struct SubsonicPlainLyricsResponse {
    status: String,
    #[serde(default)]
    lyrics: Option<SubsonicPlainLyricsData>,
}

#[derive(Debug, Deserialize)]
struct SubsonicPlainLyricsData {
    #[serde(default)]
    value: String,
}

// --- Apple Music lyrics types ---

#[derive(Debug, Deserialize)]
struct ItunesSearchResponse {
    #[serde(default)]
    results: Vec<ItunesSong>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItunesSong {
    track_id: u64,
    track_name: String,
    artist_name: String,
    #[serde(default)]
    track_time_millis: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PaxsenixAppleLyricsResponse {
    #[serde(default)]
    content: Vec<PaxsenixAppleLyricLine>,
    #[serde(default)]
    lrc: Option<String>,
    #[serde(default)]
    plain: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PaxsenixAppleLyricLine {
    #[serde(default)]
    text: Vec<PaxsenixAppleLyricPart>,
    #[serde(default, rename = "backgroundText")]
    background_text: Vec<PaxsenixAppleLyricPart>,
    timestamp: u64,
    #[serde(default)]
    endtime: Option<u64>,
    #[serde(default)]
    background: bool,
    #[serde(default, rename = "oppositeTurn")]
    opposite_turn: bool,
}

#[derive(Debug, Deserialize)]
struct PaxsenixAppleLyricPart {
    text: String,
    #[serde(default)]
    timestamp: Option<u64>,
    #[serde(default)]
    part: bool,
}

// --- YouTube lyrics types ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PaxsenixYoutubeSearchResult {
    video_id: String,
    title: String,
    author: String,
    duration: String,
}

// --- Public API ---

/// Fetch lyrics for a track, trying sources in priority order while preferring
/// word-timed lyrics over line-only matches:
/// 1. Local .lrc file alongside the audio file (local tracks only)
/// 2. Jellyfin or Subsonic server lyrics API (server tracks)
/// 3. Paxsenix Apple Music lyrics (syllable/line synced fallback for all tracks)
/// 4. Paxsenix YouTube lyrics (direct video-id LRC for YouTube Music tracks)
/// 5. Optional Musixmatch richsync fallback
/// 6. lrclib.net (fallback for all tracks)
///
/// For Jellyfin: `server_token` = access token, `server_user_id` = user_id (unused for lyrics)
/// For Subsonic: `server_token` = password, `server_user_id` = username
// skip_all, not skip(track_path): a bare skip auto-records every other arg
// as a span field, which would leak server_token (and url/user_id) into the
// trace + log. Record only artist/title, explicitly.
#[tracing::instrument(name = "lyrics.fetch", skip_all, fields(artist = %artist, title = %title))]
#[allow(clippy::too_many_arguments)]
pub async fn fetch_lyrics(
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
    track_path: &str,
    server_url: Option<&str>,
    server_token: Option<&str>,
    server_user_id: Option<&str>,
    prefer_local: bool,
    enable_musixmatch: bool,
) -> Option<Lyrics> {
    fetch_lyrics_with_progress(
        artist,
        title,
        album,
        duration,
        track_path,
        server_url,
        server_token,
        server_user_id,
        prefer_local,
        enable_musixmatch,
        true,
        |_| {},
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn fetch_lyrics_progressive<F>(
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
    track_path: &str,
    server_url: Option<&str>,
    server_token: Option<&str>,
    server_user_id: Option<&str>,
    prefer_local: bool,
    enable_musixmatch: bool,
    on_progress: F,
) -> Option<Lyrics>
where
    F: FnMut(Lyrics),
{
    fetch_lyrics_with_progress(
        artist,
        title,
        album,
        duration,
        track_path,
        server_url,
        server_token,
        server_user_id,
        prefer_local,
        enable_musixmatch,
        true,
        on_progress,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn fetch_lyrics_with_progress<F>(
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
    track_path: &str,
    server_url: Option<&str>,
    server_token: Option<&str>,
    server_user_id: Option<&str>,
    prefer_local: bool,
    enable_musixmatch: bool,
    allow_lrclib: bool,
    mut on_progress: F,
) -> Option<Lyrics>
where
    F: FnMut(Lyrics),
{
    let cache_key = lyrics_cache_key(
        artist,
        title,
        album,
        duration,
        track_path,
        enable_musixmatch,
    );
    let cache_key_hash = log_lyrics_key_hash(&cache_key);
    let total_start = Instant::now();
    lyrics_debug!(
        "fetch start key_hash={} artist={:?} title={:?} duration={} prefer_local={}",
        cache_key_hash,
        artist,
        title,
        duration,
        prefer_local
    );
    if let Some(cached) = lyrics_cache()
        .lock()
        .ok()
        .and_then(|mut cache| cache.get_cloned(&cache_key))
    {
        tracing::info!(
            target: "kopuz::lyrics",
            "cache hit key_hash={} kind={}",
            log_lyrics_key_hash(&cache_key),
            lyrics_kind(cached.as_ref())
        );
        lyrics_debug!(
            "cache hit key_hash={} kind={}",
            cache_key_hash,
            lyrics_kind(cached.as_ref())
        );
        return cached;
    }

    let _inflight_guard = if try_begin_lyrics_fetch(&cache_key) {
        lyrics_debug!("inflight acquired key_hash={}", cache_key_hash);
        Some(LyricsInflightGuard {
            key: cache_key.clone(),
        })
    } else {
        lyrics_debug!(
            "inflight wait key_hash={} timeout_ms={}",
            cache_key_hash,
            LYRICS_INFLIGHT_WAIT_TIMEOUT.as_millis()
        );
        let wait_start = Instant::now();
        while wait_start.elapsed() < LYRICS_INFLIGHT_WAIT_TIMEOUT {
            crate::sleep(LYRICS_INFLIGHT_POLL_INTERVAL).await;
            if let Some(cached) = lyrics_cache()
                .lock()
                .ok()
                .and_then(|mut cache| cache.get_cloned(&cache_key))
            {
                lyrics_debug!(
                    "inflight resolved from cache key_hash={} elapsed_ms={} kind={}",
                    cache_key_hash,
                    wait_start.elapsed().as_millis(),
                    lyrics_kind(cached.as_ref())
                );
                return cached;
            }
        }

        if try_begin_lyrics_fetch(&cache_key) {
            lyrics_debug!(
                "inflight timed out, acquired retry key_hash={} waited_ms={}",
                cache_key_hash,
                wait_start.elapsed().as_millis()
            );
            Some(LyricsInflightGuard {
                key: cache_key.clone(),
            })
        } else {
            if let Some(cached) = lyrics_cache()
                .lock()
                .ok()
                .and_then(|mut cache| cache.get_cloned(&cache_key))
            {
                lyrics_debug!(
                    "inflight final cache hit key_hash={} kind={}",
                    cache_key_hash,
                    lyrics_kind(cached.as_ref())
                );
                return cached;
            }
            lyrics_debug!(
                "inflight unresolved key_hash={} waited_ms={}",
                cache_key_hash,
                wait_start.elapsed().as_millis()
            );
            return None;
        }
    };

    let is_server = track_path.starts_with("jellyfin:")
        || track_path.starts_with("subsonic:")
        || track_path.starts_with("custom:");
    let mut fallback: Option<Lyrics> = None;

    // 1. Local .lrc file (only for local tracks)
    if !is_server {
        let started = Instant::now();
        let local = fetch_local_lrc(track_path).await;
        tracing::info!(
            target: "kopuz::lyrics",
            "local_lrc key_hash={} elapsed_ms={} kind={}",
            log_lyrics_key_hash(&cache_key),
            started.elapsed().as_millis(),
            lyrics_kind(local.as_ref())
        );
        lyrics_debug!(
            "provider=local_lrc elapsed_ms={} kind={}",
            started.elapsed().as_millis(),
            lyrics_kind(local.as_ref())
        );
        if let Some(lyrics) = local {
            if has_word_timestamps(&lyrics) {
                if let Ok(mut cache) = lyrics_cache().lock() {
                    cache.put(cache_key.clone(), Some(lyrics.clone()));
                }
                tracing::info!(
                    target: "kopuz::lyrics",
                    "selected key_hash={} source=local_lrc kind={} total_ms={}",
                    log_lyrics_key_hash(&cache_key),
                    lyrics_kind(Some(&lyrics)),
                    total_start.elapsed().as_millis()
                );
                lyrics_debug!(
                    "selected source=local_lrc key_hash={} kind={} total_ms={}",
                    cache_key_hash,
                    lyrics_kind(Some(&lyrics)),
                    total_start.elapsed().as_millis()
                );
                return Some(lyrics);
            }
            fallback = Some(lyrics);
        }
    }

    if prefer_local && !is_server {
        if let Ok(mut cache) = lyrics_cache().lock() {
            cache.put(cache_key.clone(), fallback.clone());
        }
        tracing::info!(
            target: "kopuz::lyrics",
            "selected key_hash={} source=prefer_local kind={} total_ms={}",
            log_lyrics_key_hash(&cache_key),
            lyrics_kind(fallback.as_ref()),
            total_start.elapsed().as_millis()
        );
        lyrics_debug!(
            "selected source=prefer_local key_hash={} kind={} total_ms={}",
            cache_key_hash,
            lyrics_kind(fallback.as_ref()),
            total_start.elapsed().as_millis()
        );
        return fallback;
    }

    // 2. Server lyrics
    if let Some(server_url) = server_url {
        if track_path.starts_with("jellyfin:") {
            if let (Some(item_id), Some(token)) =
                (extract_server_id(track_path, "jellyfin:"), server_token)
            {
                let started = Instant::now();
                let server_lyrics = fetch_jellyfin_lyrics(&item_id, server_url, token).await;
                tracing::info!(
                    target: "kopuz::lyrics",
                    "server_jellyfin key_hash={} elapsed_ms={} kind={}",
                    log_lyrics_key_hash(&cache_key),
                    started.elapsed().as_millis(),
                    lyrics_kind(server_lyrics.as_ref())
                );
                lyrics_debug!(
                    "provider=jellyfin elapsed_ms={} kind={}",
                    started.elapsed().as_millis(),
                    lyrics_kind(server_lyrics.as_ref())
                );
                if let Some(lyrics) = server_lyrics {
                    if has_word_timestamps(&lyrics) {
                        if let Ok(mut cache) = lyrics_cache().lock() {
                            cache.put(cache_key.clone(), Some(lyrics.clone()));
                        }
                        tracing::info!(
                            target: "kopuz::lyrics",
                            "selected key_hash={} source=jellyfin kind={} total_ms={}",
                            log_lyrics_key_hash(&cache_key),
                            lyrics_kind(Some(&lyrics)),
                            total_start.elapsed().as_millis()
                        );
                        lyrics_debug!(
                            "selected source=jellyfin key_hash={} kind={} total_ms={}",
                            cache_key_hash,
                            lyrics_kind(Some(&lyrics)),
                            total_start.elapsed().as_millis()
                        );
                        return Some(lyrics);
                    }
                    fallback.get_or_insert(lyrics);
                }
            }
        } else if track_path.starts_with("subsonic:") || track_path.starts_with("custom:") {
            let prefix = if track_path.starts_with("subsonic:") {
                "subsonic:"
            } else {
                "custom:"
            };
            if let (Some(song_id), Some(username), Some(password)) = (
                extract_server_id(track_path, prefix),
                server_user_id,
                server_token,
            ) {
                let started = Instant::now();
                let server_lyrics =
                    fetch_subsonic_lyrics(&song_id, server_url, username, password, artist, title)
                        .await;
                tracing::info!(
                    target: "kopuz::lyrics",
                    "server_subsonic key_hash={} elapsed_ms={} kind={}",
                    log_lyrics_key_hash(&cache_key),
                    started.elapsed().as_millis(),
                    lyrics_kind(server_lyrics.as_ref())
                );
                lyrics_debug!(
                    "provider=subsonic elapsed_ms={} kind={}",
                    started.elapsed().as_millis(),
                    lyrics_kind(server_lyrics.as_ref())
                );
                if let Some(lyrics) = server_lyrics {
                    if has_word_timestamps(&lyrics) {
                        if let Ok(mut cache) = lyrics_cache().lock() {
                            cache.put(cache_key.clone(), Some(lyrics.clone()));
                        }
                        tracing::info!(
                            target: "kopuz::lyrics",
                            "selected key_hash={} source=subsonic kind={} total_ms={}",
                            log_lyrics_key_hash(&cache_key),
                            lyrics_kind(Some(&lyrics)),
                            total_start.elapsed().as_millis()
                        );
                        lyrics_debug!(
                            "selected source=subsonic key_hash={} kind={} total_ms={}",
                            cache_key_hash,
                            lyrics_kind(Some(&lyrics)),
                            total_start.elapsed().as_millis()
                        );
                        return Some(lyrics);
                    }
                    fallback.get_or_insert(lyrics);
                }
            }
        }
    }

    // 3/4/5. Apple Music and direct YouTube lyrics are the primary remote
    // providers. Optional Musixmatch starts with them, but can only replace
    // the primary result when it returns strictly better timing quality.
    let apple_started = Instant::now();
    let youtube_started = Instant::now();
    let musixmatch_started = Instant::now();
    let lrclib_started = Instant::now();
    let apple = fetch_from_paxsenix_apple_music(artist, title, duration);
    let youtube = fetch_from_paxsenix_youtube(artist, title, duration, track_path);
    let musixmatch = fetch_from_musixmatch_enhanced(artist, title);
    let lrclib = fetch_from_lrclib(artist, title, album, duration);
    tokio::pin!(apple);
    tokio::pin!(youtube);
    tokio::pin!(musixmatch);
    tokio::pin!(lrclib);

    let mut apple_done = false;
    let mut youtube_done = false;
    let mut musixmatch_done = !enable_musixmatch;
    let mut lrclib_done = !allow_lrclib;
    let mut progressed: Option<Lyrics> = None;
    let mut primary_quality = 0;
    let mut musixmatch_candidate: Option<Lyrics> = None;
    if !enable_musixmatch {
        lyrics_debug!("provider=musixmatch skipped reason=disabled");
    }
    if !allow_lrclib {
        lyrics_debug!("provider=lrclib skipped reason=disabled");
    }

    while !apple_done
        || !youtube_done
        || (!musixmatch_done && primary_quality < 2)
        || (!lrclib_done
            && lyrics_quality_option(fallback.as_ref())
                .max(lyrics_quality_option(musixmatch_candidate.as_ref()))
                < 1)
    {
        tokio::select! {
            result = &mut apple, if !apple_done => {
                apple_done = true;
                tracing::info!(
                    target: "kopuz::lyrics",
                    "paxsenix_apple key_hash={} elapsed_ms={} kind={}",
                    log_lyrics_key_hash(&cache_key),
                    apple_started.elapsed().as_millis(),
                    lyrics_kind(result.as_ref())
                );
                lyrics_debug!(
                    "provider=paxsenix_apple elapsed_ms={} kind={}",
                    apple_started.elapsed().as_millis(),
                    lyrics_kind(result.as_ref())
                );
                if let Some(lyrics) = result {
                    let should_replace = fallback
                        .as_ref()
                        .map(|current| lyrics_quality(&lyrics) >= lyrics_quality(current))
                        .unwrap_or(true);
                    if should_replace {
                        fallback = Some(lyrics.clone());
                    }
                    primary_quality = primary_quality.max(lyrics_quality(&lyrics));
                    let should_progress = progressed
                        .as_ref()
                        .map(|current| lyrics_quality(&lyrics) >= lyrics_quality(current))
                        .unwrap_or(true);
                    if should_progress {
                        progressed = Some(lyrics.clone());
                        on_progress(lyrics);
                    }
                }
            }
            result = &mut youtube, if !youtube_done => {
                youtube_done = true;
                tracing::info!(
                    target: "kopuz::lyrics",
                    "paxsenix_youtube key_hash={} elapsed_ms={} kind={}",
                    log_lyrics_key_hash(&cache_key),
                    youtube_started.elapsed().as_millis(),
                    lyrics_kind(result.as_ref())
                );
                lyrics_debug!(
                    "provider=paxsenix_youtube elapsed_ms={} kind={}",
                    youtube_started.elapsed().as_millis(),
                    lyrics_kind(result.as_ref())
                );
                if let Some(lyrics) = result {
                    let should_replace = fallback
                        .as_ref()
                        .map(|current| lyrics_quality(&lyrics) > lyrics_quality(current))
                        .unwrap_or(true);
                    if should_replace {
                        fallback = Some(lyrics.clone());
                    }
                    primary_quality = primary_quality.max(lyrics_quality(&lyrics));
                    let should_progress = progressed
                        .as_ref()
                        .map(|current| lyrics_quality(&lyrics) > lyrics_quality(current))
                        .unwrap_or(true);
                    if should_progress {
                        progressed = Some(lyrics.clone());
                        on_progress(lyrics);
                    }
                }
            }
            result = &mut musixmatch, if !musixmatch_done && primary_quality < 2 => {
                musixmatch_done = true;
                tracing::info!(
                    target: "kopuz::lyrics",
                    "musixmatch_enhanced key_hash={} elapsed_ms={} kind={}",
                    log_lyrics_key_hash(&cache_key),
                    musixmatch_started.elapsed().as_millis(),
                    lyrics_kind(result.as_ref())
                );
                lyrics_debug!(
                    "provider=musixmatch elapsed_ms={} kind={}",
                    musixmatch_started.elapsed().as_millis(),
                    lyrics_kind(result.as_ref())
                );
                if let Some(lyrics) = result
                {
                    musixmatch_candidate = Some(lyrics);
                }
            }
            result = &mut lrclib, if !lrclib_done && lyrics_quality_option(fallback.as_ref()).max(lyrics_quality_option(musixmatch_candidate.as_ref())) < 1 => {
                lrclib_done = true;
                tracing::info!(
                    target: "kopuz::lyrics",
                    "lrclib key_hash={} elapsed_ms={} kind={}",
                    log_lyrics_key_hash(&cache_key),
                    lrclib_started.elapsed().as_millis(),
                    lyrics_kind(result.as_ref())
                );
                lyrics_debug!(
                    "provider=lrclib elapsed_ms={} kind={}",
                    lrclib_started.elapsed().as_millis(),
                    lyrics_kind(result.as_ref())
                );
                if let Some(lyrics) = result
                    && fallback
                        .as_ref()
                        .map(|current| lyrics_quality(&lyrics) > lyrics_quality(current))
                        .unwrap_or(true)
                {
                    fallback = Some(lyrics.clone());
                    on_progress(lyrics);
                }
            }
        }
    }

    if let Some(lyrics) = musixmatch_candidate
        && fallback
            .as_ref()
            .map(|current| lyrics_quality(&lyrics) > lyrics_quality(current))
            .unwrap_or(true)
    {
        fallback = Some(lyrics.clone());
        on_progress(lyrics);
    }

    if enable_musixmatch && !musixmatch_done {
        lyrics_debug!("provider=musixmatch skipped reason=primary_quality");
    }

    if allow_lrclib && !lrclib_done {
        lyrics_debug!("provider=lrclib skipped reason=current_quality");
    }

    let fetched = fallback;
    if let Ok(mut cache) = lyrics_cache().lock() {
        cache.put(cache_key.clone(), fetched.clone());
    }
    tracing::info!(
        target: "kopuz::lyrics",
        "selected key_hash={} source=final kind={} total_ms={}",
        log_lyrics_key_hash(&cache_key),
        lyrics_kind(fetched.as_ref()),
        total_start.elapsed().as_millis()
    );
    lyrics_debug!(
        "selected source=final key_hash={} kind={} total_ms={}",
        cache_key_hash,
        lyrics_kind(fetched.as_ref()),
        total_start.elapsed().as_millis()
    );
    fetched
}

pub fn cached_lyrics(
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
    track_path: &str,
    enable_musixmatch: bool,
) -> Option<Option<Lyrics>> {
    let cache_key = lyrics_cache_key(
        artist,
        title,
        album,
        duration,
        track_path,
        enable_musixmatch,
    );
    lyrics_cache()
        .lock()
        .ok()
        .and_then(|mut cache| cache.get_cloned(&cache_key))
}

fn has_word_timestamps(lyrics: &Lyrics) -> bool {
    match lyrics {
        Lyrics::Synced(lines) => lines.iter().any(|line| line.chunks.len() > 1),
        Lyrics::Plain(_) => false,
    }
}

fn lyrics_quality(lyrics: &Lyrics) -> u8 {
    match lyrics {
        Lyrics::Synced(lines) if lines.iter().any(|line| line.chunks.len() > 1) => 2,
        Lyrics::Synced(_) => 1,
        Lyrics::Plain(_) => 0,
    }
}

fn lyrics_quality_option(lyrics: Option<&Lyrics>) -> u8 {
    lyrics.map(lyrics_quality).unwrap_or(0)
}

fn lyrics_kind(lyrics: Option<&Lyrics>) -> &'static str {
    match lyrics {
        Some(Lyrics::Synced(lines)) if lines.iter().any(|line| line.chunks.len() > 1) => {
            "synced_word"
        }
        Some(Lyrics::Synced(_)) => "synced_line",
        Some(Lyrics::Plain(_)) => "plain",
        None => "none",
    }
}

fn timed_line_count(lines: &[LyricLine]) -> usize {
    lines.iter().filter(|line| !line.chunks.is_empty()).count()
}

fn timed_part_count(lines: &[LyricLine]) -> usize {
    lines.iter().map(|line| line.chunks.len()).sum()
}

fn has_usable_line_timing(lines: &[LyricLine]) -> bool {
    match lines {
        [] => false,
        [_] => true,
        _ => lines
            .windows(2)
            .any(|pair| pair[1].start_time > pair[0].start_time),
    }
}

fn lrc_has_usable_timing(lrc_text: &str) -> bool {
    let mut timestamps = Vec::new();

    for raw_line in lrc_text.lines() {
        let (line_timestamps, content) = extract_line_timestamps(raw_line);
        if !line_timestamps.is_empty() && !content.trim().is_empty() {
            timestamps.extend(line_timestamps);
            continue;
        }

        let (_, chunks) = parse_enhanced_words(content);
        if !chunks.is_empty() {
            timestamps.extend(chunks.into_iter().map(|chunk| chunk.start_time));
        }
    }

    match timestamps.as_slice() {
        [] => false,
        [_] => true,
        _ => timestamps.windows(2).any(|pair| pair[1] > pair[0]),
    }
}

fn log_lyrics_key_hash(key: &str) -> String {
    let mut hasher = DefaultHasher::new();
    key.trim().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn lyrics_terminal_debug_enabled() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        std::env::var_os("KOPUZ_LYRICS_DEBUG").is_some()
    }
}

fn lyrics_cache() -> &'static Mutex<LyricsCache> {
    LYRICS_CACHE.get_or_init(|| Mutex::new(LyricsCache::new(LYRICS_CACHE_CAPACITY)))
}

fn lyrics_inflight() -> &'static Mutex<HashSet<String>> {
    LYRICS_INFLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

fn try_begin_lyrics_fetch(key: &str) -> bool {
    let Ok(mut inflight) = lyrics_inflight().lock() else {
        return true;
    };

    if inflight.contains(key) {
        false
    } else {
        inflight.insert(key.to_string());
        true
    }
}

fn lyrics_cache_key(
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
    track_path: &str,
    enable_musixmatch: bool,
) -> String {
    let provider_policy = if enable_musixmatch { "mm:on" } else { "mm:off" };
    if !track_path.trim().is_empty() {
        return format!("{}|{}", track_path.trim(), provider_policy);
    }

    format!(
        "{}|{}|{}|{}|{}",
        artist.trim().to_lowercase(),
        title.trim().to_lowercase(),
        album.trim().to_lowercase(),
        duration,
        provider_policy
    )
}

fn extract_server_id(path: &str, prefix: &str) -> Option<String> {
    path.strip_prefix(prefix)
        .and_then(|rest| rest.split(':').next())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
async fn fetch_local_lrc(audio_path: &str) -> Option<Lyrics> {
    let content = read_local_lrc(audio_path).or_else(|| read_embedded_lyrics(audio_path))?;
    if content.trim().is_empty() {
        return None;
    }
    let lines = if lrc_has_usable_timing(&content) {
        parse_lrc(&content)
    } else {
        Vec::new()
    };
    if has_usable_line_timing(&lines) {
        Some(Lyrics::Synced(lines))
    } else {
        Some(Lyrics::Plain(content))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_embedded_lyrics(audio_path: &str) -> Option<String> {
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    use lofty::tag::ItemKey;

    let tagged = Probe::open(audio_path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    tag.get_string(ItemKey::Lyrics)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_local_lrc(audio_path: &str) -> Option<String> {
    use std::path::Path;

    let audio = Path::new(audio_path);

    let stem_lrc = audio.with_extension("lrc");
    if let Ok(content) = std::fs::read_to_string(&stem_lrc) {
        return Some(content);
    }

    let appended = format!("{audio_path}.lrc");
    if let Ok(content) = std::fs::read_to_string(&appended) {
        return Some(content);
    }

    let parent = audio.parent()?;
    let file_name = audio.file_name()?.to_string_lossy().to_lowercase();
    let stem = audio
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase());

    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let path = entry.path();
        if path
            .extension()
            .map(|e| !e.eq_ignore_ascii_case("lrc"))
            .unwrap_or(true)
        {
            continue;
        }
        let Some(cand_name) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
            continue;
        };
        let matches_appended = cand_name == format!("{file_name}.lrc");
        let matches_stem = stem
            .as_ref()
            .map(|s| cand_name == format!("{s}.lrc"))
            .unwrap_or(false);
        if (matches_appended || matches_stem)
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            return Some(content);
        }
    }

    None
}

#[cfg(target_arch = "wasm32")]
async fn fetch_local_lrc(_audio_path: &str) -> Option<Lyrics> {
    None
}

async fn fetch_jellyfin_lyrics(item_id: &str, server_url: &str, token: &str) -> Option<Lyrics> {
    let url = format!(
        "{}/Items/{}/Lyrics",
        server_url.trim_end_matches('/'),
        item_id
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("X-Emby-Token", token)
        .timeout(SERVER_LYRICS_TIMEOUT)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: JellyfinLyricsResponse = resp.json().await.ok()?;
    if data.lyrics.is_empty() {
        return None;
    }

    let has_timestamps = data.lyrics.iter().any(|l| l.start.is_some());
    if has_timestamps {
        let mut lines: Vec<LyricLine> = data
            .lyrics
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .filter_map(|l| {
                l.start.map(|ticks| LyricLine {
                    // Jellyfin uses 100-nanosecond ticks; divide by 10,000,000 to get seconds
                    start_time: ticks as f64 / 10_000_000.0,
                    end_time: None,
                    text: l.text.clone(),
                    chunks: Vec::new(),
                    parent_line_index: None,
                    background: false,
                    opposite_turn: false,
                })
            })
            .collect();
        lines.sort_by(|a, b| {
            a.start_time
                .partial_cmp(&b.start_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if has_usable_line_timing(&lines) {
            return Some(Lyrics::Synced(lines));
        }
    }

    let text: String = data
        .lyrics
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.trim().is_empty() {
        Some(Lyrics::Plain(text))
    } else {
        None
    }
}

async fn fetch_subsonic_lyrics(
    song_id: &str,
    server_url: &str,
    username: &str,
    password: &str,
    artist: &str,
    title: &str,
) -> Option<Lyrics> {
    let base_url = server_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    // Subsonic uses MD5(password+salt) for auth
    let salt = "kopuzlyrics";
    let token = format!("{:x}", md5::compute(format!("{}{}", password, salt)));

    // Try getLyricsBySongId first (OpenSubsonic extension, synced lyrics)
    if let Some(lyrics) =
        subsonic_get_by_id(&client, base_url, username, &token, salt, song_id).await
    {
        return Some(lyrics);
    }

    // Fall back to getLyrics (title+artist, plain text or LRC)
    subsonic_get_by_title(&client, base_url, username, &token, salt, artist, title).await
}

async fn subsonic_get_by_id(
    client: &reqwest::Client,
    base_url: &str,
    username: &str,
    token: &str,
    salt: &str,
    song_id: &str,
) -> Option<Lyrics> {
    let url = format!("{}/rest/getLyricsBySongId.view", base_url);
    let params = [
        ("u", username),
        ("t", token),
        ("s", salt),
        ("v", "1.16.1"),
        ("c", "kopuz"),
        ("f", "json"),
        ("id", song_id),
    ];

    let resp = client
        .get(&url)
        .query(&params)
        .timeout(SERVER_LYRICS_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let data: SubsonicSongLyricsEnvelope = resp.json().await.ok()?;
    if !data.response.status.eq_ignore_ascii_case("ok") {
        return None;
    }

    let structured = data
        .response
        .lyrics_list?
        .structured_lyrics
        .into_iter()
        .next()?;

    if structured.synced {
        let mut lines: Vec<LyricLine> = structured
            .line
            .iter()
            .filter(|l| !l.value.trim().is_empty())
            .filter_map(|l| {
                l.start.map(|ms| LyricLine {
                    start_time: ms as f64 / 1000.0,
                    end_time: None,
                    text: l.value.clone(),
                    chunks: Vec::new(),
                    parent_line_index: None,
                    background: false,
                    opposite_turn: false,
                })
            })
            .collect();
        lines.sort_by(|a, b| {
            a.start_time
                .partial_cmp(&b.start_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if has_usable_line_timing(&lines) {
            return Some(Lyrics::Synced(lines));
        }
    }

    let text: String = structured
        .line
        .iter()
        .map(|l| l.value.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.trim().is_empty() {
        Some(Lyrics::Plain(text))
    } else {
        None
    }
}

async fn subsonic_get_by_title(
    client: &reqwest::Client,
    base_url: &str,
    username: &str,
    token: &str,
    salt: &str,
    artist: &str,
    title: &str,
) -> Option<Lyrics> {
    let url = format!("{}/rest/getLyrics.view", base_url);
    let params = [
        ("u", username),
        ("t", token),
        ("s", salt),
        ("v", "1.16.1"),
        ("c", "kopuz"),
        ("f", "json"),
        ("artist", artist),
        ("title", title),
    ];

    let resp = client
        .get(&url)
        .query(&params)
        .timeout(SERVER_LYRICS_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let data: SubsonicPlainLyricsEnvelope = resp.json().await.ok()?;
    if !data.response.status.eq_ignore_ascii_case("ok") {
        return None;
    }

    let value = data.response.lyrics?.value;
    if value.trim().is_empty() {
        return None;
    }

    // getLyrics may return LRC-formatted text
    let parsed = if lrc_has_usable_timing(&value) {
        parse_lrc(&value)
    } else {
        Vec::new()
    };
    if has_usable_line_timing(&parsed) {
        Some(Lyrics::Synced(parsed))
    } else {
        Some(Lyrics::Plain(value))
    }
}

async fn fetch_from_musixmatch_enhanced(artist: &str, title: &str) -> Option<Lyrics> {
    let query = format!("{title} {artist}");
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    let client = reqwest::Client::new();
    let started = Instant::now();
    let search = musixmatch_get(
        &client,
        "track.search",
        vec![
            ("q", query.to_string()),
            ("page_size", "5".to_string()),
            ("page", "1".to_string()),
        ],
        true,
    )
    .await?;
    tracing::info!(
        target: "kopuz::lyrics",
        "musixmatch request action=track.search elapsed_ms={}",
        started.elapsed().as_millis()
    );

    let status = search.pointer("/message/header/status_code")?.as_i64()?;
    if status != 200 {
        tracing::info!(
            target: "kopuz::lyrics",
            "musixmatch track.search status={status}"
        );
        lyrics_debug!("musixmatch track.search status={status}");
        return None;
    }

    let tracks = search.pointer("/message/body/track_list")?.as_array()?;
    let Some(track_id) = best_musixmatch_track_id(tracks, query) else {
        tracing::info!(
            target: "kopuz::lyrics",
            "musixmatch no_match query={query:?} candidates={}",
            tracks.len()
        );
        lyrics_debug!(
            "musixmatch no_match candidates={} query={:?}",
            tracks.len(),
            query
        );
        return None;
    };
    lyrics_debug!(
        "musixmatch selected_track_id={} candidates={}",
        track_id,
        tracks.len()
    );
    let started = Instant::now();
    let richsync = musixmatch_get(
        &client,
        "track.richsync.get",
        vec![("track_id", track_id)],
        true,
    )
    .await?;
    tracing::info!(
        target: "kopuz::lyrics",
        "musixmatch request action=track.richsync.get elapsed_ms={}",
        started.elapsed().as_millis()
    );

    let status = richsync.pointer("/message/header/status_code")?.as_i64()?;
    if status != 200 {
        tracing::info!(
            target: "kopuz::lyrics",
            "musixmatch richsync status={status}"
        );
        lyrics_debug!("musixmatch richsync status={status}");
        return None;
    }

    let body = richsync
        .pointer("/message/body/richsync/richsync_body")?
        .as_str()?;
    let enhanced_lrc = musixmatch_richsync_to_lrc(body)?;
    if !lrc_has_usable_timing(&enhanced_lrc) {
        lyrics_debug!("musixmatch richsync has no usable timing");
        return None;
    }
    let parsed = parse_lrc(&enhanced_lrc);
    if has_usable_line_timing(&parsed) && parsed.iter().any(|line| !line.chunks.is_empty()) {
        lyrics_debug!(
            "musixmatch richsync parsed lines={} timed_lines={} timed_parts={}",
            parsed.len(),
            timed_line_count(&parsed),
            timed_part_count(&parsed)
        );
        Some(Lyrics::Synced(parsed))
    } else {
        lyrics_debug!("musixmatch richsync parsed without word timestamps");
        None
    }
}

async fn fetch_from_paxsenix_youtube(
    artist: &str,
    title: &str,
    duration: u64,
    track_path: &str,
) -> Option<Lyrics> {
    let client = reqwest::Client::new();
    let video_id = if let Some(video_id) = extract_youtube_video_id(track_path) {
        video_id
    } else {
        let query = format!("{title} {artist}");
        let query = query.trim();
        if query.is_empty() {
            return None;
        }

        let results = client
            .get(format!("{PAXSENIX_ROOT_URL}/youtube/search"))
            .query(&[("q", query)])
            .timeout(PAXSENIX_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                tracing::info!(
                    target: "kopuz::lyrics",
                    "paxsenix_youtube search failed={error}"
                );
            })
            .ok()?
            .json::<Vec<PaxsenixYoutubeSearchResult>>()
            .await
            .map_err(|error| {
                tracing::info!(
                    target: "kopuz::lyrics",
                    "paxsenix_youtube search json_failed={error}"
                );
            })
            .ok()?;

        let selected = best_youtube_result(&results, query, duration)?;
        lyrics_debug!(
            "paxsenix_youtube selected_video id={} title={:?} artist={:?} candidates={}",
            selected.video_id,
            selected.title,
            selected.author,
            results.len()
        );
        selected.video_id.clone()
    };

    let lrc = client
        .get(format!("{PAXSENIX_ROOT_URL}/youtube/lyrics"))
        .query(&[("id", video_id.as_str())])
        .timeout(PAXSENIX_YOUTUBE_LYRICS_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_youtube lyrics failed={error}"
            );
        })
        .ok()?
        .text()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_youtube lyrics text_failed={error}"
            );
        })
        .ok()?;

    if lrc.trim().is_empty() || !lrc_has_usable_timing(&lrc) {
        return None;
    }

    let lines = parse_lrc(&lrc);
    if has_usable_line_timing(&lines) {
        Some(Lyrics::Synced(lines))
    } else {
        None
    }
}

fn best_youtube_result<'a>(
    results: &'a [PaxsenixYoutubeSearchResult],
    query: &str,
    duration: u64,
) -> Option<&'a PaxsenixYoutubeSearchResult> {
    results
        .iter()
        .filter_map(|result| {
            let candidate = format!("{} {}", result.title, result.author);
            let text_score = lyrics_match_score(&candidate, query);
            if text_score < 55.0 {
                return None;
            }

            let duration_score = match (duration, parse_colon_duration(&result.duration)) {
                (0, _) | (_, None) => 0.0,
                (expected, Some(candidate_seconds)) => {
                    let delta = candidate_seconds.abs_diff(expected);
                    if delta > 12 {
                        return None;
                    }
                    12.0 - delta as f64
                }
            };

            Some((text_score + duration_score, result))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, result)| result)
}

fn parse_colon_duration(duration: &str) -> Option<u64> {
    let mut total = 0_u64;
    let mut parts = duration.split(':').peekable();
    parts.peek()?;
    for part in parts {
        total = total
            .checked_mul(60)?
            .checked_add(part.parse::<u64>().ok()?)?;
    }
    Some(total)
}

fn extract_youtube_video_id(track_path: &str) -> Option<String> {
    track_path
        .strip_prefix("ytmusic:")
        .and_then(|rest| rest.split(':').next())
        .filter(|video_id| !video_id.trim().is_empty())
        .map(|video_id| video_id.to_string())
}

async fn fetch_from_paxsenix_apple_music(
    artist: &str,
    title: &str,
    duration: u64,
) -> Option<Lyrics> {
    let query = format!("{title} {artist}");
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    let client = reqwest::Client::new();
    let search = client
        .get(ITUNES_SEARCH_ROOT_URL)
        .query(&[
            ("term", query),
            ("entity", "song"),
            ("limit", "8"),
            ("country", "US"),
        ])
        .timeout(PAXSENIX_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_apple itunes_search failed={error}"
            );
        })
        .ok()?
        .json::<ItunesSearchResponse>()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_apple itunes_search json_failed={error}"
            );
        })
        .ok()?;

    let Some(song) = best_itunes_song(&search.results, query, duration) else {
        tracing::info!(
            target: "kopuz::lyrics",
            "paxsenix_apple no_match query={query:?} candidates={}",
            search.results.len()
        );
        lyrics_debug!(
            "paxsenix_apple no_match candidates={} query={:?}",
            search.results.len(),
            query
        );
        return None;
    };
    lyrics_debug!(
        "paxsenix_apple selected_track id={} title={:?} artist={:?} candidates={}",
        song.track_id,
        song.track_name,
        song.artist_name,
        search.results.len()
    );

    let response = client
        .get(format!("{PAXSENIX_ROOT_URL}/apple-music/lyrics"))
        .query(&[("id", song.track_id.to_string())])
        .timeout(PAXSENIX_APPLE_LYRICS_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_apple lyrics failed={error}"
            );
        })
        .ok()?
        .json::<PaxsenixAppleLyricsResponse>()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_apple lyrics json_failed={error}"
            );
        })
        .ok()?;

    let lyrics = paxsenix_apple_to_lyrics(response)?;
    if let Lyrics::Synced(lines) = &lyrics {
        lyrics_debug!(
            "paxsenix_apple parsed kind={} lines={} syllable_lines={} syllable_parts={}",
            lyrics_kind(Some(&lyrics)),
            lines.len(),
            timed_line_count(lines),
            timed_part_count(lines)
        );
    } else {
        lyrics_debug!("paxsenix_apple parsed kind={}", lyrics_kind(Some(&lyrics)));
    }
    Some(lyrics)
}

fn best_itunes_song<'a>(
    songs: &'a [ItunesSong],
    query: &str,
    duration: u64,
) -> Option<&'a ItunesSong> {
    songs
        .iter()
        .filter_map(|song| {
            let candidate = format!("{} {}", song.track_name, song.artist_name);
            let text_score = lyrics_match_score(&candidate, query);
            if text_score < 55.0 {
                return None;
            }

            let duration_score = match (duration, song.track_time_millis) {
                (0, _) | (_, None) => 0.0,
                (expected, Some(candidate_ms)) => {
                    let candidate_seconds = candidate_ms / 1000;
                    let delta = candidate_seconds.abs_diff(expected);
                    if delta > 12 {
                        return None;
                    }
                    12.0 - delta as f64
                }
            };

            Some((text_score + duration_score, song))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, song)| song)
}

fn paxsenix_apple_to_lyrics(response: PaxsenixAppleLyricsResponse) -> Option<Lyrics> {
    let lines = paxsenix_apple_to_lines(response.content);
    if let Some(lines) = lines
        && has_usable_line_timing(&lines)
    {
        return Some(Lyrics::Synced(lines));
    }

    if let Some(lrc) = response.lrc
        && !lrc.trim().is_empty()
    {
        let parsed = if lrc_has_usable_timing(&lrc) {
            parse_lrc(&lrc)
        } else {
            Vec::new()
        };
        if has_usable_line_timing(&parsed) {
            return Some(Lyrics::Synced(parsed));
        }
    }

    response
        .plain
        .filter(|plain| !plain.trim().is_empty())
        .map(Lyrics::Plain)
}

fn paxsenix_apple_to_lines(rows: Vec<PaxsenixAppleLyricLine>) -> Option<Vec<LyricLine>> {
    if !paxsenix_apple_has_timing(&rows) {
        lyrics_debug!("paxsenix_apple content has no usable timing");
        return None;
    }

    let mut lines = Vec::new();

    for row in rows {
        let row_start_time = row.timestamp as f64 / 1000.0;
        let row_end_time = row.endtime.map(|endtime| endtime as f64 / 1000.0);
        let opposite_turn = row.opposite_turn;
        let row_has_background_text = !row.background_text.is_empty();
        let main_line_is_background = row.background && !row_has_background_text;
        let mut parent_line_index = None;

        if let Some(line) = paxsenix_apple_parts_to_line(
            row.text,
            row_start_time,
            row_end_time,
            None,
            main_line_is_background,
            opposite_turn,
        ) {
            lines.push(line);
            if !main_line_is_background {
                parent_line_index = lines.len().checked_sub(1);
            }
        }

        let background_start_time = row
            .background_text
            .iter()
            .find_map(|part| part.timestamp)
            .map(|timestamp| timestamp as f64 / 1000.0)
            .unwrap_or(row_start_time);

        if let Some(line) = paxsenix_apple_parts_to_line(
            row.background_text,
            background_start_time,
            row_end_time,
            parent_line_index,
            true,
            opposite_turn,
        ) {
            lines.push(line);
        }
    }

    (!lines.is_empty()).then_some(lines)
}

fn paxsenix_apple_has_timing(rows: &[PaxsenixAppleLyricLine]) -> bool {
    rows.iter().any(|row| {
        row.timestamp > 0
            || row.endtime.is_some_and(|endtime| endtime > 0)
            || row
                .text
                .iter()
                .any(|part| part.timestamp.is_some_and(|timestamp| timestamp > 0))
            || row
                .background_text
                .iter()
                .any(|part| part.timestamp.is_some_and(|timestamp| timestamp > 0))
    })
}

fn paxsenix_apple_parts_to_line(
    parts: Vec<PaxsenixAppleLyricPart>,
    start_time: f64,
    end_time: Option<f64>,
    parent_line_index: Option<usize>,
    background: bool,
    opposite_turn: bool,
) -> Option<LyricLine> {
    let mut text = String::new();
    let mut chunks = Vec::new();
    let mut previous_part_continues = false;

    for part in parts {
        if part.text.trim().is_empty() {
            continue;
        }

        let prefix = if should_insert_apple_space(&text, previous_part_continues, &part.text) {
            " "
        } else {
            ""
        };
        let display_text = format!("{prefix}{}", part.text);

        text.push_str(&display_text);
        if let Some(timestamp) = part.timestamp {
            chunks.push(LyricChunk {
                start_time: timestamp as f64 / 1000.0,
                text: display_text,
            });
        }
        previous_part_continues = part.part;
    }

    let text = text.trim().to_string();
    (!text.is_empty()).then_some(LyricLine {
        start_time,
        end_time,
        text,
        chunks,
        parent_line_index,
        background,
        opposite_turn,
    })
}

fn should_insert_apple_space(
    current_text: &str,
    previous_part_continues: bool,
    next_text: &str,
) -> bool {
    if current_text.is_empty() || previous_part_continues {
        return false;
    }

    let Some(first_char) = next_text.chars().next() else {
        return false;
    };

    !matches!(
        first_char,
        ',' | '.' | '?' | '!' | ':' | ';' | ')' | ']' | '}' | '\'' | '’'
    )
}

async fn musixmatch_get(
    client: &reqwest::Client,
    action: &str,
    mut query: Vec<(&str, String)>,
    needs_token: bool,
) -> Option<serde_json::Value> {
    if needs_token {
        let token = musixmatch_token(client).await?;
        query.push(("usertoken", token));
    }

    query.push(("app_id", "web-desktop-app-v1.0".to_string()));
    query.push(("t", now_ms().to_string()));

    client
        .get(format!("{MUSIXMATCH_ROOT_URL}{action}"))
        .query(&query)
        .timeout(MUSIXMATCH_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "musixmatch request action={action} failed={error}"
            );
        })
        .ok()?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "musixmatch request action={action} json_failed={error}"
            );
        })
        .ok()
}

async fn musixmatch_token(client: &reqwest::Client) -> Option<String> {
    let now = now_ms();
    if let Some(token) = musixmatch_token_cache().lock().ok().and_then(|cache| {
        cache
            .as_ref()
            .filter(|token| token.expires_at_ms > now)
            .map(|token| token.value.clone())
    }) {
        return Some(token);
    }

    let response = client
        .get(format!("{MUSIXMATCH_ROOT_URL}token.get"))
        .query(&[
            ("user_language", "en".to_string()),
            ("app_id", "web-desktop-app-v1.0".to_string()),
            ("t", now.to_string()),
        ])
        .timeout(MUSIXMATCH_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "musixmatch token request failed={error}"
            );
        })
        .ok()?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "musixmatch token json_failed={error}"
            );
        })
        .ok()?;

    let status = response.pointer("/message/header/status_code")?.as_i64()?;
    if status != 200 {
        tracing::info!(
            target: "kopuz::lyrics",
            "musixmatch token status={status}"
        );
        return None;
    }

    let token = response
        .pointer("/message/body/user_token")?
        .as_str()?
        .to_string();

    if let Ok(mut cache) = musixmatch_token_cache().lock() {
        *cache = Some(MusixmatchToken {
            value: token.clone(),
            expires_at_ms: now + 10 * 60 * 1000,
        });
    }

    Some(token)
}

fn musixmatch_token_cache() -> &'static Mutex<Option<MusixmatchToken>> {
    MUSIXMATCH_TOKEN.get_or_init(|| Mutex::new(None))
}

fn best_musixmatch_track_id(tracks: &[serde_json::Value], query: &str) -> Option<String> {
    tracks
        .iter()
        .filter_map(|entry| {
            let track = entry.get("track")?;
            let name = track.get("track_name")?.as_str().unwrap_or_default();
            let artist = track.get("artist_name")?.as_str().unwrap_or_default();
            let candidate = format!("{name} {artist}");
            let score = lyrics_match_score(&candidate, query);
            let id = track.get("track_id")?.as_i64()?.to_string();
            Some((score, id))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .and_then(|(score, id)| (score >= 65.0).then_some(id))
}

fn lyrics_match_score(candidate: &str, query: &str) -> f64 {
    let candidate_tokens = normalized_lyric_match_tokens(candidate);
    let query_tokens = normalized_lyric_match_tokens(query);
    if candidate_tokens.is_empty() || query_tokens.is_empty() {
        return 0.0;
    }

    let shared = candidate_tokens
        .iter()
        .filter(|token| query_tokens.contains(*token))
        .count();
    (2 * shared) as f64 * 100.0 / (candidate_tokens.len() + query_tokens.len()) as f64
}

fn normalized_lyric_match_tokens(value: &str) -> HashSet<String> {
    value
        .to_lowercase()
        .replace("(feat.", " ")
        .replace("(ft.", " ")
        .replace("(featuring", " ")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn musixmatch_richsync_to_lrc(body: &str) -> Option<String> {
    let rows = serde_json::from_str::<Vec<serde_json::Value>>(body).ok()?;
    let mut output = String::new();

    for row in rows {
        let line_start = json_number(&row, "ts")?;
        output.push('[');
        output.push_str(&format_lrc_time(line_start));
        output.push(']');

        if let Some(words) = row.get("l").and_then(|value| value.as_array()) {
            for word in words {
                let offset = json_number(word, "o").unwrap_or(0.0);
                let content = word.get("c").and_then(|value| value.as_str()).unwrap_or("");
                if content.trim().is_empty() {
                    continue;
                }
                output.push('<');
                output.push_str(&format_lrc_time(line_start + offset));
                output.push('>');
                output.push_str(content);
                output.push(' ');
            }
        }

        output.push('\n');
    }

    (!output.trim().is_empty()).then_some(output)
}

fn json_number(value: &serde_json::Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(|number| number.as_f64().or_else(|| number.as_str()?.parse().ok()))
}

fn format_lrc_time(time_in_seconds: f64) -> String {
    let time = time_in_seconds.max(0.0);
    let total_centiseconds = (time * 100.0).round() as u64;
    let minutes = total_centiseconds / 6000;
    let seconds = (total_centiseconds / 100) % 60;
    let centiseconds = total_centiseconds % 100;
    format!("{minutes:02}:{seconds:02}.{centiseconds:02}")
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

async fn fetch_from_lrclib(
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
) -> Option<Lyrics> {
    let mut url = format!(
        "https://lrclib.net/api/get?artist_name={}&track_name={}",
        percent_encoding::utf8_percent_encode(artist, NON_ALPHANUMERIC),
        percent_encoding::utf8_percent_encode(title, NON_ALPHANUMERIC)
    );

    if !album.is_empty() {
        url.push_str(&format!(
            "&album_name={}",
            percent_encoding::utf8_percent_encode(album, NON_ALPHANUMERIC)
        ));
    }
    if duration > 0 {
        url.push_str(&format!("&duration={}", duration));
    }

    let client = reqwest::Client::new();
    let res = match client
        .get(&url)
        .header("User-Agent", concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
        .timeout(LRCLIB_TIMEOUT)
        .send()
        .await
    {
        Ok(res) => res,
        Err(error) => {
            tracing::info!(
                target: "kopuz::lyrics",
                "lrclib get failed={error}"
            );
            return None;
        }
    };

    let mut fallback = None;

    if res.status().is_success()
        && let Ok(data) = res.json::<LrcLibResponse>().await
        && let Some(lyrics) = extract_from_lrclib_response(&data)
    {
        if has_word_timestamps(&lyrics) {
            return Some(lyrics);
        }
        fallback = Some(lyrics);
    }

    let search_url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        percent_encoding::utf8_percent_encode(title, NON_ALPHANUMERIC),
        percent_encoding::utf8_percent_encode(artist, NON_ALPHANUMERIC)
    );
    let search_res = match client
        .get(&search_url)
        .header("User-Agent", concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
        .timeout(LRCLIB_TIMEOUT)
        .send()
        .await
    {
        Ok(res) => res,
        Err(error) => {
            tracing::info!(
                target: "kopuz::lyrics",
                "lrclib search failed={error}"
            );
            return fallback;
        }
    };

    if search_res.status().is_success()
        && let Ok(results) = search_res.json::<Vec<LrcLibResponse>>().await
    {
        for data in results {
            if let Some(lyrics) = extract_from_lrclib_response(&data) {
                if has_word_timestamps(&lyrics) {
                    return Some(lyrics);
                }
                fallback.get_or_insert(lyrics);
            }
        }
    }

    fallback
}

fn extract_from_lrclib_response(data: &LrcLibResponse) -> Option<Lyrics> {
    if let Some(synced) = &data.synced_lyrics
        && !synced.trim().is_empty()
        && lrc_has_usable_timing(synced)
    {
        return Some(Lyrics::Synced(parse_lrc(synced)));
    }
    if let Some(plain) = &data.plain_lyrics
        && !plain.trim().is_empty()
    {
        return Some(Lyrics::Plain(plain.clone()));
    }
    None
}

fn append_translation(target: &mut String, text: &str) {
    let text = text.trim();

    if text.is_empty() {
        return;
    }

    if !target.is_empty() {
        target.push('\n');
    }

    if text.starts_with('(') && text.ends_with(')') {
        target.push_str(text);
    } else {
        target.push('(');
        target.push_str(text);
        target.push(')');
    }
}

fn append_line_translation(target: &mut LyricLine, text: &str) {
    append_translation(&mut target.text, text);
}

fn parse_lrc(lrc_text: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();

    for raw_line in lrc_text.lines() {
        let (line_timestamps, content) = extract_line_timestamps(raw_line);
        let (text, words) = parse_enhanced_words(content);

        if line_timestamps.is_empty() {
            let is_metadata_tag = raw_line.trim().starts_with('[')
                && raw_line.trim().ends_with(']')
                && raw_line.contains(':');
            if is_metadata_tag {
                continue;
            }

            if !words.is_empty() {
                lines.push(LyricLine {
                    start_time: words[0].start_time,
                    end_time: None,
                    text,
                    chunks: words,
                    parent_line_index: None,
                    background: false,
                    opposite_turn: false,
                });
            } else if !text.is_empty()
                && let Some(last) = lines.last_mut()
            {
                append_line_translation(last, &text);
            }
            continue;
        }

        if text.is_empty() && words.is_empty() {
            continue;
        }

        for start_time in line_timestamps {
            lines.push(LyricLine {
                start_time,
                end_time: None,
                text: text.clone(),
                chunks: words.clone(),
                parent_line_index: None,
                background: false,
                opposite_turn: false,
            });
        }
    }

    lines.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut merged: Vec<LyricLine> = Vec::new();

    for mut line in lines {
        if let Some(last) = merged.last_mut()
            && last.start_time == line.start_time
        {
            if last.chunks.is_empty() && !line.chunks.is_empty() {
                last.chunks = std::mem::take(&mut line.chunks);
            }
            if last.end_time.is_none() {
                last.end_time = line.end_time;
            }
            append_line_translation(last, &line.text);
            continue;
        }

        merged.push(line);
    }

    merged
}

fn extract_line_timestamps(line: &str) -> (Vec<f64>, &str) {
    let mut timestamps = Vec::new();
    let mut rest = line.trim_start();

    while let Some(after_open) = rest.strip_prefix('[') {
        let Some(close_idx) = after_open.find(']') else {
            break;
        };

        let tag = &after_open[..close_idx];
        let after_tag = &after_open[close_idx + 1..];
        if let Some(time) = parse_lrc_time(tag) {
            timestamps.push(time);
            rest = after_tag;
        } else if tag.chars().any(|c| c.is_ascii_alphabetic()) && tag.contains(':') {
            rest = after_tag;
        } else {
            break;
        }
    }

    (timestamps, rest)
}

fn parse_enhanced_words(content: &str) -> (String, Vec<LyricChunk>) {
    let mut words = Vec::new();
    let mut text = String::new();
    let mut rest = content;
    let mut pending_time: Option<f64> = None;

    while let Some(open_idx) = rest.find('<') {
        let before = &rest[..open_idx];
        text.push_str(before);
        if let Some(start_time) = pending_time.take()
            && !before.is_empty()
        {
            words.push(LyricChunk {
                start_time,
                text: before.to_string(),
            });
        }

        let after_open = &rest[open_idx + 1..];
        let Some(close_idx) = after_open.find('>') else {
            text.push_str(&rest[open_idx..]);
            rest = "";
            break;
        };

        let tag = &after_open[..close_idx];
        if let Some(time) = parse_lrc_time(tag) {
            pending_time = Some(time);
            rest = &after_open[close_idx + 1..];
        } else {
            text.push('<');
            text.push_str(tag);
            text.push('>');
            rest = &after_open[close_idx + 1..];
        }
    }

    text.push_str(rest);
    if let Some(start_time) = pending_time
        && !rest.is_empty()
    {
        words.push(LyricChunk {
            start_time,
            text: rest.to_string(),
        });
    }

    (text.trim().to_string(), words)
}

fn parse_lrc_time(time_str: &str) -> Option<f64> {
    let parts: Vec<&str> = time_str.split([':', '.']).collect();
    if parts.len() >= 2 {
        let min = parts[0].parse::<f64>().ok()?;
        let sec = parts[1].parse::<f64>().ok()?;
        let mut total = min * 60.0 + sec;
        if parts.len() == 3 {
            let ms_str = parts[2];
            let ms = ms_str.parse::<f64>().ok()?;
            let divisor = 10_f64.powi(ms_str.len() as i32);
            total += ms / divisor;
        }
        Some(total)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_regular_lrc_lines() {
        let lines = parse_lrc("[00:01.00]Hello\n[00:02.50]World");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_time, 1.0);
        assert_eq!(lines[0].text, "Hello");
        assert!(lines[0].chunks.is_empty());
        assert_eq!(lines[1].start_time, 2.5);
        assert_eq!(lines[1].text, "World");
    }

    #[test]
    fn parses_enhanced_lrc_word_timestamps() {
        let lines = parse_lrc("[00:10.00]<00:10.10>Hello <00:10.60>world");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_time, 10.0);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].chunks.len(), 2);
        assert_eq!(lines[0].chunks[0].start_time, 10.1);
        assert_eq!(lines[0].chunks[0].text, "Hello ");
        assert_eq!(lines[0].chunks[1].start_time, 10.6);
        assert_eq!(lines[0].chunks[1].text, "world");
    }

    #[test]
    fn duplicate_lrc_timestamps_preserve_chunks() {
        let lines = parse_lrc("[00:10.00]Translation\n[00:10.00]<00:10.10>Hello <00:10.60>world");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Translation\n(Hello world)");
        assert_eq!(lines[0].chunks.len(), 2);
        assert_eq!(lines[0].chunks[0].start_time, 10.1);
        assert_eq!(lines[0].chunks[0].text, "Hello ");
        assert_eq!(lines[0].chunks[1].start_time, 10.6);
        assert_eq!(lines[0].chunks[1].text, "world");
    }

    #[test]
    fn detects_word_timed_lyrics() {
        let line_only = Lyrics::Synced(parse_lrc("[00:01.00]Hello"));
        let single_chunk = Lyrics::Synced(parse_lrc("[00:01.00]<00:01.10>Hello"));
        let word_timed = Lyrics::Synced(parse_lrc("[00:01.00]<00:01.10>Hello <00:01.50>world"));

        assert!(!has_word_timestamps(&line_only));
        assert!(!has_word_timestamps(&single_chunk));
        assert!(has_word_timestamps(&word_timed));
    }

    #[test]
    fn lyrics_log_key_is_hashed() {
        let key = "/Users/someone/Music/private-library/song.flac";
        let logged = log_lyrics_key_hash(key);

        assert_eq!(logged.len(), 16);
        assert!(logged.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!logged.contains("/Users"));
        assert_ne!(logged, key);
    }

    #[test]
    fn converts_musixmatch_richsync_to_enhanced_lrc() {
        let body = r#"[{"ts":10.0,"l":[{"o":0.1,"c":"Hello"},{"o":0.6,"c":"world"}]}]"#;
        let lrc = musixmatch_richsync_to_lrc(body).expect("richsync should convert");
        let parsed = parse_lrc(&lrc);

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].start_time, 10.0);
        assert_eq!(parsed[0].chunks.len(), 2);
        assert_eq!(parsed[0].chunks[0].start_time, 10.1);
        assert_eq!(parsed[0].chunks[0].text, "Hello ");
        assert_eq!(parsed[0].chunks[1].start_time, 10.6);
        assert_eq!(parsed[0].chunks[1].text, "world ");
    }

    #[test]
    fn rejects_multi_line_lrc_without_progressing_timestamps() {
        assert!(lrc_has_usable_timing("[00:00.00]Only line"));
        assert!(lrc_has_usable_timing("[00:00.00]First\n[00:01.00]Second"));
        assert!(!lrc_has_usable_timing("[00:00.00]First\n[00:00.00]Second"));
    }

    #[test]
    fn lrclib_falls_back_to_plain_when_synced_lrc_has_no_progressing_timestamps() {
        let response = LrcLibResponse {
            synced_lyrics: Some("[00:00.00]First\n[00:00.00]Second".to_string()),
            plain_lyrics: Some("First\nSecond".to_string()),
        };

        let lyrics = extract_from_lrclib_response(&response).expect("plain lyrics should convert");
        assert_eq!(lyrics, Lyrics::Plain("First\nSecond".to_string()));
    }

    #[test]
    fn extracts_youtube_video_id_from_track_path() {
        assert_eq!(
            extract_youtube_video_id("ytmusic:r9jGBwgzEzA:https%3A%2F%2Fimg"),
            Some("r9jGBwgzEzA".to_string())
        );
        assert_eq!(
            extract_youtube_video_id("ytmusic:r9jGBwgzEzA"),
            Some("r9jGBwgzEzA".to_string())
        );
        assert_eq!(extract_youtube_video_id("/music/song.flac"), None);
    }

    #[test]
    fn parses_youtube_duration() {
        assert_eq!(parse_colon_duration("3:28"), Some(208));
        assert_eq!(parse_colon_duration("1:02:03"), Some(3723));
        assert_eq!(parse_colon_duration(""), None);
        assert_eq!(parse_colon_duration("nope"), None);
    }

    #[test]
    fn youtube_selector_checks_duration() {
        let results = vec![
            PaxsenixYoutubeSearchResult {
                video_id: "wrong-duration".to_string(),
                title: "90210".to_string(),
                author: "blackbear".to_string(),
                duration: "5:40".to_string(),
            },
            PaxsenixYoutubeSearchResult {
                video_id: "right-duration".to_string(),
                title: "90210".to_string(),
                author: "blackbear".to_string(),
                duration: "3:28".to_string(),
            },
        ];

        let selected = best_youtube_result(&results, "90210 blackbear", 208)
            .expect("a duration-matched result should be selected");

        assert_eq!(selected.video_id, "right-duration");
    }

    #[test]
    fn converts_paxsenix_apple_syllable_lyrics() {
        let response = PaxsenixAppleLyricsResponse {
            content: vec![PaxsenixAppleLyricLine {
                text: vec![
                    PaxsenixAppleLyricPart {
                        text: "Hel".to_string(),
                        timestamp: Some(10100),
                        part: true,
                    },
                    PaxsenixAppleLyricPart {
                        text: "lo".to_string(),
                        timestamp: Some(10300),
                        part: false,
                    },
                    PaxsenixAppleLyricPart {
                        text: "world".to_string(),
                        timestamp: Some(10600),
                        part: false,
                    },
                ],
                background_text: Vec::new(),
                timestamp: 10000,
                endtime: Some(11000),
                background: false,
                opposite_turn: false,
            }],
            lrc: None,
            plain: None,
        };

        let Lyrics::Synced(lines) =
            paxsenix_apple_to_lyrics(response).expect("lyrics should convert")
        else {
            panic!("apple lyrics should be synced");
        };

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_time, 10.0);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].chunks.len(), 3);
        assert_eq!(lines[0].chunks[0].start_time, 10.1);
        assert_eq!(lines[0].chunks[0].text, "Hel");
        assert_eq!(lines[0].chunks[2].start_time, 10.6);
        assert_eq!(lines[0].chunks[2].text, " world");
    }

    #[test]
    fn skips_untimed_paxsenix_apple_content() {
        let response = PaxsenixAppleLyricsResponse {
            content: vec![PaxsenixAppleLyricLine {
                text: vec![PaxsenixAppleLyricPart {
                    text: "Untimed".to_string(),
                    timestamp: None,
                    part: false,
                }],
                background_text: Vec::new(),
                timestamp: 0,
                endtime: Some(0),
                background: false,
                opposite_turn: false,
            }],
            lrc: None,
            plain: Some("Untimed".to_string()),
        };

        let lyrics = paxsenix_apple_to_lyrics(response).expect("plain lyrics should convert");
        assert_eq!(lyrics, Lyrics::Plain("Untimed".to_string()));
    }

    #[test]
    fn splits_paxsenix_apple_background_text() {
        let response = PaxsenixAppleLyricsResponse {
            content: vec![
                PaxsenixAppleLyricLine {
                    text: vec![
                        PaxsenixAppleLyricPart {
                            text: "Looking".to_string(),
                            timestamp: Some(10000),
                            part: false,
                        },
                        PaxsenixAppleLyricPart {
                            text: "for".to_string(),
                            timestamp: Some(10500),
                            part: false,
                        },
                    ],
                    background_text: vec![PaxsenixAppleLyricPart {
                        text: "Echo".to_string(),
                        timestamp: Some(10800),
                        part: false,
                    }],
                    timestamp: 10000,
                    endtime: Some(12000),
                    background: true,
                    opposite_turn: true,
                },
                PaxsenixAppleLyricLine {
                    text: vec![PaxsenixAppleLyricPart {
                        text: "Next".to_string(),
                        timestamp: Some(10600),
                        part: false,
                    }],
                    background_text: Vec::new(),
                    timestamp: 10600,
                    endtime: Some(13000),
                    background: false,
                    opposite_turn: true,
                },
            ],
            lrc: None,
            plain: None,
        };

        let Lyrics::Synced(lines) =
            paxsenix_apple_to_lyrics(response).expect("lyrics should convert")
        else {
            panic!("apple lyrics should be synced");
        };

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "Looking for");
        assert!(!lines[0].background);
        assert!(lines[0].opposite_turn);
        assert_eq!(lines[0].parent_line_index, None);
        assert_eq!(lines[1].text, "Echo");
        assert!(lines[1].background);
        assert!(lines[1].opposite_turn);
        assert_eq!(lines[1].parent_line_index, Some(0));
        assert_eq!(lines[1].start_time, 10.8);
        assert_eq!(lines[2].text, "Next");
        assert_eq!(lines[2].parent_line_index, None);
    }

    #[test]
    fn itunes_selector_checks_duration() {
        let songs = vec![
            ItunesSong {
                track_id: 1,
                track_name: "Somebody Told Me".to_string(),
                artist_name: "The Killers".to_string(),
                track_time_millis: Some(230_000),
            },
            ItunesSong {
                track_id: 2,
                track_name: "Somebody Told Me".to_string(),
                artist_name: "The Killers".to_string(),
                track_time_millis: Some(197_200),
            },
        ];

        let selected = best_itunes_song(&songs, "Somebody Told Me The Killers", 198)
            .expect("a duration-matched result should be selected");

        assert_eq!(selected.track_id, 2);
    }
}
