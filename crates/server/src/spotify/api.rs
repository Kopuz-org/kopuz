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
///
/// Newer Spotify apps get the renamed API surface: entries live at
/// `/playlists/{id}/items` (each wrapping its track under `item`), and the old
/// `/tracks` path returns 403 Forbidden. Older apps still serve `/tracks` with
/// the track under `track`, so parse accepts both wrapper keys.
pub async fn playlist_entries(access: &str, playlist_id: &str) -> Result<Vec<Track>, String> {
    let mut out = Vec::new();
    let mut offset: u32 = 0;
    loop {
        let body = get_json(
            access,
            &format!("{API}/playlists/{playlist_id}/items"),
            &[("limit", "50".to_string()), ("offset", offset.to_string())],
            "playlist entries",
        )
        .await?;
        let items = body["items"].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            break;
        }
        for entry in &items {
            let wrapped = if entry["item"].is_object() {
                &entry["item"]
            } else {
                &entry["track"]
            };
            if let Some(track) = parse_track(wrapped) {
                out.push(track);
            }
        }
        offset += 50;
        if next_offset(&body, offset - 50, items.len()).is_none() {
            break;
        }
    }
    Ok(out)
}

/// Every track of one album, in album order.
pub async fn album_tracks_full(access: &str, album_id: &str) -> Result<Vec<Track>, String> {
    album_remote(access, album_id).await.map(|a| a.tracks)
}

/// One album with header metadata and every track, for the remote album page
/// and discover tiles. The tracks endpoint returns simplified items without an
/// album object, so the album's own name/artwork is stamped into each before
/// the shared [`parse_track`] mapping.
pub async fn album_remote(
    access: &str,
    album_id: &str,
) -> Result<crate::source::RemoteAlbum, String> {
    let album = get_json(access, &format!("{API}/albums/{album_id}"), &[], "album").await?;
    let album_summary = serde_json::json!({
        "name": album["name"],
        "id": album["id"],
        "images": album["images"],
    });
    let stamp = |item: &Value| {
        let mut item = item.clone();
        item["album"] = album_summary.clone();
        parse_track(&item)
    };

    let first: Vec<Value> = album["tracks"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let total = album["tracks"]["total"].as_u64().unwrap_or(0);
    let mut fetched = first.len() as u64;
    let mut out: Vec<Track> = first.iter().filter_map(stamp).collect();

    while fetched < total {
        let body = get_json(
            access,
            &format!("{API}/albums/{album_id}/tracks"),
            &[("limit", PAGE.to_string()), ("offset", fetched.to_string())],
            "album tracks",
        )
        .await?;
        let items = body["items"].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            break;
        }
        fetched += items.len() as u64;
        out.extend(items.iter().filter_map(stamp));
    }
    Ok(crate::source::RemoteAlbum {
        browse_id: album_id.to_string(),
        title: album["name"].as_str().unwrap_or_default().to_string(),
        artist: album["artists"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|a| a["name"].as_str())
            .map(str::to_string),
        year: album["release_date"]
            .as_str()
            .map(|d| d.chars().take(4).collect()),
        thumbnail: first_image(&album["images"]),
        audio_playlist_id: None,
        tracks: out,
    })
}

