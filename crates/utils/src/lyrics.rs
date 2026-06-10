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
pub struct LyricWord {
    pub start_time: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub start_time: f64,
    pub text: String,
    pub words: Vec<LyricWord>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lyrics {
    Synced(Vec<LyricLine>),
    Plain(String),
}

const LYRICS_CACHE_CAPACITY: usize = 256;
const MUSIXMATCH_ROOT_URL: &str = "https://apic-desktop.musixmatch.com/ws/1.1/";
const PAXSENIX_ROOT_URL: &str = "https://lyrics.paxsenix.org";
const MUSIXMATCH_TIMEOUT: Duration = Duration::from_secs(3);
const PAXSENIX_TIMEOUT: Duration = Duration::from_secs(5);
const LRCLIB_TIMEOUT: Duration = Duration::from_secs(5);
const SERVER_LYRICS_TIMEOUT: Duration = Duration::from_secs(5);
const LYRICS_INFLIGHT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const LYRICS_INFLIGHT_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

static LYRICS_CACHE: OnceLock<Mutex<LyricsCache>> = OnceLock::new();
static LYRICS_INFLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static MUSIXMATCH_TOKEN: OnceLock<Mutex<Option<MusixmatchToken>>> = OnceLock::new();

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

// --- Paxsenix NetEase lyrics types ---

#[derive(Debug, Deserialize)]
struct PaxsenixNeteaseSearchResponse {
    #[serde(default)]
    result: Option<PaxsenixNeteaseSearchResult>,
}

#[derive(Debug, Deserialize)]
struct PaxsenixNeteaseSearchResult {
    #[serde(default)]
    songs: Vec<PaxsenixNeteaseSong>,
}

#[derive(Debug, Deserialize)]
struct PaxsenixNeteaseSong {
    id: u64,
    name: String,
    #[serde(default)]
    duration: Option<u64>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    artists: Vec<PaxsenixNeteaseArtist>,
}

#[derive(Debug, Deserialize)]
struct PaxsenixNeteaseArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PaxsenixNeteaseLyricLine {
    #[serde(default)]
    text: Vec<PaxsenixNeteaseLyricWord>,
    timestamp: u64,
    #[serde(default)]
    background: bool,
}

#[derive(Debug, Deserialize)]
struct PaxsenixNeteaseLyricWord {
    text: String,
    timestamp: u64,
}

// --- Public API ---

/// Fetch lyrics for a track, trying sources in priority order while preferring
/// word-timed lyrics over line-only matches:
/// 1. Local .lrc file alongside the audio file (local tracks only)
/// 2. Jellyfin or Subsonic server lyrics API (server tracks)
/// 3. Paxsenix NetEase lyrics (word-synced fallback for all tracks)
/// 4. Musixmatch richsync (word-synced fallback for all tracks)
/// 5. lrclib.net (fallback for all tracks)
///
/// For Jellyfin: `server_token` = access token, `server_user_id` = user_id (unused for lyrics)
/// For Subsonic: `server_token` = password, `server_user_id` = username
// skip_all, not skip(track_path): a bare skip auto-records every other arg
// as a span field, which would leak server_token (and url/user_id) into the
// trace + log. Record only artist/title, explicitly.
#[tracing::instrument(name = "lyrics.fetch", skip_all, fields(artist = %artist, title = %title))]
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
) -> Option<Lyrics> {
    let cache_key = lyrics_cache_key(artist, title, album, duration, track_path);
    let total_start = Instant::now();
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
        return cached;
    }

    let _inflight_guard = if try_begin_lyrics_fetch(&cache_key) {
        Some(LyricsInflightGuard {
            key: cache_key.clone(),
        })
    } else {
        let wait_start = Instant::now();
        while wait_start.elapsed() < LYRICS_INFLIGHT_WAIT_TIMEOUT {
            crate::sleep(LYRICS_INFLIGHT_POLL_INTERVAL).await;
            if let Some(cached) = lyrics_cache()
                .lock()
                .ok()
                .and_then(|mut cache| cache.get_cloned(&cache_key))
            {
                return cached;
            }
        }

        if try_begin_lyrics_fetch(&cache_key) {
            Some(LyricsInflightGuard {
                key: cache_key.clone(),
            })
        } else {
            if let Some(cached) = lyrics_cache()
                .lock()
                .ok()
                .and_then(|mut cache| cache.get_cloned(&cache_key))
            {
                return cached;
            }
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
                        return Some(lyrics);
                    }
                    fallback.get_or_insert(lyrics);
                }
            }
        }
    }

    // 3. Paxsenix NetEase can provide enhanced word-by-word timestamps.
    let started = Instant::now();
    let paxsenix_netease = fetch_from_paxsenix_netease(artist, title, duration).await;
    tracing::info!(
        target: "kopuz::lyrics",
        "paxsenix_netease key_hash={} elapsed_ms={} kind={}",
        log_lyrics_key_hash(&cache_key),
        started.elapsed().as_millis(),
        lyrics_kind(paxsenix_netease.as_ref())
    );
    if let Some(lyrics) = paxsenix_netease {
        if has_word_timestamps(&lyrics) {
            if let Ok(mut cache) = lyrics_cache().lock() {
                cache.put(cache_key.clone(), Some(lyrics.clone()));
            }
            tracing::info!(
                target: "kopuz::lyrics",
                "selected key_hash={} source=paxsenix_netease kind={} total_ms={}",
                log_lyrics_key_hash(&cache_key),
                lyrics_kind(Some(&lyrics)),
                total_start.elapsed().as_millis()
            );
            return Some(lyrics);
        }
        fallback.get_or_insert(lyrics);
    }

    // 4. Musixmatch richsync can provide enhanced word-by-word timestamps.
    let started = Instant::now();
    let musixmatch = fetch_from_musixmatch_enhanced(artist, title).await;
    tracing::info!(
        target: "kopuz::lyrics",
        "musixmatch_enhanced key_hash={} elapsed_ms={} kind={}",
        log_lyrics_key_hash(&cache_key),
        started.elapsed().as_millis(),
        lyrics_kind(musixmatch.as_ref())
    );
    if let Some(lyrics) = musixmatch {
        if has_word_timestamps(&lyrics) {
            if let Ok(mut cache) = lyrics_cache().lock() {
                cache.put(cache_key.clone(), Some(lyrics.clone()));
            }
            tracing::info!(
                target: "kopuz::lyrics",
                "selected key_hash={} source=musixmatch kind={} total_ms={}",
                log_lyrics_key_hash(&cache_key),
                lyrics_kind(Some(&lyrics)),
                total_start.elapsed().as_millis()
            );
            return Some(lyrics);
        }
        fallback.get_or_insert(lyrics);
    }

    // 5. lrclib fallback
    let started = Instant::now();
    let lrclib = fetch_from_lrclib(artist, title, album, duration).await;
    tracing::info!(
        target: "kopuz::lyrics",
        "lrclib key_hash={} elapsed_ms={} kind={}",
        log_lyrics_key_hash(&cache_key),
        started.elapsed().as_millis(),
        lyrics_kind(lrclib.as_ref())
    );
    let fetched = lrclib.or(fallback);
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
    fetched
}

