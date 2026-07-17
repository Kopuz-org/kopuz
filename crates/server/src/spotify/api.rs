//! Spotify Web API calls: issuing playback on the SDK Connect device, plus the
//! read-only data pulls (search, liked songs, playlists, saved albums) that back
//! the [`crate::source::spotify::SpotifySource`] `MediaSource` impl.
//!
//! kopuz owns the queue, so we play one track URI at a time on the device rather
//! than handing Spotify a context. Everything maps a Web API track object into a
//! `reader::Track` via [`parse_track`], with the artwork URL stored raw in
//! `Track.cover` (the cover seam passes Spotify URLs straight through).

use config::MusicService;
use reader::Track;
use reader::models::TrackId;
use serde_json::Value;

const API: &str = "https://api.spotify.com/v1";
/// Web API page cap; 50 is the max for most list endpoints.
const PAGE: u32 = 50;

/// Shared client — keeps one TLS pool warm across the many library-sync and
/// playback calls (fewer connections = less chance of tripping the rate limit).
fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Seconds to wait after a 429, from the `Retry-After` header (clamped so a
/// hostile value can't stall us forever). Falls back to 2s.
fn retry_after(resp: &reqwest::Response) -> u64 {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(2)
        .clamp(1, 30)
}

/// `GET` a JSON endpoint, transparently backing off and retrying on 429. The
/// library sync fires these back-to-back, so a single rate-limit hit must not
/// fail the whole pull.
async fn get_json(
    access: &str,
    url: &str,
    query: &[(&str, String)],
    ctx: &str,
) -> Result<Value, String> {
    const MAX_RETRIES: u32 = 4;
    let mut attempt = 0;
    loop {
        let resp = client()
            .get(url)
            .query(query)
            .bearer_auth(access)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().as_u16() == 429 && attempt < MAX_RETRIES {
            let wait = retry_after(&resp);
            attempt += 1;
            tracing::warn!(
                ctx,
                wait_secs = wait,
                attempt,
                "spotify rate limited; backing off"
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            continue;
        }
        return ok_json(resp, ctx).await;
    }
}

/// Parse a 2xx JSON body, else a descriptive error carrying the status + body.
async fn ok_json(resp: reqwest::Response, ctx: &str) -> Result<Value, String> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("{ctx} failed ({status}): {body}"));
    }
    if status.as_u16() == 204 {
        return Ok(Value::Null);
    }
    resp.json::<Value>()
        .await
        .map_err(|e| format!("{ctx}: couldn't parse response: {e}"))
}

/// Start playback of `uris` on the given Connect device. kopuz passes a single
/// track URI; Spotify plays it immediately.
///
/// Retries on 429 (rate limit — often from a concurrent library sync), on 404
/// (the SDK device hasn't finished registering with Connect yet), and on
/// transient 502/503, so a momentary hiccup doesn't drop the track.
pub async fn start_playback(access: &str, device_id: &str, uris: &[String]) -> Result<(), String> {
    let body = serde_json::json!({ "uris": uris });
    const MAX_RETRIES: u32 = 5;
    let mut attempt = 0;
    loop {
        let resp = client()
            .put(format!("{API}/me/player/play"))
            .query(&[("device_id", device_id)])
            .bearer_auth(access)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let code = status.as_u16();
        if attempt < MAX_RETRIES && matches!(code, 429 | 404 | 502 | 503) {
            let wait = if code == 429 {
                retry_after(&resp)
            } else {
                // Device warming up / transient — short growing backoff.
                (u64::from(attempt) + 1).min(3)
            };
            attempt += 1;
            tracing::warn!(
                code,
                wait_secs = wait,
                attempt,
                "spotify start_playback retrying"
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            continue;
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("start_playback failed ({status}): {body}"));
    }
}

/// Lightweight auth probe used by `validate`.
pub async fn me(access: &str) -> Result<(), String> {
    get_json(access, &format!("{API}/me"), &[], "me")
        .await
        .map(|_| ())
}

/// Search tracks (and albums) for `query`.
pub async fn search(access: &str, query: &str) -> Result<(Vec<Track>, Vec<reader::Album>), String> {
    // Development-mode apps (the norm here — each user registers their own) are
    // capped at limit=10 on /search; anything higher is a 400 "Invalid limit".
    // Library endpoints still take 50, so only search uses the small page.
    let body = get_json(
        access,
        &format!("{API}/search"),
        &[
            ("q", query.to_string()),
            ("type", "track,album".to_string()),
            ("limit", "10".to_string()),
        ],
        "search",
    )
    .await?;

    let tracks = body["tracks"]["items"]
        .as_array()
        .map(|items| items.iter().filter_map(parse_track).collect())
        .unwrap_or_default();
    let albums = body["albums"]["items"]
        .as_array()
        .map(|items| items.iter().filter_map(parse_album).collect())
        .unwrap_or_default();
    Ok((tracks, albums))
}

/// One page of liked songs. The cursor is the next `offset` as a string; `None`
/// starts at 0 and a returned `None` means the list is exhausted.
pub async fn saved_tracks_page(
    access: &str,
    cursor: Option<&str>,
) -> Result<(Vec<Track>, Option<String>), String> {
    let offset: u32 = cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = get_json(
        access,
        &format!("{API}/me/tracks"),
        &[("limit", PAGE.to_string()), ("offset", offset.to_string())],
        "saved tracks",
    )
    .await?;

    // Each item wraps the track under `track`.
    let tracks: Vec<Track> = body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|i| parse_track(&i["track"]))
                .collect()
        })
        .unwrap_or_default();
    let next = next_offset(&body, offset, tracks.len());
    Ok((tracks, next))
}

