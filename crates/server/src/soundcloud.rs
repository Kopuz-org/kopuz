//! SoundCloud integration via the public `api-v2` web-player endpoint.
//!
//! Unlike Apple Music / Spotify, SoundCloud streams are **not** DRM'd: every
//! publicly streamable track exposes a `progressive` transcoding that resolves
//! to a plain MP3 URL on `cf-media.sndcdn.com`. Those flow through the exact
//! same Symphonia/cpal decode path as every other backend, identically on
//! Linux/macOS/Windows. So this is genuine full-track playback, not previews.
//!
//! There is no public, registerable API key anymore, so — like every
//! SoundCloud client (`yt-dlp`, `scdl`, …) — we lift the web player's
//! `client_id` out of its JS bundles at runtime and cache it. The id rotates
//! occasionally; any `4xx` triggers one forced re-scrape before giving up.
//!
//! Track encoding mirrors the YouTube Music backend: the path is
//! `soundcloud:<trackId>:urlhex_<artwork-url-hex>` so the shared cover resolver
//! (`utils::jellyfin_image`) can decode artwork synchronously, and the stream
//! URL is resolved lazily via [`resolve_stream`] (the controller tags it with a
//! `__SC_PENDING:` sentinel, just like `__YT_PENDING:`).

use std::collections::HashSet;
use std::path::PathBuf;

use reader::models::Track;
use serde_json::Value;
use tokio::sync::Mutex;

pub const SOURCE_PREFIX: &str = "soundcloud";

/// SoundCloud's internal web-player API. Keyless apart from the scraped
/// `client_id` query param.
const API_V2: &str = "https://api-v2.soundcloud.com";

/// The public developer API host. Unlike `api-v2` it isn't behind DataDome, so
/// authenticated writes (like/unlike) go through here.
const API_V1: &str = "https://api.soundcloud.com";

/// The web player whose HTML/JS bundles carry the `client_id`.
const WEB_HOST: &str = "https://soundcloud.com";

/// Process-wide cached `client_id`. `None` until first scrape; a `4xx` from the
/// API forces a re-scrape (the id rotates server-side every so often).
static CLIENT_ID: Mutex<Option<String>> = Mutex::const_new(None);

/// Chrome-on-macOS UA. SoundCloud's api-v2 write endpoints (like/repost) 403 any
/// request that doesn't look like the web player, so every request carries it.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_default()
}

/// Resolve the web player's `client_id`, scraping it on first use (or when
/// `force` is set after a stale-id error) and caching it for the process.
async fn client_id(http: &reqwest::Client, force: bool) -> Result<String, String> {
    let mut guard = CLIENT_ID.lock().await;
    if !force && let Some(id) = guard.as_ref() {
        return Ok(id.clone());
    }
    let id = scrape_client_id(http).await?;
    *guard = Some(id.clone());
    Ok(id)
}