pub fn cached_lyrics(
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
    track_path: &str,
) -> Option<Option<Lyrics>> {
    let cache_key = lyrics_cache_key(artist, title, album, duration, track_path);
    lyrics_cache()
        .lock()
        .ok()
        .and_then(|mut cache| cache.get_cloned(&cache_key))
}

fn has_word_timestamps(lyrics: &Lyrics) -> bool {
    match lyrics {
        Lyrics::Synced(lines) => lines.iter().any(|line| !line.words.is_empty()),
        Lyrics::Plain(_) => false,
    }
}

fn lyrics_kind(lyrics: Option<&Lyrics>) -> &'static str {
    match lyrics {
        Some(Lyrics::Synced(lines)) if lines.iter().any(|line| !line.words.is_empty()) => {
            "synced_word"
        }
        Some(Lyrics::Synced(_)) => "synced_line",
        Some(Lyrics::Plain(_)) => "plain",
        None => "none",
    }
}

fn log_lyrics_key_hash(key: &str) -> String {
    let mut hasher = DefaultHasher::new();
    key.trim().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
) -> String {
    if !track_path.trim().is_empty() {
        return track_path.trim().to_string();
    }

    format!(
        "{}|{}|{}|{}",
        artist.trim().to_lowercase(),
        title.trim().to_lowercase(),
        album.trim().to_lowercase(),
        duration
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
    let lines = parse_lrc(&content);
    if !lines.is_empty() {
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
        if matches_appended || matches_stem {
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Some(content);
            }
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
                    text: l.text.clone(),
                    words: Vec::new(),
                })
            })
            .collect();
        lines.sort_by(|a, b| {
            a.start_time
                .partial_cmp(&b.start_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if !lines.is_empty() {
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
                    text: l.value.clone(),
                    words: Vec::new(),
                })
            })
            .collect();
        lines.sort_by(|a, b| {
            a.start_time
                .partial_cmp(&b.start_time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if !lines.is_empty() {
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
    let parsed = parse_lrc(&value);
    if !parsed.is_empty() {
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
        return None;
    }

    let tracks = search.pointer("/message/body/track_list")?.as_array()?;
    let Some(track_id) = best_musixmatch_track_id(tracks, query) else {
        tracing::info!(
            target: "kopuz::lyrics",
            "musixmatch no_match query={query:?} candidates={}",
            tracks.len()
        );
        return None;
    };
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
        return None;
    }

    let body = richsync
        .pointer("/message/body/richsync/richsync_body")?
        .as_str()?;
    let enhanced_lrc = musixmatch_richsync_to_lrc(body)?;
    let parsed = parse_lrc(&enhanced_lrc);
    if parsed.iter().any(|line| !line.words.is_empty()) {
        Some(Lyrics::Synced(parsed))
    } else {
        None
    }
}

async fn fetch_from_paxsenix_netease(artist: &str, title: &str, duration: u64) -> Option<Lyrics> {
    let query = format!("{title} {artist}");
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    let client = reqwest::Client::new();
    let search = client
        .get(format!("{PAXSENIX_ROOT_URL}/netease/search"))
        .query(&[("q", query)])
        .timeout(PAXSENIX_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_netease search failed={error}"
            );
        })
        .ok()?
        .json::<PaxsenixNeteaseSearchResponse>()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_netease search json_failed={error}"
            );
        })
        .ok()?;

    let songs = search.result?.songs;
    let Some(song) = best_paxsenix_netease_song(&songs, query, duration) else {
        tracing::info!(
            target: "kopuz::lyrics",
            "paxsenix_netease no_match query={query:?} candidates={}",
            songs.len()
        );
        return None;
    };

    let rows = client
        .get(format!("{PAXSENIX_ROOT_URL}/netease/lyrics"))
        .query(&[("id", song.id.to_string()), ("word", "true".to_string())])
        .timeout(PAXSENIX_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_netease lyrics failed={error}"
            );
        })
        .ok()?
        .json::<Vec<PaxsenixNeteaseLyricLine>>()
        .await
        .map_err(|error| {
            tracing::info!(
                target: "kopuz::lyrics",
                "paxsenix_netease lyrics json_failed={error}"
            );
        })
        .ok()?;

    let lines = paxsenix_netease_to_lines(rows)?;
    if lines.iter().any(|line| !line.words.is_empty()) {
        Some(Lyrics::Synced(lines))
    } else {
        None
    }
}