/// The user's playlists (owned + followed).
pub async fn list_playlists(access: &str) -> Result<Vec<PlaylistSummary>, String> {
    let mut out = Vec::new();
    let mut offset: u32 = 0;
    loop {
        let body = get_json(
            access,
            &format!("{API}/me/playlists"),
            &[("limit", PAGE.to_string()), ("offset", offset.to_string())],
            "playlists",
        )
        .await?;
        let items = body["items"].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            break;
        }
        for p in &items {
            let Some(id) = p["id"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            out.push(PlaylistSummary {
                id: id.to_string(),
                name: p["name"].as_str().unwrap_or_default().to_string(),
                image: first_image(&p["images"]),
            });
        }
        offset += PAGE;
        if next_offset(&body, offset - PAGE, items.len()).is_none() {
            break;
        }
    }
    Ok(out)
}

/// All tracks in a playlist, in order.
pub async fn playlist_entries(access: &str, playlist_id: &str) -> Result<Vec<Track>, String> {
    let mut out = Vec::new();
    let mut offset: u32 = 0;
    loop {
        let body = get_json(
            access,
            &format!("{API}/playlists/{playlist_id}/tracks"),
            &[("limit", "100".to_string()), ("offset", offset.to_string())],
            "playlist entries",
        )
        .await?;
        let items = body["items"].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            break;
        }
        for entry in &items {
            if let Some(track) = parse_track(&entry["track"]) {
                out.push(track);
            }
        }
        offset += 100;
        if next_offset(&body, offset - 100, items.len()).is_none() {
            break;
        }
    }
    Ok(out)
}

/// Saved albums with their tracks, for the library sync snapshot.
pub async fn saved_albums(access: &str) -> Result<(Vec<reader::Album>, Vec<Track>), String> {
    let mut albums = Vec::new();
    let mut tracks = Vec::new();
    let mut offset: u32 = 0;
    loop {
        let body = get_json(
            access,
            &format!("{API}/me/albums"),
            &[("limit", PAGE.to_string()), ("offset", offset.to_string())],
            "saved albums",
        )
        .await?;
        let items = body["items"].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            break;
        }
        for item in &items {
            let album = &item["album"];
            if let Some(a) = parse_album(album) {
                let cover = first_image(&album["images"]);
                // The album payload embeds a tracks page; each entry lacks its
                // own `album` object, so stitch the parent album's data in.
                if let Some(entries) = album["tracks"]["items"].as_array() {
                    for entry in entries {
                        if let Some(mut track) = parse_track(entry) {
                            track.album = a.title.clone();
                            track.album_id = a.id.clone();
                            if track.cover.is_none() {
                                track.cover = cover.clone();
                            }
                            tracks.push(track);
                        }
                    }
                }
                albums.push(a);
            }
        }
        offset += PAGE;
        if next_offset(&body, offset - PAGE, items.len()).is_none() {
            break;
        }
    }
    Ok((albums, tracks))
}