/// Discover-home shelves from the personalization endpoints that survived
/// Spotify's dev-mode API cuts: top tracks (two ranges), recently played, and
/// new releases. Shelves whose endpoint fails (e.g. a token signed in before
/// the `user-top-read` / `user-read-recently-played` scopes were added) are
/// skipped, so the page degrades instead of erroring.
pub async fn discover_home(access: &str) -> Result<crate::ytmusic::discover::DiscoverHome, String> {
    use crate::ytmusic::discover::{DiscoverHome, DiscoverItem, DiscoverShelf};

    let song_shelf = |title: &str, tracks: Vec<Track>| DiscoverShelf {
        title: title.to_string(),
        strapline: None,
        more_browse_id: None,
        items: tracks
            .into_iter()
            .map(|t| DiscoverItem::Song(Box::new(t)))
            .collect(),
        is_song_list: false,
    };

    let mut shelves = Vec::new();

    match get_json(
        access,
        &format!("{API}/me/top/tracks"),
        &[
            ("limit", "20".to_string()),
            ("time_range", "short_term".to_string()),
        ],
        "top tracks",
    )
    .await
    {
        Ok(body) => {
            let tracks: Vec<Track> = body["items"]
                .as_array()
                .map(|items| items.iter().filter_map(parse_track).collect())
                .unwrap_or_default();
            if !tracks.is_empty() {
                shelves.push(song_shelf("On repeat", tracks));
            }
        }
        Err(e) => tracing::warn!(error = %e, "spotify discover: top tracks unavailable"),
    }

    match get_json(
        access,
        &format!("{API}/me/player/recently-played"),
        &[("limit", "50".to_string())],
        "recently played",
    )
    .await
    {
        Ok(body) => {
            let mut seen = std::collections::HashSet::new();
            let tracks: Vec<Track> = body["items"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| parse_track(&i["track"]))
                        .filter(|t| seen.insert(t.id.key().into_owned()))
                        .take(20)
                        .collect()
                })
                .unwrap_or_default();
            if !tracks.is_empty() {
                shelves.push(song_shelf("Jump back in", tracks));
            }
        }
        Err(e) => tracing::warn!(error = %e, "spotify discover: recently played unavailable"),
    }

    match get_json(
        access,
        &format!("{API}/me/top/tracks"),
        &[
            ("limit", "20".to_string()),
            ("time_range", "long_term".to_string()),
        ],
        "top tracks (all time)",
    )
    .await
    {
        Ok(body) => {
            let tracks: Vec<Track> = body["items"]
                .as_array()
                .map(|items| items.iter().filter_map(parse_track).collect())
                .unwrap_or_default();
            if !tracks.is_empty() {
                shelves.push(song_shelf("All-time favorites", tracks));
            }
        }
        Err(e) => tracing::warn!(error = %e, "spotify discover: all-time top tracks unavailable"),
    }

    if shelves.is_empty() {
        return Err(
            "Spotify discover returned nothing — if you signed in before this feature, \
             sign in again from Settings to grant the new permissions."
                .to_string(),
        );
    }
    Ok(DiscoverHome {
        shelves,
        continuation: None,
    })
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
                let mut album_tracks = Vec::new();
                if let Some(entries) = album["tracks"]["items"].as_array() {
                    for entry in entries {
                        if let Some(mut track) = parse_track(entry) {
                            track.album = a.title.clone();
                            track.album_id = a.id.clone();
                            if track.cover.is_none() {
                                track.cover = cover.clone();
                            }
                            album_tracks.push(track);
                        }
                    }
                }
                let total = album["tracks"]["total"].as_u64().unwrap_or(0);
                if (album_tracks.len() as u64) < total
                    && let Ok(full) = album_tracks_full(access, &a.id).await
                    && !full.is_empty()
                {
                    album_tracks = full;
                }
                tracks.extend(album_tracks);
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

/// Like / unlike a track via the consolidated save-library-items endpoint —
/// the old `PUT/DELETE /me/tracks` returns a bare 403 for newer apps.
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
            .request(method.clone(), format!("{API}/me/library"))
            .query(&[("uris", format!("spotify:track:{track_id}"))])
            .bearer_auth(access)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        if status.as_u16() == 429 && attempt < MAX_RETRIES {
            let wait = retry_after(&resp);
            attempt += 1;
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            continue;
        }
        if status.is_success() {
            return Ok(());
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("set saved failed ({status}): {body}"));
    }
}

/// A Spotify Connect device from `/me/player/devices`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectDevice {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub is_active: bool,
}