/// Pull a fresh `client_id` out of the web player's JavaScript bundles.
///
/// The homepage references a handful of `*.sndcdn.com/assets/*.js` chunks; the
/// id lives in one of them as `client_id:"<32+ chars>"`. It's usually in one of
/// the later bundles, so we scan them newest-first and stop at the first hit.
async fn scrape_client_id(http: &reqwest::Client) -> Result<String, String> {
    let html = http
        .get(WEB_HOST)
        .send()
        .await
        .map_err(|e| format!("SoundCloud homepage HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("SoundCloud homepage HTTP: {e}"))?
        .text()
        .await
        .map_err(|e| format!("SoundCloud homepage body: {e}"))?;

    let mut scripts = Vec::new();
    for chunk in html.split("<script") {
        if let Some(src) = extract_attr(chunk, "src")
            && src.contains("sndcdn.com/assets/")
            && src.ends_with(".js")
        {
            scripts.push(src.to_string());
        }
    }

    for src in scripts.iter().rev() {
        if let Ok(resp) = http.get(src).send().await
            && let Ok(js) = resp.text().await
            && let Some(id) = find_client_id(&js)
        {
            return Ok(id);
        }
    }
    Err("SoundCloud: couldn't extract a client_id from the web player".to_string())
}

/// Read the value of an HTML `attr="…"` from a fragment, if present.
fn extract_attr<'a>(chunk: &'a str, attr: &str) -> Option<&'a str> {
    let key = format!("{attr}=\"");
    let start = chunk.find(&key)? + key.len();
    let rest = &chunk[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Locate a `client_id` literal in a JS bundle. SoundCloud emits it as
/// `client_id:"…"` (and occasionally as a quoted JSON key); both are covered.
fn find_client_id(js: &str) -> Option<String> {
    for marker in ["client_id:\"", "\"client_id\":\"", "client_id=\""] {
        if let Some(pos) = js.find(marker) {
            let rest = &js[pos + marker.len()..];
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if id.len() >= 16 {
                return Some(id);
            }
        }
    }
    None
}

/// Bump a SoundCloud artwork URL (".../…-large.jpg", 100px) up to a
/// display-friendly 500px. The CDN honours the `-t500x500` size token, so this
/// is a plain string swap — falls back to the original URL otherwise.
fn upscale_artwork(url: &str) -> String {
    url.replace("-large.", "-t500x500.")
}

/// Hex-encode a remote artwork URL into the `urlhex_<hex>` tag the shared cover
/// resolver decodes. Inlined here for the same reason `ytmusic` inlines its own
/// copy: the `server` crate doesn't depend on `utils`.
fn encode_cover_url(url: &str) -> String {
    format!("urlhex_{}", hex::encode(url.as_bytes()))
}

/// Search the SoundCloud catalog for tracks matching `query`.
#[tracing::instrument(name = "soundcloud.search", fields(query = %query))]
pub async fn search_tracks(query: &str) -> Result<Vec<Track>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let http = http_client();

    let resp = match api_search(&http, query, &client_id(&http, false).await?).await {
        Ok(v) => v,
        Err(_) => api_search(&http, query, &client_id(&http, true).await?).await?,
    };

    let collection = resp
        .get("collection")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::with_capacity(collection.len());
    let mut seen = HashSet::new();
    for item in &collection {
        if item.get("kind").and_then(|v| v.as_str()) != Some("track") {
            continue;
        }
        if let Some(track) = parse_track(item) {
            let id = track_id(&track);
            if !id.is_empty() && seen.insert(id) {
                out.push(track);
            }
        }
    }
    Ok(out)
}

async fn api_search(http: &reqwest::Client, query: &str, cid: &str) -> Result<Value, String> {
    http.get(format!("{API_V2}/search/tracks"))
        .query(&[("q", query), ("client_id", cid), ("limit", "50")])
        .send()
        .await
        .map_err(|e| format!("SoundCloud search HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("SoundCloud search HTTP: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("SoundCloud search JSON: {e}"))
}

/// A resolved, playable SoundCloud stream. Progressive is the keyless 128 kbps
/// MP3 every track exposes; `HlsAac` is the 256 kbps AAC HLS playlist that only
/// surfaces for an authenticated SoundCloud Go+ subscriber.
pub enum ResolvedStream {
    Progressive(String),
    HlsAac(String),
}

/// Resolve a track id to a playable stream via the `/tracks` lookup endpoint,
/// called lazily at play time. When `token` is an authenticated Go+ session the
/// track's AAC HLS transcoding is preferred for higher quality; otherwise it
/// falls back to the universal progressive MP3.
#[tracing::instrument(name = "soundcloud.resolve_stream", skip(token), fields(track_id = %track_id))]
pub async fn resolve_stream(track_id: &str, token: Option<&str>) -> Result<ResolvedStream, String> {
    let http = http_client();

    let track = match lookup_track(&http, track_id, &client_id(&http, false).await?, token).await {
        Ok(v) => v,
        Err(_) => lookup_track(&http, track_id, &client_id(&http, true).await?, token).await?,
    };

    let transcodings = track
        .get("media")
        .and_then(|m| m.get("transcodings"))
        .and_then(|t| t.as_array())
        .ok_or("SoundCloud track exposes no media transcodings")?;

    let track_auth = track.get("track_authorization").and_then(|v| v.as_str());

    if let Some(tc) = transcodings.iter().find(|tc| {
        transcoding_protocol(tc) == Some("hls") && transcoding_mime(tc) == Some("audio/mp4")
    }) {
        // Best-effort: if the AAC/HLS transcoding can't be resolved, fall
        // through to the progressive MP3 stream rather than failing the play.
        if let Some(hls_url) = tc.get("url").and_then(|v| v.as_str()) {
            match resolve_media_url(&http, hls_url, track_auth, token).await {
                Ok(media) => return Ok(ResolvedStream::HlsAac(media)),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "SoundCloud HLS resolve failed; falling back to progressive"
                    );
                }
            }
        }
    }

    let progressive_url = transcodings
        .iter()
        .find(|tc| transcoding_protocol(tc) == Some("progressive"))
        .and_then(|tc| tc.get("url"))
        .and_then(|v| v.as_str())
        .ok_or("SoundCloud track has no progressive (non-HLS) stream")?;
    let media = resolve_media_url(&http, progressive_url, track_auth, token).await?;
    Ok(ResolvedStream::Progressive(media))
}