fn best_paxsenix_netease_song<'a>(
    songs: &'a [PaxsenixNeteaseSong],
    query: &str,
    duration: u64,
) -> Option<&'a PaxsenixNeteaseSong> {
    songs
        .iter()
        .filter_map(|song| {
            let artists = song
                .artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let candidate = format!("{} {}", song.name, artists);
            let text_score = lyrics_match_score(&candidate, query);
            if text_score < 55.0 {
                return None;
            }

            let duration_score = match (duration, song.duration) {
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

            let provider_score = song.score.unwrap_or_default() / 10.0;
            Some((text_score + duration_score + provider_score, song))
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, song)| song)
}

fn paxsenix_netease_to_lines(rows: Vec<PaxsenixNeteaseLyricLine>) -> Option<Vec<LyricLine>> {
    let mut lines = rows
        .into_iter()
        .filter(|row| !row.background)
        .filter_map(|row| {
            let words = row
                .text
                .into_iter()
                .filter(|word| !word.text.trim().is_empty())
                .map(|word| LyricWord {
                    start_time: word.timestamp as f64 / 1000.0,
                    text: word.text,
                })
                .collect::<Vec<_>>();

            let text = words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<String>()
                .trim()
                .to_string();

            if text.is_empty() {
                None
            } else {
                Some(LyricLine {
                    start_time: row.timestamp as f64 / 1000.0,
                    text,
                    words,
                })
            }
        })
        .collect::<Vec<_>>();

    lines.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    (!lines.is_empty()).then_some(lines)
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
                    text,
                    words,
                });
            } else if !text.is_empty() {
                if let Some(last) = lines.last_mut() {
                    append_line_translation(last, &text);
                }
            }
            continue;
        }

        if text.is_empty() && words.is_empty() {
            continue;
        }

        for start_time in line_timestamps {
            lines.push(LyricLine {
                start_time,
                text: text.clone(),
                words: words.clone(),
            });
        }
    }

    lines.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut merged: Vec<LyricLine> = Vec::new();

    for line in lines {
        if let Some(last) = merged.last_mut() {
            if last.start_time == line.start_time {
                append_line_translation(last, &line.text);
                continue;
            }
        }

        merged.push(line);
    }

    merged
}

