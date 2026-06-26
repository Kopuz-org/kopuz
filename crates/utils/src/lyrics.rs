use percent_encoding::NON_ALPHANUMERIC;
use serde::Deserialize;
use std::collections::{HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

mod cache;
mod local;
mod lrc;
mod model;
mod request;
mod server;
mod apple_music;

use cache::{
    LyricsInflightGuard, load_persisted_lyrics, lyrics_cache, store_lyrics, try_begin_lyrics_fetch,
};
use local::fetch_local_lrc;
use lrc::{extract_line_timestamps, parse_enhanced_words, parse_lrc};
pub use model::{LyricChunk, LyricLine, Lyrics};
pub use request::{AppleMusicLyricsAuth, LyricsRequest, LyricsServerAuth};
use server::{fetch_jellyfin_lyrics, fetch_subsonic_lyrics};

const LRCLIB_TIMEOUT: Duration = Duration::from_secs(5);
const SERVER_LYRICS_TIMEOUT: Duration = Duration::from_secs(5);
const LYRICS_INFLIGHT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const LYRICS_INFLIGHT_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

macro_rules! lyrics_debug {
    ($($arg:tt)*) => {
        if $crate::lyrics::lyrics_terminal_debug_enabled() {
            tracing::debug!("[lyrics] {}", format_args!($($arg)*));
        }
    };
}

mod musixmatch;
mod paxsenix;

use musixmatch::fetch_from_musixmatch_enhanced;
use paxsenix::{fetch_from_paxsenix_apple_music, fetch_from_paxsenix_youtube};

#[derive(Debug, Deserialize)]
struct LrcLibResponse {
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
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
#[tracing::instrument(name = "lyrics.fetch", skip_all, fields(artist = %request.artist, title = %request.title))]
pub async fn fetch_lyrics_for_request(request: &LyricsRequest) -> Option<Lyrics> {
    fetch_lyrics_with_progress(request, true, |_| {}).await
}

pub async fn fetch_lyrics_progressive_for_request<F>(
    request: &LyricsRequest,
    on_progress: F,
) -> Option<Lyrics>
where
    F: FnMut(Lyrics),
{
    fetch_lyrics_with_progress(request, true, on_progress).await
}

#[deprecated(note = "use LyricsRequest with fetch_lyrics_for_request")]
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
    let request = LyricsRequest::new(artist, title, album, duration, track_path)
        .with_server(server_url, server_token, server_user_id)
        .prefer_local(prefer_local)
        .enable_musixmatch(enable_musixmatch);
    fetch_lyrics_for_request(&request).await
}

#[deprecated(note = "use LyricsRequest with fetch_lyrics_progressive_for_request")]
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
    let request = LyricsRequest::new(artist, title, album, duration, track_path)
        .with_server(server_url, server_token, server_user_id)
        .prefer_local(prefer_local)
        .enable_musixmatch(enable_musixmatch);
    fetch_lyrics_progressive_for_request(&request, on_progress).await
}

async fn fetch_lyrics_with_progress<F>(
    request: &LyricsRequest,
    allow_lrclib: bool,
    mut on_progress: F,
) -> Option<Lyrics>
where
    F: FnMut(Lyrics),
{
    let cache_key = request.cache_key();
    let artist = request.artist.as_str();
    let title = request.title.as_str();
    let album = request.album.as_str();
    let duration = request.duration;
    let track_path = request.track_path.as_str();
    let server_url = request.server.as_ref().map(|server| server.url.as_str());
    let server_token = request
        .server
        .as_ref()
        .and_then(|server| server.token.as_deref());
    let server_user_id = request
        .server
        .as_ref()
        .and_then(|server| server.user_id.as_deref());
    let prefer_local = request.prefer_local;
    let enable_musixmatch = request.enable_musixmatch;
    let cache_key_hash = log_lyrics_key_hash(&cache_key);
    let total_start = Instant::now();
    lyrics_debug!(
        "fetch start key_hash={} artist={:?} title={:?} duration={} prefer_local={}",
        cache_key_hash,
        request.artist,
        request.title,
        request.duration,
        request.prefer_local
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

    // Persistent layer: lyrics survive restarts — a DB hit skips the whole
    // provider chain and seeds the in-memory LRU.
    if let Some(persisted) = load_persisted_lyrics(&cache_key).await {
        lyrics_debug!(
            "db hit key_hash={} kind={}",
            cache_key_hash,
            lyrics_kind(persisted.as_ref())
        );
        if let Ok(mut cache) = lyrics_cache().lock() {
            cache.put(cache_key.clone(), persisted.clone());
        }
        return persisted;
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
                store_lyrics(&cache_key, &Some(lyrics.clone())).await;
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
        store_lyrics(&cache_key, &fallback).await;
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
                        store_lyrics(&cache_key, &Some(lyrics.clone())).await;
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
                        store_lyrics(&cache_key, &Some(lyrics.clone())).await;
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

    if let Some(am_auth) = &request.apple_music_auth {
        if track_path.starts_with("applemusic:") {
            let started = Instant::now();
            let am_lyrics = apple_music::fetch_apple_music_lyrics(am_auth).await;
            tracing::info!(
                target: "kopuz::lyrics",
                "apple_music key_hash={} elapsed_ms={} kind={}",
                log_lyrics_key_hash(&cache_key),
                started.elapsed().as_millis(),
                lyrics_kind(am_lyrics.as_ref())
            );
            lyrics_debug!(
                "provider=apple_music elapsed_ms={} kind={}",
                started.elapsed().as_millis(),
                lyrics_kind(am_lyrics.as_ref())
            );
            if let Some(lyrics) = am_lyrics {
                if has_word_timestamps(&lyrics) {
                    store_lyrics(&cache_key, &Some(lyrics.clone())).await;
                    tracing::info!(
                        target: "kopuz::lyrics",
                        "selected key_hash={} source=apple_music kind={} total_ms={}",
                        log_lyrics_key_hash(&cache_key),
                        lyrics_kind(Some(&lyrics)),
                        total_start.elapsed().as_millis()
                    );
                    lyrics_debug!(
                        "selected source=apple_music key_hash={} kind={} total_ms={}",
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
    store_lyrics(&cache_key, &fetched).await;
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

pub fn cached_lyrics_for_request(request: &LyricsRequest) -> Option<Option<Lyrics>> {
    let cache_key = request.cache_key();
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

#[cfg(test)]
mod tests {
    use super::musixmatch::musixmatch_richsync_to_lrc;
    use super::paxsenix::{
        best_itunes_song, best_youtube_result, extract_youtube_video_id, parse_colon_duration,
        paxsenix_apple_to_lyrics,
    };
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
