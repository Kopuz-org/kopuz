use percent_encoding::NON_ALPHANUMERIC;
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct LrcLibResponse {
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub start_time: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lyrics {
    Synced(Vec<LyricLine>),
    Plain(String),
}

const LYRICS_CACHE_CAPACITY: usize = 256;

static LYRICS_CACHE: OnceLock<Mutex<LyricsCache>> = OnceLock::new();
static LYRICS_INFLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

struct LyricsCache {
    entries: HashMap<String, Option<Lyrics>>,
    order: VecDeque<String>,
    capacity: usize,
}

struct LyricsInflightGuard {
    key: String,
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

// --- Public API ---

/// Fetch lyrics for a track, trying sources in priority order:
/// 1. Local .lrc file alongside the audio file (local tracks only)
/// 2. Jellyfin or Subsonic server lyrics API (server tracks)
/// 3. lrclib.net (fallback for all tracks)
///
/// For Jellyfin: `server_token` = access token, `server_user_id` = user_id (unused for lyrics)
/// For Subsonic: `server_token` = password, `server_user_id` = username
pub async fn fetch_lyrics(
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
    track_path: &str,
    server_url: Option<&str>,
    server_token: Option<&str>,
    server_user_id: Option<&str>,
) -> Option<Lyrics> {
    let cache_key = lyrics_cache_key(artist, title, album, duration, track_path);
    if let Some(cached) = lyrics_cache()
        .lock()
        .ok()
        .and_then(|mut cache| cache.get_cloned(&cache_key))
    {
        return cached;
    }

    let _inflight_guard = if try_begin_lyrics_fetch(&cache_key) {
        Some(LyricsInflightGuard {
            key: cache_key.clone(),
        })
    } else {
        for _ in 0..100 {
            crate::sleep(Duration::from_millis(50)).await;
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

    // 1. Local .lrc file (only for local tracks)
    if !is_server {
        if let Some(lyrics) = fetch_local_lrc(track_path).await {
            if let Ok(mut cache) = lyrics_cache().lock() {
                cache.put(cache_key, Some(lyrics.clone()));
            }
            return Some(lyrics);
        }
    }

    // 2. Server lyrics
    if let Some(server_url) = server_url {
        if track_path.starts_with("jellyfin:") {
            if let (Some(item_id), Some(token)) =
                (extract_server_id(track_path, "jellyfin:"), server_token)
            {
                if let Some(lyrics) = fetch_jellyfin_lyrics(&item_id, server_url, token).await {
                    if let Ok(mut cache) = lyrics_cache().lock() {
                        cache.put(cache_key, Some(lyrics.clone()));
                    }
                    return Some(lyrics);
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
                if let Some(lyrics) =
                    fetch_subsonic_lyrics(&song_id, server_url, username, password, artist, title)
                        .await
                {
                    if let Ok(mut cache) = lyrics_cache().lock() {
                        cache.put(cache_key, Some(lyrics.clone()));
                    }
                    return Some(lyrics);
                }
            }
        }
    }

    // 3. lrclib fallback
    let fetched = fetch_from_lrclib(artist, title, album, duration).await;
    if let Ok(mut cache) = lyrics_cache().lock() {
        cache.put(cache_key, fetched.clone());
    }
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
    let lrc_path = std::path::Path::new(audio_path).with_extension("lrc");
    let content = std::fs::read_to_string(&lrc_path).ok()?;
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

    let resp = client.get(&url).query(&params).send().await.ok()?;
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

    let resp = client.get(&url).query(&params).send().await.ok()?;
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
    let res = client
        .get(&url)
        .header("User-Agent", concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .ok()?;

    if res.status().is_success() {
        if let Ok(data) = res.json::<LrcLibResponse>().await {
            if let Some(lyrics) = extract_from_lrclib_response(&data) {
                return Some(lyrics);
            }
        }
    }

    let search_url = format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        percent_encoding::utf8_percent_encode(title, NON_ALPHANUMERIC),
        percent_encoding::utf8_percent_encode(artist, NON_ALPHANUMERIC)
    );
    let search_res = client
        .get(&search_url)
        .header("User-Agent", concat!("Kopuz/", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .ok()?;

    if search_res.status().is_success() {
        if let Ok(results) = search_res.json::<Vec<LrcLibResponse>>().await {
            for data in results {
                if let Some(lyrics) = extract_from_lrclib_response(&data) {
                    return Some(lyrics);
                }
            }
        }
    }

    None
}

fn extract_from_lrclib_response(data: &LrcLibResponse) -> Option<Lyrics> {
    if let Some(synced) = &data.synced_lyrics {
        if !synced.trim().is_empty() {
            return Some(Lyrics::Synced(parse_lrc(synced)));
        }
    }
    if let Some(plain) = &data.plain_lyrics {
        if !plain.trim().is_empty() {
            return Some(Lyrics::Plain(plain.clone()));
        }
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

fn parse_lrc(lrc_text: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();

    for line in lrc_text.lines() {
        let mut current_pos = 0;
        let mut current_timestamps = Vec::new();
        let chars: Vec<char> = line.chars().collect();
        let mut text_start = 0;

        while current_pos < chars.len() {
            if chars[current_pos] == '[' {
                let mut j = current_pos + 1;
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ']' {
                    let time_str: String = chars[current_pos + 1..j].iter().collect();
                    if let Some(time) = parse_lrc_time(&time_str) {
                        let text: String = chars[text_start..current_pos].iter().collect();
                        let text = text.trim().to_string();
                        if !text.is_empty() {
                            for t in &current_timestamps {
                                lines.push(LyricLine {
                                    start_time: *t,
                                    text: text.clone(),
                                });
                            }
                            current_timestamps.clear();
                        }

                        current_timestamps.push(time);
                        current_pos = j + 1;
                        text_start = current_pos;
                        continue;
                    } else if time_str.chars().any(|c| c.is_ascii_alphabetic())
                        && time_str.contains(':')
                    {
                        current_pos = j + 1;
                        text_start = current_pos;
                        continue;
                    }
                }
            }
            current_pos += 1;
        }

        let text: String = chars[text_start..].iter().collect();
        let text = text.trim().to_string();

        if current_timestamps.is_empty() {
            if let Some(last) = lines.last_mut() {
                append_translation(&mut last.text, &text);
            }
            continue;
        }

        for t in current_timestamps {
            lines.push(LyricLine {
                start_time: t,
                text: text.clone(),
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
                append_translation(&mut last.text, &line.text);
                continue;
            }
        }

        merged.push(line);
    }

    merged
}

fn parse_lrc_time(time_str: &str) -> Option<f64> {
    let parts: Vec<&str> = time_str.split(|c| c == ':' || c == '.').collect();
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