/// Exchange a transcoding URL for its time-limited CDN media URL (a direct MP3
/// for progressive, or an `.m3u8` playlist for HLS).
async fn resolve_media_url(
    http: &reqwest::Client,
    transcoding_url: &str,
    track_auth: Option<&str>,
    token: Option<&str>,
) -> Result<String, String> {
    let cid = client_id(http, false).await?;
    let mut req = apply_auth(
        http.get(transcoding_url)
            .query(&[("client_id", cid.as_str())]),
        token,
    );
    if let Some(auth) = track_auth {
        req = req.query(&[("track_authorization", auth)]);
    }

    let resolved = req
        .send()
        .await
        .map_err(|e| format!("SoundCloud stream resolve HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("SoundCloud stream resolve HTTP: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("SoundCloud stream resolve JSON: {e}"))?;

    resolved
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "SoundCloud returned no stream URL for this track".to_string())
}

async fn lookup_track(
    http: &reqwest::Client,
    id: &str,
    cid: &str,
    token: Option<&str>,
) -> Result<Value, String> {
    apply_auth(
        http.get(format!("{API_V2}/tracks/{id}"))
            .query(&[("client_id", cid)]),
        token,
    )
    .send()
    .await
    .map_err(|e| format!("SoundCloud lookup HTTP: {e}"))?
    .error_for_status()
    .map_err(|e| format!("SoundCloud lookup HTTP: {e}"))?
    .json::<Value>()
    .await
    .map_err(|e| format!("SoundCloud lookup JSON: {e}"))
}

/// Attach the web-app `Authorization: OAuth <token>` header for a signed-in
/// session. A `None`/empty token leaves the request anonymous (keyless mode).
fn apply_auth(req: reqwest::RequestBuilder, token: Option<&str>) -> reqwest::RequestBuilder {
    match token {
        Some(t) if !t.is_empty() => req.header("Authorization", format!("OAuth {t}")),
        _ => req,
    }
}

fn transcoding_protocol(tc: &Value) -> Option<&str> {
    tc.get("format")
        .and_then(|f| f.get("protocol"))
        .and_then(|p| p.as_str())
}

fn transcoding_mime(tc: &Value) -> Option<&str> {
    tc.get("format")
        .and_then(|f| f.get("mime_type"))
        .and_then(|m| m.as_str())
}

/// Pull the id segment out of a `soundcloud:<id>[:tag]` track path.
fn track_id(t: &Track) -> String {
    t.path
        .to_string_lossy()
        .split(':')
        .nth(1)
        .unwrap_or("")
        .to_string()
}

