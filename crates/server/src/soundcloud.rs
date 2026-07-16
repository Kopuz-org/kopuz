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

use reader::models::Track;
use serde_json::Value;
use tokio::sync::Mutex;

/// SoundCloud's internal web-player API. Keyless apart from the scraped
/// `client_id` query param.
const API_V2: &str = "https://api-v2.soundcloud.com";

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

/// Search the SoundCloud catalog for tracks matching `query`.
#[tracing::instrument(name = "soundcloud.search", fields(query = %query))]
pub(crate) async fn search_tracks(query: &str) -> Result<Vec<Track>, String> {
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
pub(crate) enum ResolvedStream {
    Progressive(String),
    HlsAac(String),
}

/// Resolve a track id to a playable stream via the `/tracks` lookup endpoint,
/// called lazily at play time. When `token` is an authenticated Go+ session the
/// track's AAC HLS transcoding is preferred for higher quality; otherwise it
/// falls back to the universal progressive MP3.
#[tracing::instrument(name = "soundcloud.resolve_stream", skip(token), fields(track_id = %track_id))]
pub(crate) async fn resolve_stream(
    track_id: &str,
    token: Option<&str>,
) -> Result<ResolvedStream, String> {
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

/// A track's public soundcloud.com page URL (`permalink_url`), for sharing.
pub(crate) async fn track_permalink(
    track_id: &str,
    token: Option<&str>,
) -> Result<Option<String>, String> {
    let http = http_client();
    let track = match lookup_track(&http, track_id, &client_id(&http, false).await?, token).await {
        Ok(v) => v,
        Err(_) => lookup_track(&http, track_id, &client_id(&http, true).await?, token).await?,
    };
    Ok(track
        .get("permalink_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string()))
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

/// The SoundCloud track id (the typed identity's key).
fn track_id(t: &Track) -> String {
    t.id.key().into_owned()
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

    Some(Track {
        // Typed identity (replaces the old `soundcloud:<id>:urlhex_…` path hack);
        // the artwork URL lives in `cover`, resolved straight through by the
        // SoundCloud arm of the cover seam.
        id: reader::models::TrackId::Server {
            service: config::MusicService::SoundCloud,
            item_id: track_id.to_string(),
        },
        cover: artwork,
        album_id: String::new(),
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
pub(crate) struct PlaylistSummary {
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
pub(crate) async fn get_me(token: &str) -> Result<Value, String> {
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

/// One page of the user's liked tracks: pass `cursor = None` for the first page
/// (built from the resolved user id), then the returned `next_href` for each
/// subsequent page (`None` once exhausted). Cursor-based + `Send` so a
/// `MediaSource::fetch_favorites_page` impl can pull it; mirrors YT's
/// `liked_songs_page`. The `/me/likes/tracks` route 404s, so the web player's
/// `/users/{id}/track_likes` is used.
pub(crate) async fn liked_tracks_page(
    token: &str,
    cursor: Option<&str>,
) -> Result<(Vec<Track>, Option<String>), String> {
    let http = http_client();
    let url = match cursor {
        Some(c) => c.to_string(),
        None => {
            let cid = client_id(&http, false).await?;
            let uid = derive_user_id(token)
                .await
                .ok_or("SoundCloud: couldn't resolve the signed-in user id")?;
            format!(
                "{API_V2}/users/{uid}/track_likes?client_id={cid}&limit=200&linked_partitioning=1"
            )
        }
    };
    let json = auth_get_json(&http, &url, Some(token)).await?;
    let page: Vec<Track> = json
        .get("collection")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                // Each entry is `{created_at, kind:"like", track:{…}}`; the track
                // object is nested under "track". Fall back to the bare item.
                .map(|item| item.get("track").unwrap_or(item))
                .filter_map(parse_track)
                .collect()
        })
        .unwrap_or_default();
    let next = json
        .get("next_href")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Ok((page, next))
}

/// List the signed-in user's own playlists.
pub(crate) async fn list_playlists(token: &str) -> Result<Vec<PlaylistSummary>, String> {
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
pub(crate) async fn get_playlist_entries(
    playlist_id: &str,
    token: &str,
) -> Result<Vec<Track>, String> {
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
/// The route the web player actually uses (captured from live traffic):
/// `PUT`/`DELETE api-v2.soundcloud.com/users/{user_id}/track_likes/{track_id}`.
/// The public `api.soundcloud.com/likes/tracks/{id}` route this used to call
/// now rejects every request with 403. `Origin`/`Referer` match the web app —
/// api-v2 writes are picky about looking like the player.
pub(crate) async fn set_track_like(track_id: &str, like: bool, token: &str) -> Result<(), String> {
    let http = http_client();
    let uid = derive_user_id(token)
        .await
        .ok_or("SoundCloud: couldn't resolve the signed-in user id")?;

    let send = |cid: String| {
        let url = format!("{API_V2}/users/{uid}/track_likes/{track_id}?client_id={cid}");
        let req = if like {
            http.put(&url)
        } else {
            http.delete(&url)
        }
        .header("Accept", "application/json, text/javascript, */*; q=0.1")
        .header("Origin", WEB_HOST)
        .header("Referer", format!("{WEB_HOST}/"));
        apply_auth(req, Some(token)).send()
    };

    match send(client_id(&http, false).await?).await {
        Ok(r) if r.status().is_success() => return Ok(()),
        // A stale scraped client_id also 4xxes — re-scrape once and retry,
        // like every other endpoint here.
        _ => {}
    }
    send(client_id(&http, true).await?)
        .await
        .map_err(|e| format!("SoundCloud like HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("SoundCloud like HTTP: {e}"))?;
    Ok(())
}

/// One-time SoundCloud sign-in via an isolated browser profile, reusing the
/// shared [`crate::cookies`] machinery. We extract the single `oauth_token`
/// cookie the web app sends as `Authorization: OAuth <token>`.
pub mod signin {
    use std::time::Duration;

    use config::Browser;

    use crate::cookies;

    const PROFILE_PREFIX: &str = "sc-profile";

    pub fn delete_profile(server_id: &str) -> std::io::Result<()> {
        cookies::delete_profile(PROFILE_PREFIX, server_id)
    }

    #[tracing::instrument(name = "sc.signin", skip(server_id, signin_timeout), fields(browser = %browser))]
    pub async fn launch_signin_and_extract(
        browser: Browser,
        server_id: &str,
        signin_timeout: Duration,
    ) -> Result<String, String> {
        cookies::launch_signin_and_extract(
            browser,
            server_id,
            PROFILE_PREFIX,
            "https://soundcloud.com/signin",
            signin_timeout,
            |browser, profile| async move {
                Ok(cookies::read_cookies(browser, &profile, "soundcloud.com")
                    .await
                    .ok()
                    .and_then(|cs| {
                        cs.into_iter()
                            .find(|c| c.name == "oauth_token" && !c.value.is_empty())
                            .map(|c| c.value)
                    }))
            },
        )
        .await
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
        assert_eq!(t.id.service(), Some(config::MusicService::SoundCloud));
        // The artwork URL lives in `cover` now (not a path-encoded tag).
        assert!(t.cover.as_deref().is_some_and(|c| c.contains("sndcdn.com")));
    }
}