/// The account's currently available Connect devices.
pub async fn devices(access: &str) -> Result<Vec<ConnectDevice>, String> {
    let body = get_json(access, &format!("{API}/me/player/devices"), &[], "devices").await?;
    Ok(body["devices"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|d| {
                    Some(ConnectDevice {
                        id: d["id"].as_str().filter(|s| !s.is_empty())?.to_string(),
                        name: d["name"].as_str().unwrap_or_default().to_string(),
                        kind: d["type"].as_str().unwrap_or_default().to_string(),
                        is_active: d["is_active"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Move the live playback session to another Connect device.
pub async fn transfer_playback(access: &str, device_id: &str, play: bool) -> Result<(), String> {
    let resp = client()
        .put(format!("{API}/me/player"))
        .bearer_auth(access)
        .json(&serde_json::json!({ "device_ids": [device_id], "play": play }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(resp, "transfer playback").await.map(|_| ())
}

async fn player_put(access: &str, path: &str, query: &[(&str, String)]) -> Result<(), String> {
    let resp = client()
        .put(format!("{API}/me/player/{path}"))
        .query(query)
        .header(reqwest::header::CONTENT_LENGTH, 0)
        .bearer_auth(access)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(resp, path).await.map(|_| ())
}

/// Web-API transport for playback on a foreign Connect device (the in-app SDK
/// device is driven over the host WebSocket instead).
pub async fn player_pause(access: &str) -> Result<(), String> {
    player_put(access, "pause", &[]).await
}

pub async fn player_resume(access: &str) -> Result<(), String> {
    player_put(access, "play", &[]).await
}

pub async fn player_seek(access: &str, position_ms: u64) -> Result<(), String> {
    player_put(access, "seek", &[("position_ms", position_ms.to_string())]).await
}

pub async fn player_volume(access: &str, percent: u8) -> Result<(), String> {
    player_put(
        access,
        "volume",
        &[("volume_percent", percent.min(100).to_string())],
    )
    .await
}

/// A playback-state snapshot for polling a foreign Connect device; `None`
/// when nothing is playing anywhere on the account. `track` carries the full
/// now-playing item so the UI can hydrate title/artist/cover for a session it
/// didn't start itself (adopted device, or a track changed remotely).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    pub is_playing: bool,
    pub progress_ms: u64,
    pub duration_ms: u64,
    pub track_id: Option<String>,
    pub track: Option<Track>,
    pub device_id: Option<String>,
}

pub async fn player_state(access: &str) -> Result<Option<PlayerState>, String> {
    let body = get_json(access, &format!("{API}/me/player"), &[], "player state").await?;
    Ok(parse_player_state(&body))
}

/// Map a `/me/player` response body into a `PlayerState`. `None` for a null body
/// (nothing playing anywhere on the account).
fn parse_player_state(body: &Value) -> Option<PlayerState> {
    if body.is_null() {
        return None;
    }
    Some(PlayerState {
        is_playing: body["is_playing"].as_bool().unwrap_or(false),
        progress_ms: body["progress_ms"].as_u64().unwrap_or(0),
        duration_ms: body["item"]["duration_ms"].as_u64().unwrap_or(0),
        track_id: body["item"]["id"].as_str().map(str::to_string),
        track: parse_track(&body["item"]),
        device_id: body["device"]["id"].as_str().map(str::to_string),
    })
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
    fn player_state_carries_full_track_for_remote_sync() {
        let body = serde_json::json!({
            "is_playing": true,
            "progress_ms": 61_000,
            "device": {"id": "dev-other-pc"},
            "item": {
                "id": "trk9",
                "name": "The Headphones Ever",
                "duration_ms": 665_000,
                "artists": [{"name": "DankPods"}],
                "album": {"id": "alb9", "name": "Album", "images": [{"url": "https://i/x"}]}
            }
        });
        let st = parse_player_state(&body).expect("state");
        assert!(st.is_playing);
        assert_eq!(st.progress_ms, 61_000);
        assert_eq!(st.duration_ms, 665_000);
        assert_eq!(st.track_id.as_deref(), Some("trk9"));
        assert_eq!(st.device_id.as_deref(), Some("dev-other-pc"));
        let track = st.track.expect("track hydrated from /me/player item");
        assert_eq!(track.title, "The Headphones Ever");
        assert_eq!(track.artist, "DankPods");
    }

    #[test]
    fn player_state_none_when_idle() {
        assert!(parse_player_state(&Value::Null).is_none());
    }

    #[test]
    fn next_offset_stops_at_total() {
        let body = serde_json::json!({ "total": 50 });
        assert_eq!(next_offset(&body, 0, 50), None);
        let body = serde_json::json!({ "total": 120 });
        assert_eq!(next_offset(&body, 0, 50).as_deref(), Some("50"));
    }
}