fn extract_line_timestamps(line: &str) -> (Vec<f64>, &str) {
    let mut timestamps = Vec::new();
    let mut rest = line.trim_start();

    loop {
        let Some(after_open) = rest.strip_prefix('[') else {
            break;
        };
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

fn parse_enhanced_words(content: &str) -> (String, Vec<LyricWord>) {
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
            words.push(LyricWord {
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
        words.push(LyricWord {
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
        assert!(lines[0].words.is_empty());
        assert_eq!(lines[1].start_time, 2.5);
        assert_eq!(lines[1].text, "World");
    }

    #[test]
    fn parses_enhanced_lrc_word_timestamps() {
        let lines = parse_lrc("[00:10.00]<00:10.10>Hello <00:10.60>world");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_time, 10.0);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[0].start_time, 10.1);
        assert_eq!(lines[0].words[0].text, "Hello ");
        assert_eq!(lines[0].words[1].start_time, 10.6);
        assert_eq!(lines[0].words[1].text, "world");
    }

    #[test]
    fn detects_word_timed_lyrics() {
        let line_only = Lyrics::Synced(parse_lrc("[00:01.00]Hello"));
        let word_timed = Lyrics::Synced(parse_lrc("[00:01.00]<00:01.10>Hello"));

        assert!(!has_word_timestamps(&line_only));
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
        assert_eq!(parsed[0].words.len(), 2);
        assert_eq!(parsed[0].words[0].start_time, 10.1);
        assert_eq!(parsed[0].words[0].text, "Hello ");
        assert_eq!(parsed[0].words[1].start_time, 10.6);
        assert_eq!(parsed[0].words[1].text, "world ");
    }

    #[test]
    fn converts_paxsenix_netease_word_lyrics() {
        let rows = vec![PaxsenixNeteaseLyricLine {
            text: vec![
                PaxsenixNeteaseLyricWord {
                    text: "Hello ".to_string(),
                    timestamp: 10100,
                },
                PaxsenixNeteaseLyricWord {
                    text: "world".to_string(),
                    timestamp: 10600,
                },
            ],
            timestamp: 10000,
            background: false,
        }];

        let lines = paxsenix_netease_to_lines(rows).expect("lyrics should convert");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].start_time, 10.0);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].words.len(), 2);
        assert_eq!(lines[0].words[0].start_time, 10.1);
        assert_eq!(lines[0].words[0].text, "Hello ");
        assert_eq!(lines[0].words[1].start_time, 10.6);
        assert_eq!(lines[0].words[1].text, "world");
    }

    #[test]
    fn paxsenix_netease_selector_checks_duration() {
        let songs = vec![
            PaxsenixNeteaseSong {
                id: 1,
                name: "Hello".to_string(),
                duration: Some(300_000),
                score: Some(100.0),
                artists: vec![PaxsenixNeteaseArtist {
                    name: "Adele".to_string(),
                }],
            },
            PaxsenixNeteaseSong {
                id: 2,
                name: "Hello".to_string(),
                duration: Some(295_000),
                score: Some(80.0),
                artists: vec![PaxsenixNeteaseArtist {
                    name: "Adele".to_string(),
                }],
            },
        ];

        let selected = best_paxsenix_netease_song(&songs, "Hello Adele", 295)
            .expect("a duration-matched result should be selected");

        assert_eq!(selected.id, 2);
    }
}
