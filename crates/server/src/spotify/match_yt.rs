//! Resolve a Spotify track to a playable **anonymous YouTube** stream.
//!
//! Spotify hands us only metadata (and, for non-Premium accounts, a 30-second
//! preview — see [`super::stream`]). To play the *full* song without a Spotify
//! Premium audio key, we treat Spotify purely as the catalog: take the track's
//! title/artist/duration, search YouTube Music **anonymously** (no cookies, no
//! account — `cookies = None`), pick the best-matching video, and resolve it
//! through the same [`crate::ytmusic::player`] path the YouTube backend uses.
//! The player then streams it via the existing googlevideo decode path.
//!
//! Matching is best-effort: we score candidates on duration closeness + title /
//! artist overlap and always return the top scorer (the user opted into "play
//! the best guess even if confidence is low"). The Spotify-track → video-id
//! match is cached process-wide so replays and skips don't re-search.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::ytmusic::player::{self, YtStreamInfo};
use crate::ytmusic::search;

/// Spotify base62 track id → resolved YouTube video id. Populated on first play.
fn cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve a Spotify track (base62 id) to an anonymous YouTube stream.
///
/// Fetches the Spotify tags, searches YouTube Music for the best match, then
/// resolves that video to a playable stream. The match is cached so a re-play
/// skips straight to stream resolution.
pub async fn resolve(
    token: &str,
    track_id: &str,
    known: Option<reader::Track>,
) -> Result<YtStreamInfo, String> {
    let cached = cache().lock().unwrap().get(track_id).cloned();
    if let Some(vid) = cached {
        tracing::info!(track = %track_id, video = %vid, "spotify→yt: cached match");
        return player::resolve(&vid, None).await;
    }

    // Prefer the metadata the caller already has (the track row from the DB) so
    // a cold play skips a Spotify round-trip; only hit the AP when it's absent.
    let meta = match known {
        Some(t) if !t.title.is_empty() => t,
        _ => super::metadata::track_meta(token.to_string(), track_id.to_string()).await?,
    };
    let query = format!("{} {}", meta.artist, meta.title);
    tracing::info!(track = %track_id, %query, "spotify→yt: searching YouTube for a full-track match");

    let candidates = search::music_search_tracks(&query, None).await?;
    let video_id = pick_best(&meta, &candidates).ok_or_else(|| {
        format!("no YouTube match for \"{}\" by {}", meta.title, meta.artist)
    })?;
    tracing::info!(track = %track_id, video = %video_id, "spotify→yt: matched");

    cache()
        .lock()
        .unwrap()
        .insert(track_id.to_string(), video_id.clone());
    player::resolve(&video_id, None).await
}

/// Pick the best-scoring candidate's video id, or `None` if the search was empty.
fn pick_best(target: &reader::Track, candidates: &[reader::Track]) -> Option<String> {
    candidates
        .iter()
        .map(|c| (score(target, c), c.id.key().into_owned()))
        .filter(|(_, vid)| !vid.is_empty())
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, vid)| vid)
}

/// Heuristic match score. Higher is better. Weighs (in descending importance)
/// title overlap, duration closeness, and artist overlap — duration is the
/// strongest guard against wrong picks (live cuts, extended mixes, the wrong
/// remix all drift in length).
fn score(target: &reader::Track, cand: &reader::Track) -> f32 {
    let title = jaccard(&norm_tokens(&target.title), &norm_tokens(&cand.title));

    // Match the Spotify artist against the candidate's artist *and* title — YT
    // music videos often fold the artist into the title ("Artist - Song").
    let want_artist = norm_tokens(&target.artist);
    let have_artist = {
        let mut t = norm_tokens(&cand.artist);
        t.extend(norm_tokens(&cand.title));
        t
    };
    let artist = if want_artist.is_empty() {
        0.0
    } else {
        let hits = want_artist.iter().filter(|w| have_artist.contains(*w)).count();
        hits as f32 / want_artist.len() as f32
    };

    let dur = match (target.duration, cand.duration) {
        (0, _) | (_, 0) => 0.0,
        (a, b) => {
            let delta = a.abs_diff(b) as f32;
            (1.0 - (delta / 30.0)).max(0.0) // full credit at 0s, none past 30s
        }
    };

    title * 3.0 + dur * 2.0 + artist * 2.0
}

/// Stop-words that add noise to title/artist matching (release adornments,
/// upload tags). Stripped before comparison.
const NOISE: &[&str] = &[
    "feat", "ft", "featuring", "official", "video", "audio", "lyrics", "lyric",
    "remaster", "remastered", "hd", "hq", "mv", "the", "a", "an",
];

/// Lowercase, split on non-alphanumerics, drop noise words → comparable tokens.
fn norm_tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !NOISE.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Jaccard similarity of two token sets (|∩| / |∪|), 0.0–1.0.
fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.iter().filter(|w| b.contains(*w)).count();
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f32 / union as f32
    }
}
