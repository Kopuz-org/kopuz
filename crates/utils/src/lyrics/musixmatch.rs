use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{
    Lyrics, has_usable_line_timing, lrc_has_usable_timing, lyrics_match_score, parse_lrc,
    timed_line_count, timed_part_count,
};

const MUSIXMATCH_ROOT_URL: &str = "https://apic-desktop.musixmatch.com/ws/1.1/";
const MUSIXMATCH_TIMEOUT: Duration = Duration::from_secs(3);

static MUSIXMATCH_TOKEN: OnceLock<Mutex<Option<MusixmatchToken>>> = OnceLock::new();

struct MusixmatchToken {
    value: String,
    expires_at_ms: u128,
}

pub(super) async fn fetch_from_musixmatch_enhanced(artist: &str, title: &str) -> Option<Lyrics> {
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

pub(super) fn musixmatch_richsync_to_lrc(body: &str) -> Option<String> {
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