fn parse_track(item: &Value) -> Option<Track> {
    let track_id = item.get("id").and_then(|v| v.as_u64())?;

    let has_progressive = item
        .get("media")
        .and_then(|m| m.get("transcodings"))
        .and_then(|t| t.as_array())
        .is_some_and(|arr| {
            arr.iter()
                .any(|tc| transcoding_protocol(tc) == Some("progressive"))
        });
    if !has_progressive {
        return None;
    }

    let title = item
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let artist = item
        .get("user")
        .and_then(|u| u.get("username"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let artwork = item
        .get("artwork_url")
        .and_then(|v| v.as_str())
        .or_else(|| {
            item.get("user")
                .and_then(|u| u.get("avatar_url"))
                .and_then(|v| v.as_str())
        })
        .filter(|s| !s.is_empty())
        .map(upscale_artwork);

    let duration = item
        .get("full_duration")
        .and_then(|v| v.as_u64())
        .or_else(|| item.get("duration").and_then(|v| v.as_u64()))
        .map(|ms| ms / 1000)
        .unwrap_or(0);

    let cover_tag = artwork.as_ref().map(|url| encode_cover_url(url));
    let path = match &cover_tag {
        Some(tag) => PathBuf::from(format!("{SOURCE_PREFIX}:{track_id}:{tag}")),
        None => PathBuf::from(format!("{SOURCE_PREFIX}:{track_id}")),
    };
    let album_id = match &cover_tag {
        Some(tag) => format!("{SOURCE_PREFIX}:_:{tag}"),
        None => format!("{SOURCE_PREFIX}:_"),
    };

    Some(Track {
        path,
        album_id,
        title,
        artist: artist.clone(),
        album: String::new(),
        duration,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: if artist.is_empty() {
            Vec::new()
        } else {
            vec![artist]
        },
    })
}

/// A SoundCloud playlist as listed in the user's library — enough to seed a
/// store entry; the tracks are loaded lazily via [`get_playlist_entries`].
pub struct PlaylistSummary {
    pub id: String,
    pub title: String,
    pub artwork_url: Option<String>,
}

async fn auth_get_json(
    http: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> Result<Value, String> {
    apply_auth(http.get(url), token)
        .send()
        .await
        .map_err(|e| format!("SoundCloud API HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("SoundCloud API HTTP: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("SoundCloud API JSON: {e}"))
}

/// Fetch the signed-in user's profile (`/me`).
pub async fn get_me(token: &str) -> Result<Value, String> {
    let http = http_client();
    let cid = client_id(&http, false).await?;
    auth_get_json(&http, &format!("{API_V2}/me?client_id={cid}"), Some(token)).await
}

/// Resolve a signed-in session to its numeric user id (used as `user_id`).
pub async fn derive_user_id(token: &str) -> Option<String> {
    get_me(token)
        .await
        .ok()
        .and_then(|me| me.get("id").and_then(|v| v.as_u64()))
        .map(|n| n.to_string())
}

/// Stream the user's liked tracks page-by-page, mirroring the YT Music
/// `stream_liked_songs` shape so the favorites view can render incrementally.
pub async fn stream_liked_tracks<F: FnMut(Vec<Track>)>(
    token: &str,
    mut on_page: F,
) -> Result<(), String> {
    let http = http_client();
    let cid = client_id(&http, false).await?;
    // `/me/likes/tracks` 404s; the web player reads likes from
    // `/users/{id}/track_likes` (same base the like-write endpoint uses).
    let uid = derive_user_id(token)
        .await
        .ok_or("SoundCloud: couldn't resolve the signed-in user id")?;
    let mut next = Some(format!(
        "{API_V2}/users/{uid}/track_likes?client_id={cid}&limit=200&linked_partitioning=1"
    ));
    let mut pages = 0;
    while let Some(url) = next.take() {
        if pages >= 20 {
            tracing::warn!(
                pages,
                "SoundCloud liked-tracks pagination cap hit; sync is partial"
            );
            break;
        }
        pages += 1;
        let json = auth_get_json(&http, &url, Some(token)).await?;
        let page: Vec<Track> = json
            .get("collection")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    // `/me/likes/tracks` wraps each entry as
                    // `{created_at, kind:"like", track:{…}}`; the actual
                    // track object is nested under "track". Fall back to the
                    // item itself for endpoints that return bare tracks.
                    .map(|item| item.get("track").unwrap_or(item))
                    .filter_map(parse_track)
                    .collect()
            })
            .unwrap_or_default();
        if !page.is_empty() {
            on_page(page);
        }
        next = json
            .get("next_href")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
    }
    Ok(())
}

/// List the signed-in user's own playlists.
pub async fn list_playlists(token: &str) -> Result<Vec<PlaylistSummary>, String> {
    let http = http_client();
    let cid = client_id(&http, false).await?;
    // `/me/playlists` 404s like `/me/likes/tracks`; the web player reads a
    // user's playlists from `/users/{id}/playlists`.
    let uid = derive_user_id(token)
        .await
        .ok_or("SoundCloud: couldn't resolve the signed-in user id")?;
    let mut next = Some(format!(
        "{API_V2}/users/{uid}/playlists?client_id={cid}&limit=50&linked_partitioning=1"
    ));
    let mut out = Vec::new();
    let mut pages = 0;
    while let Some(url) = next.take() {
        if pages >= 10 {
            tracing::warn!(
                pages,
                "SoundCloud playlists pagination cap hit; list is partial"
            );
            break;
        }
        pages += 1;
        let json = auth_get_json(&http, &url, Some(token)).await?;
        if let Some(arr) = json.get("collection").and_then(|v| v.as_array()) {
            for p in arr {
                let Some(id) = p.get("id").and_then(|v| v.as_u64()) else {
                    continue;
                };
                out.push(PlaylistSummary {
                    id: id.to_string(),
                    title: p
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    artwork_url: p
                        .get("artwork_url")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(upscale_artwork),
                });
            }
        }
        next = json
            .get("next_href")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
    }
    Ok(out)
}

/// Load a playlist's full track list. SoundCloud returns most entries as bare
/// `{id}` stubs, so the ids are collected in order and batch-hydrated through
/// `/tracks?ids=` (capped at 50 per request) before parsing.
pub async fn get_playlist_entries(playlist_id: &str, token: &str) -> Result<Vec<Track>, String> {
    let http = http_client();
    let cid = client_id(&http, false).await?;
    let json = auth_get_json(
        &http,
        &format!("{API_V2}/playlists/{playlist_id}?client_id={cid}"),
        Some(token),
    )
    .await?;

    let ids: Vec<u64> = json
        .get("tracks")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("id").and_then(|v| v.as_u64()))
                .collect()
        })
        .unwrap_or_default();

    let mut by_id: std::collections::HashMap<u64, Value> = std::collections::HashMap::new();
    for chunk in ids.chunks(50) {
        let ids_str = chunk
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let url = format!("{API_V2}/tracks?ids={ids_str}&client_id={cid}");
        match auth_get_json(&http, &url, Some(token)).await {
            Ok(arr) => {
                if let Some(items) = arr.as_array() {
                    for t in items {
                        if let Some(id) = t.get("id").and_then(|v| v.as_u64()) {
                            by_id.insert(id, t.clone());
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "SoundCloud playlist hydration chunk failed; contents may be incomplete"
                );
            }
        }
    }

    Ok(ids
        .iter()
        .filter_map(|id| by_id.get(id))
        .filter_map(parse_track)
        .collect())
}

/// Like or unlike a track on the signed-in account.
///
/// Routes through the public `api.soundcloud.com` like endpoint, which (unlike
/// the web player's `api-v2` host) isn't behind DataDome bot-protection — so a
/// plain request with the `OAuth` token works without matching a browser TLS
/// fingerprint. `datadome` is kept only for the legacy `api-v2` fallback.
pub async fn set_track_like(
    track_id: &str,
    like: bool,
    token: &str,
    datadome: Option<&str>,
) -> Result<(), String> {
    let http = http_client();

    // Public API: POST/DELETE https://api.soundcloud.com/likes/tracks/{id}
    let url = format!("{API_V1}/likes/tracks/{track_id}");
    let req = if like {
        http.post(&url)
    } else {
        http.delete(&url)
    }
    .header("Accept", "application/json; charset=utf-8");
    let resp = apply_auth(req, Some(token))
        .send()
        .await
        .map_err(|e| format!("SoundCloud like HTTP: {e}"))?;
    if resp.status().is_success() {
        return Ok(());
    }
    let pub_status = resp.status();
    let pub_body: String = resp
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    tracing::warn!(%pub_status, pub_body = %pub_body, "soundcloud public-api like failed; trying api-v2");

    // Fallback: the web player's api-v2 endpoint (DataDome-gated; needs the
    // browser session's datadome cookie + a real Chrome fingerprint, so this
    // usually only succeeds from the browser itself).
    let cid = client_id(&http, false).await?;
    let uid = derive_user_id(token)
        .await
        .ok_or("SoundCloud: couldn't resolve the signed-in user id")?;
    let url = format!("{API_V2}/users/{uid}/track_likes/{track_id}?client_id={cid}");
    let mut req = if like {
        http.put(url)
    } else {
        http.delete(url)
    }
    .header("Origin", WEB_HOST)
    .header("Referer", format!("{WEB_HOST}/"));
    if let Some(dd) = datadome.filter(|s| !s.is_empty()) {
        req = req.header("Cookie", format!("datadome={dd}"));
    }
    let resp = apply_auth(req, Some(token))
        .send()
        .await
        .map_err(|e| format!("SoundCloud like HTTP: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        // DataDome answers blocked writes with a captcha-challenge redirect
        // (HTTP 403 + a geo.captcha-delivery.com URL). Surface a readable
        // reason instead of dumping the raw challenge payload.
        if status.as_u16() == 403 || body.contains("captcha-delivery.com") {
            return Err(
                "SoundCloud blocks likes outside its web player (DataDome bot \
                 protection). Like this track in the SoundCloud app or website."
                    .to_string(),
            );
        }
        let body: String = body.chars().take(300).collect();
        return Err(format!("SoundCloud like HTTP {status}: {body}"));
    }
    Ok(())
}

/// One-time SoundCloud sign-in via an isolated browser profile. Reuses the YT
/// Music browser-launch machinery; the only differences are the sign-in URL,
/// the cookie domain, and that we extract the single `oauth_token` cookie that
/// the web app sends as `Authorization: OAuth <token>`.
pub mod signin {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use config::Browser;

    use crate::ytmusic::isolated_profile as ip;

    const SIGNIN_URL: &str = "https://soundcloud.com/signin";

    pub fn profile_dir(server_id: &str) -> PathBuf {
        let safe: String = server_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .collect();
        let leaf = if safe.is_empty() {
            "sc-profile".to_string()
        } else {
            format!("sc-profile-{safe}")
        };
        directories::ProjectDirs::from("com", "temidaradev", "kopuz")
            .map(|d| {
                #[cfg(target_os = "windows")]
                let base = d.data_local_dir();
                #[cfg(not(target_os = "windows"))]
                let base = d.config_dir();
                base.join(&leaf)
            })
            .unwrap_or_else(|| PathBuf::from(format!("./{leaf}")))
    }

    pub fn delete_profile(server_id: &str) -> std::io::Result<()> {
        match std::fs::remove_dir_all(profile_dir(server_id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Launch the chosen browser at the SoundCloud sign-in page and poll the
    /// isolated profile's cookie store until `oauth_token` appears. The browser
    /// is always killed before returning.
    #[tracing::instrument(name = "sc.signin", skip(server_id, signin_timeout), fields(browser = %browser))]
    pub async fn launch_signin_and_extract(
        browser: Browser,
        server_id: &str,
        signin_timeout: Duration,
    ) -> Result<String, String> {
        let profile = profile_dir(server_id);
        match tokio::fs::remove_dir_all(&profile).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("wipe sc-profile: {e}")),
        }
        tokio::fs::create_dir_all(&profile)
            .await
            .map_err(|e| format!("mkdir sc-profile: {e}"))?;

        let bin = if ip::in_flatpak() {
            ip::find_host_browser_bin(browser).await.ok_or_else(|| {
                format!(
                    "{browser} not found on the host (looked for: {}). Install it on the host system.",
                    ip::browser_candidates(browser).join(", ")
                )
            })?
        } else {
            ip::find_browser_bin(browser).ok_or_else(|| {
                format!(
                    "{browser} not found in PATH (looked for: {}). Install it, or set $KOPUZ_{}_BIN.",
                    ip::browser_candidates(browser).join(", "),
                    browser.id().to_uppercase().replace('-', "_")
                )
            })?
        };

        let mut cmd = ip::browser_command(&bin);
        cmd.arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(format!("--user-data-dir={}", profile.display()));
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0100_0000);
        }
        let mut child = cmd
            .arg(SIGNIN_URL)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("spawn {bin}: {e}"))?;

        let deadline = Instant::now() + signin_timeout;
        let outcome = loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if Instant::now() > deadline {
                break Err(format!(
                    "Sign-in not detected within {}s",
                    signin_timeout.as_secs()
                ));
            }
            let _ = child.try_wait();
            if let Ok(Some(token)) = extract_oauth_token(browser, &profile).await {
                break Ok(token);
            }
        };
        let _ = child.kill().await;
        outcome
    }

    /// Pull the `oauth_token` cookie value out of the isolated profile's cookie
    /// store (decrypted by `rookie`, exactly like the YT cookie reader).
    pub async fn extract_oauth_token(
        browser: Browser,
        profile_root: &Path,
    ) -> Result<Option<String>, String> {
        extract_cookie(browser, profile_root, "oauth_token").await
    }

    /// Pull the `datadome` cookie set by SoundCloud's DataDome bot-protection
    /// during the human sign-in. api-v2 *write* endpoints (like/repost) 403 with
    /// a captcha challenge unless the request carries this session cookie; reads
    /// aren't gated, so this is only needed on mutations.
    pub async fn extract_datadome(
        browser: Browser,
        profile_root: &Path,
    ) -> Result<Option<String>, String> {
        extract_cookie(browser, profile_root, "datadome").await
    }

    /// Decrypt the isolated profile's cookie store (via `rookie`) and return the
    /// value of the named cookie, if present and non-empty.
    pub async fn extract_cookie(
        browser: Browser,
        profile_root: &Path,
        name: &str,
    ) -> Result<Option<String>, String> {
        let db_path =
            pick_cookies_path(profile_root).ok_or_else(|| "no Cookies database yet".to_string())?;
        let profile_owned = profile_root.to_path_buf();
        let browser_name = rookie_browser_name(browser);

        let cookies =
            tokio::task::spawn_blocking(move || -> Result<Vec<rookie::enums::Cookie>, String> {
                let domains = Some(vec!["soundcloud.com".to_string()]);
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = profile_owned;
                    let config = rookie::config::get_browser_config(browser_name);
                    rookie::chromium_based(config, db_path, domains).map_err(|e| e.to_string())
                }
                #[cfg(target_os = "windows")]
                {
                    let _ = browser_name;
                    let key_path = profile_owned.join("Local State");
                    rookie::chromium_based(key_path, db_path, domains).map_err(|e| e.to_string())
                }
            })
            .await
            .map_err(|e| format!("cookie extract task: {e}"))??;

        Ok(cookies
            .into_iter()
            .find(|c| c.name == name && !c.value.is_empty())
            .map(|c| c.value))
    }

    fn rookie_browser_name(browser: Browser) -> &'static str {
        match browser {
            Browser::Brave => "brave",
            Browser::Chrome => "chrome",
            Browser::Chromium => "chromium",
            Browser::Edge => "edge",
            Browser::Vivaldi => "vivaldi",
        }
    }

    fn pick_cookies_path(profile_root: &Path) -> Option<PathBuf> {
        [
            profile_root.join("Default").join("Network").join("Cookies"),
            profile_root.join("Default").join("Cookies"),
        ]
        .into_iter()
        .find(|p| p.exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_client_id_from_bundle_forms() {
        assert_eq!(
            find_client_id(r#"foo,client_id:"abcdefghij0123456789",bar"#).as_deref(),
            Some("abcdefghij0123456789")
        );
        assert_eq!(
            find_client_id(r#"{"client_id":"ABCDEFGHIJ0123456789"}"#).as_deref(),
            Some("ABCDEFGHIJ0123456789")
        );
        assert_eq!(find_client_id(r#"client_id:"short""#), None);
        assert_eq!(find_client_id("no id here"), None);
    }

    #[test]
    fn extract_attr_reads_src() {
        let chunk = r#" crossorigin src="https://a-v2.sndcdn.com/assets/0-abc.js"></script>"#;
        assert_eq!(
            extract_attr(chunk, "src"),
            Some("https://a-v2.sndcdn.com/assets/0-abc.js")
        );
        assert_eq!(extract_attr("<div>", "src"), None);
    }

    #[test]
    fn upscale_artwork_swaps_size_token() {
        assert_eq!(
            upscale_artwork("https://i1.sndcdn.com/artworks-xyz-large.jpg"),
            "https://i1.sndcdn.com/artworks-xyz-t500x500.jpg"
        );
        assert_eq!(upscale_artwork("https://x/y.png"), "https://x/y.png");
    }

    #[test]
    fn parse_track_requires_progressive_transcoding() {
        let hls_only = serde_json::json!({
            "id": 1, "title": "t", "kind": "track",
            "user": {"username": "u"},
            "media": {"transcodings": [{"url": "x", "format": {"protocol": "hls"}}]}
        });
        assert!(parse_track(&hls_only).is_none());

        let ok = serde_json::json!({
            "id": 42, "title": "Song", "kind": "track",
            "duration": 215000,
            "artwork_url": "https://i1.sndcdn.com/artworks-z-large.jpg",
            "user": {"username": "Artist"},
            "media": {"transcodings": [{"url": "x", "format": {"protocol": "progressive"}}]}
        });
        let t = parse_track(&ok).expect("progressive track parses");
        assert_eq!(t.title, "Song");
        assert_eq!(t.artist, "Artist");
        assert_eq!(t.duration, 215);
        assert_eq!(track_id(&t), "42");
        assert!(
            t.path
                .to_string_lossy()
                .starts_with("soundcloud:42:urlhex_")
        );
    }
}