/// Like / unlike a track.
pub async fn set_saved(access: &str, track_id: &str, on: bool) -> Result<(), String> {
    let method = if on {
        reqwest::Method::PUT
    } else {
        reqwest::Method::DELETE
    };
    const MAX_RETRIES: u32 = 4;
    let mut attempt = 0;
    loop {
        let resp = client()
            .request(method.clone(), format!("{API}/me/tracks"))
            .query(&[("ids", track_id)])
            .bearer_auth(access)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().as_u16() == 429 && attempt < MAX_RETRIES {
            let wait = retry_after(&resp);
            attempt += 1;
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            continue;
        }
        return ok_json(resp, "set saved").await.map(|_| ());
    }
}

/// Metadata for a Spotify playlist.
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
}

/// Compute the next-page cursor: `Some(offset+count)` while more remain, else
/// `None`. Uses `total` when present, otherwise "a full page implies more".
fn next_offset(body: &Value, offset: u32, count: usize) -> Option<String> {
    if count == 0 {
        return None;
    }
    let next = offset + count as u32;
    match body["total"].as_u64() {
        Some(total) if (next as u64) >= total => None,
        Some(_) => Some(next.to_string()),
        // No total (e.g. search): a short page means we're done.
        None if (count as u32) < PAGE => None,
        None => Some(next.to_string()),
    }
}

fn first_image(images: &Value) -> Option<String> {
    images
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|img| img["url"].as_str())
        .map(str::to_string)
}

/// Map a Web API track object into a `reader::Track`. Returns `None` for a null
/// or id-less object (e.g. a removed/local playlist entry).
pub fn parse_track(item: &Value) -> Option<Track> {
    let id = item["id"].as_str().filter(|s| !s.is_empty())?;

    let artists: Vec<String> = item["artists"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let artist = artists.first().cloned().unwrap_or_default();

    let album_obj = &item["album"];
    let album = album_obj["name"].as_str().unwrap_or_default().to_string();
    let album_id = album_obj["id"].as_str().unwrap_or_default().to_string();
    let cover = first_image(&album_obj["images"]);

    Some(Track {
        id: TrackId::Server {
            service: MusicService::Spotify,
            item_id: id.to_string(),
        },
        cover,
        album_id,
        title: item["name"].as_str().unwrap_or_default().to_string(),
        artist,
        album,
        duration: item["duration_ms"].as_u64().unwrap_or(0) / 1000,
        khz: 0,
        bitrate: 0,
        track_number: item["track_number"].as_u64().map(|n| n as u32),
        disc_number: item["disc_number"].as_u64().map(|n| n as u32),
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists,
    })
}

fn parse_album(item: &Value) -> Option<reader::Album> {
    let id = item["id"].as_str().filter(|s| !s.is_empty())?;
    let artist = item["artists"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|a| a["name"].as_str())
        .unwrap_or_default()
        .to_string();
    // release_date is "YYYY" or "YYYY-MM-DD".
    let year = item["release_date"]
        .as_str()
        .and_then(|d| d.get(0..4))
        .and_then(|y| y.parse::<u16>().ok())
        .unwrap_or(0);
    Some(reader::Album {
        id: id.to_string(),
        title: item["name"].as_str().unwrap_or_default().to_string(),
        artist,
        genre: String::new(),
        year,
        cover_path: None,
        manual_cover: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_track_maps_core_fields() {
        let v = serde_json::json!({
            "id": "abc123",
            "name": "Song",
            "duration_ms": 200_000,
            "track_number": 3,
            "disc_number": 1,
            "artists": [{"name": "A"}, {"name": "B"}],
            "album": {
                "id": "alb1",
                "name": "Album",
                "images": [{"url": "https://i.scdn.co/image/x"}]
            }
        });
        let t = parse_track(&v).expect("track");
        assert_eq!(t.id.key(), "abc123");
        assert_eq!(t.title, "Song");
        assert_eq!(t.duration, 200);
        assert_eq!(t.artist, "A");
        assert_eq!(t.artists, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(t.album, "Album");
        assert_eq!(t.album_id, "alb1");
        assert_eq!(t.cover.as_deref(), Some("https://i.scdn.co/image/x"));
    }

    #[test]
    fn parse_track_rejects_null() {
        assert!(parse_track(&Value::Null).is_none());
        assert!(parse_track(&serde_json::json!({"name": "no id"})).is_none());
    }

    #[test]
    fn next_offset_stops_at_total() {
        let body = serde_json::json!({ "total": 50 });
        assert_eq!(next_offset(&body, 0, 50), None);
        let body = serde_json::json!({ "total": 120 });
        assert_eq!(next_offset(&body, 0, 50).as_deref(), Some("50"));
    }
}
