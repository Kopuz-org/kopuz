//! SoundCloud, as a kopuz plugin, through the public `api-v2` web-player API.
//!
//! SoundCloud streams are not DRM'd: every publicly streamable track exposes a
//! progressive MP3 or an AAC HLS transcoding the player reads like any other
//! stream, so this is full-track playback, not previews.
//!
//! There is no registerable API key, so, like every SoundCloud client, the
//! plugin lifts the web player's `client_id` out of its JS bundles and keeps
//! it. The id rotates now and then; a failed call forces one re-scrape.
//!
//! Signing in happens in a browser the host drives; the session only ever
//! sees the resulting OAuth token.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use kopuz_plugin_sdk::http::{Client, RequestBuilder};
use kopuz_plugin_sdk::{
    ArtistCredit, AuthOutcome, Connection, PlaylistMeta, Result, SearchResults, Source,
    SourceError, StreamInfo, StreamQuality, Track, TrackPage,
};
use serde_json::Value;

/// SoundCloud's internal web-player API. Keyless apart from the scraped
/// `client_id` query param.
const API_V2: &str = "https://api-v2.soundcloud.com";

/// The public developer API host. Unlike `api-v2` it isn't behind DataDome, so
/// authenticated writes (like/unlike) go through here.
const API_V1: &str = "https://api.soundcloud.com";

/// The web player whose HTML/JS bundles carry the `client_id`.
const WEB_HOST: &str = "https://soundcloud.com";

/// Chrome-on-macOS UA. SoundCloud's api-v2 write endpoints (like/repost) 403 any
/// request that doesn't look like the web player, so every request carries it.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub struct SoundCloud {
    http: Client,
    /// OAuth token for the signed-in account; `None` = anonymous (search and
    /// play of public tracks still work with the scraped client id).
    token: Option<String>,
    client_id: RefCell<Option<String>>,
    user_id: RefCell<Option<String>>,
}

kopuz_plugin_sdk::export_source!(SoundCloud);

fn backend(message: impl Into<String>) -> SourceError {
    SourceError::Backend(message.into())
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
            let id: String = js[pos + marker.len()..]
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
/// display-friendly 500px. The CDN honours the `-t500x500` size token.
fn upscale_artwork(url: &str) -> String {
    url.replace("-large.", "-t500x500.")
}

fn transcoding_protocol(tc: &Value) -> Option<&str> {
    tc.get("format")?.get("protocol")?.as_str()
}

fn transcoding_mime(tc: &Value) -> Option<&str> {
    tc.get("format")?.get("mime_type")?.as_str()
}

/// The AAC HLS stream; its mime type carries codec parameters (`audio/mp4; codecs="mp4a.40.2"`).
fn is_hls_aac(tc: &Value) -> bool {
    transcoding_protocol(tc) == Some("hls")
        && transcoding_mime(tc).is_some_and(|mime| mime.starts_with("audio/mp4"))
}

/// Whether a stream can be resolved from this transcoding; most tracks now offer only HLS.
fn is_playable(tc: &Value) -> bool {
    transcoding_protocol(tc) == Some("progressive") || is_hls_aac(tc)
}

fn parse_track(item: &Value) -> Option<Track> {
    let track_id = item.get("id").and_then(Value::as_u64)?;
    let playable = item
        .get("media")
        .and_then(|m| m.get("transcodings"))
        .and_then(Value::as_array)
        .is_some_and(|arr| arr.iter().any(is_playable));
    if !playable {
        return None;
    }

    let title = item
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let user = item.get("user");
    let artist = user
        .and_then(|u| u.get("username"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // SoundCloud has no artist entity; the uploader is it.
    let artist_id = user
        .and_then(|u| u.get("id"))
        .and_then(Value::as_u64)
        .map(|id| id.to_string());
    let artwork = item
        .get("artwork_url")
        .and_then(Value::as_str)
        .or_else(|| user?.get("avatar_url")?.as_str())
        .filter(|s| !s.is_empty())
        .map(upscale_artwork);
    let duration = item
        .get("full_duration")
        .and_then(Value::as_u64)
        .or_else(|| item.get("duration").and_then(Value::as_u64))
        .map(|ms| ms / 1000)
        .unwrap_or(0);

    Some(Track {
        item_id: track_id.to_string(),
        cover: artwork,
        title,
        duration_secs: duration,
        credits: match (artist.is_empty(), artist_id) {
            (true, _) => Vec::new(),
            (false, Some(id)) => vec![ArtistCredit::linked(&artist, id)],
            (false, None) => vec![ArtistCredit::unlinked(&artist)],
        },
        artists: if artist.is_empty() {
            Vec::new()
        } else {
            vec![artist.clone()]
        },
        artist,
        ..Track::default()
    })
}

fn next_href(json: &Value) -> Option<String> {
    json.get("next_href")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl SoundCloud {
    fn get(&self, url: &str) -> RequestBuilder {
        let request = self.http.get(url);
        match self.token.as_deref() {
            Some(token) => request.header("Authorization", format!("OAuth {token}")),
            None => request,
        }
    }

    fn json(&self, request: RequestBuilder, what: &str) -> Result<Value> {
        let response = request.send()?;
        if !response.is_success() {
            return Err(SourceError::from_status(response.status(), what));
        }
        response
            .json()
            .map_err(|error| backend(format!("{what}: {error}")))
    }

    fn token(&self) -> Result<&str> {
        self.token.as_deref().ok_or(SourceError::Auth)
    }

    /// The web player's `client_id`, scraped on first use or when `force` is
    /// set after a call failed with the old one.
    fn client_id(&self, force: bool) -> Result<String> {
        if !force && let Some(id) = self.client_id.borrow().as_ref() {
            return Ok(id.clone());
        }
        let id = self.scrape_client_id()?;
        *self.client_id.borrow_mut() = Some(id.clone());
        Ok(id)
    }

    /// The homepage references a handful of `*.sndcdn.com/assets/*.js` chunks;
    /// the id lives in one of them, usually a later one, so they are scanned
    /// newest-first.
    fn scrape_client_id(&self) -> Result<String> {
        let response = self.http.get(WEB_HOST).send()?;
        if !response.is_success() {
            return Err(SourceError::from_status(
                response.status(),
                "SoundCloud homepage",
            ));
        }
        let html = response.text();
        let scripts: Vec<&str> = html
            .split("<script")
            .filter_map(|chunk| extract_attr(chunk, "src"))
            .filter(|src| src.contains("sndcdn.com/assets/") && src.ends_with(".js"))
            .collect();
        for src in scripts.iter().rev() {
            if let Ok(response) = self.http.get(src).send()
                && let Some(id) = find_client_id(&response.text())
            {
                return Ok(id);
            }
        }
        Err(backend(
            "SoundCloud: couldn't extract a client_id from the web player",
        ))
    }

    /// Run `call` with the cached client id, and once more with a fresh one if
    /// it fails: the id rotating is the usual reason.
    fn with_client_id<T>(&self, call: impl Fn(&str) -> Result<T>) -> Result<T> {
        match call(&self.client_id(false)?) {
            Ok(value) => Ok(value),
            Err(SourceError::Connectivity) => Err(SourceError::Connectivity),
            Err(_) => call(&self.client_id(true)?),
        }
    }

    fn me(&self) -> Result<Value> {
        self.token()?;
        self.with_client_id(|cid| {
            self.json(
                self.get(&format!("{API_V2}/me"))
                    .query(&[("client_id", cid)]),
                "SoundCloud profile",
            )
        })
    }

    fn signed_in_user(&self) -> Result<String> {
        if let Some(id) = self.user_id.borrow().as_ref() {
            return Ok(id.clone());
        }
        let id = self
            .me()?
            .get("id")
            .and_then(Value::as_u64)
            .map(|id| id.to_string())
            .ok_or_else(|| backend("SoundCloud: couldn't resolve the signed-in user id"))?;
        *self.user_id.borrow_mut() = Some(id.clone());
        Ok(id)
    }

    /// Exchange a transcoding URL for its time-limited CDN media URL (a direct
    /// MP3 for progressive, or an `.m3u8` playlist for HLS).
    fn resolve_media_url(&self, transcoding_url: &str, track_auth: Option<&str>) -> Result<String> {
        let cid = self.client_id(false)?;
        let mut request = self
            .get(transcoding_url)
            .query(&[("client_id", cid.as_str())]);
        if let Some(auth) = track_auth {
            request = request.query(&[("track_authorization", auth)]);
        }
        self.json(request, "SoundCloud stream resolve")?
            .get("url")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| backend("SoundCloud returned no stream URL for this track"))
    }
}

impl Source for SoundCloud {
    fn open(connection: Connection) -> Self {
        Self {
            http: Client::new()
                .with_timeout(Duration::from_secs(15))
                .with_header("User-Agent", USER_AGENT),
            token: (!connection.token.is_empty()).then_some(connection.token),
            client_id: RefCell::new(None),
            user_id: RefCell::new((!connection.user_id.is_empty()).then_some(connection.user_id)),
        }
    }

    fn validate(&self) -> AuthOutcome {
        if self.token.is_none() {
            // Anonymous mode is always usable (public search and play).
            return AuthOutcome::Valid;
        }
        match self.me() {
            Ok(_) => AuthOutcome::Valid,
            // An expired token and a network blip look alike here, so don't
            // force a re-sign-in: any failure reads as unreachable.
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    fn user_id(&self) -> Result<Option<String>> {
        if self.token.is_none() {
            return Ok(None);
        }
        self.signed_in_user().map(Some)
    }

    fn search(&self, query: &str) -> Result<SearchResults> {
        if query.trim().is_empty() {
            return Ok(SearchResults {
                tracks: Vec::new(),
                albums: Vec::new(),
            });
        }
        let response = self.with_client_id(|cid| {
            self.json(
                self.http.get(format!("{API_V2}/search/tracks")).query(&[
                    ("q", query),
                    ("client_id", cid),
                    ("limit", "50"),
                ]),
                "SoundCloud search",
            )
        })?;
        let mut seen = HashSet::new();
        let tracks = response
            .get("collection")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("kind").and_then(Value::as_str) == Some("track"))
            .filter_map(parse_track)
            .filter(|track| seen.insert(track.item_id.clone()))
            .collect();
        Ok(SearchResults {
            tracks,
            albums: Vec::new(),
        })
    }

    /// Prefers the AAC HLS stream a Go+ session exposes, and falls back to the
    /// progressive MP3 every track has when that cannot be resolved.
    fn resolve_stream(&self, item_id: &str, _: StreamQuality) -> Result<StreamInfo> {
        let track = self.with_client_id(|cid| {
            self.json(
                self.get(&format!("{API_V2}/tracks/{item_id}"))
                    .query(&[("client_id", cid)]),
                "SoundCloud lookup",
            )
        })?;
        let transcodings = track
            .get("media")
            .and_then(|m| m.get("transcodings"))
            .and_then(Value::as_array)
            .ok_or_else(|| backend("SoundCloud track exposes no media transcodings"))?;
        let track_auth = track.get("track_authorization").and_then(Value::as_str);

        if let Some(url) = transcodings
            .iter()
            .find(|tc| is_hls_aac(tc))
            .and_then(|tc| tc.get("url")?.as_str())
        {
            match self.resolve_media_url(url, track_auth) {
                Ok(media) => {
                    return Ok(StreamInfo {
                        hls: true,
                        ..StreamInfo::url(media)
                    });
                }
                Err(error) => kopuz_plugin_sdk::warn!(
                    "SoundCloud HLS resolve failed; falling back to progressive: {error}"
                ),
            }
        }

        let progressive = transcodings
            .iter()
            .find(|tc| transcoding_protocol(tc) == Some("progressive"))
            .and_then(|tc| tc.get("url")?.as_str())
            .ok_or_else(|| backend("SoundCloud track has no progressive (non-HLS) stream"))?;
        Ok(StreamInfo::url(
            self.resolve_media_url(progressive, track_auth)?,
        ))
    }

    fn fetch_favorites(&self) -> Result<Vec<String>> {
        if self.token.is_none() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = self.fetch_favorites_page(cursor.as_deref())?;
            ids.extend(page.tracks.into_iter().map(|track| track.item_id));
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(ids),
            }
        }
    }

    /// The `/me/likes/tracks` route 404s, so the web player's
    /// `/users/{id}/track_likes` is walked instead.
    fn fetch_favorites_page(&self, cursor: Option<&str>) -> Result<TrackPage> {
        if self.token.is_none() {
            return Ok(TrackPage::last(Vec::new()));
        }
        let url = match cursor {
            Some(cursor) => cursor.to_string(),
            None => format!(
                "{API_V2}/users/{}/track_likes?client_id={}&limit=200&linked_partitioning=1",
                self.signed_in_user()?,
                self.client_id(false)?
            ),
        };
        let json = self.json(self.get(&url), "SoundCloud likes")?;
        let tracks = json
            .get("collection")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            // Each entry is `{created_at, kind:"like", track:{…}}`.
            .map(|item| item.get("track").unwrap_or(item))
            .filter_map(parse_track)
            .collect();
        Ok(TrackPage {
            tracks,
            next: next_href(&json),
        })
    }

    /// Through the public `api.soundcloud.com` endpoint, which unlike `api-v2`
    /// is not behind DataDome, so the OAuth token alone authorizes the write.
    fn push_favorite(&self, item_id: &str, on: bool) -> Result<()> {
        let token = self.token()?;
        let url = format!("{API_V1}/likes/tracks/{item_id}");
        let request = if on {
            self.http.post(url)
        } else {
            self.http.delete(url)
        };
        let response = request
            .header("Accept", "application/json; charset=utf-8")
            .header("Authorization", format!("OAuth {token}"))
            .send()?;
        if response.is_success() {
            Ok(())
        } else {
            Err(SourceError::from_status(
                response.status(),
                "SoundCloud like",
            ))
        }
    }

    /// `/me/playlists` 404s like `/me/likes/tracks`; the web player reads a
    /// user's playlists from `/users/{id}/playlists`.
    fn fetch_playlists(&self) -> Result<Vec<PlaylistMeta>> {
        if self.token.is_none() {
            return Ok(Vec::new());
        }
        let mut next = Some(format!(
            "{API_V2}/users/{}/playlists?client_id={}&limit=50&linked_partitioning=1",
            self.signed_in_user()?,
            self.client_id(false)?
        ));
        let mut out = Vec::new();
        let mut pages = 0;
        while let Some(url) = next.take() {
            if pages >= 10 {
                kopuz_plugin_sdk::warn!("SoundCloud playlists pagination cap hit; list is partial");
                break;
            }
            pages += 1;
            let json = self.json(self.get(&url), "SoundCloud playlists")?;
            for playlist in json
                .get("collection")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = playlist.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                out.push(PlaylistMeta {
                    id: id.to_string(),
                    name: playlist
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    image_tag: playlist
                        .get("artwork_url")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(upscale_artwork),
                });
            }
            next = next_href(&json);
        }
        Ok(out)
    }

    /// SoundCloud returns most entries as bare `{id}` stubs, so the ids are
    /// collected in order and hydrated through `/tracks?ids=`, 50 at a time.
    fn fetch_playlist_entries(&self, playlist_id: &str) -> Result<Vec<Track>> {
        if self.token.is_none() {
            return Ok(Vec::new());
        }
        let cid = self.client_id(false)?;
        let json = self.json(
            self.get(&format!("{API_V2}/playlists/{playlist_id}"))
                .query(&[("client_id", cid.as_str())]),
            "SoundCloud playlist",
        )?;
        let ids: Vec<u64> = json
            .get("tracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|track| track.get("id").and_then(Value::as_u64))
            .collect();

        let mut by_id: HashMap<u64, Value> = HashMap::new();
        for chunk in ids.chunks(50) {
            let joined = chunk
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let request = self
                .get(&format!("{API_V2}/tracks"))
                .query(&[("ids", joined.as_str()), ("client_id", cid.as_str())]);
            match self.json(request, "SoundCloud tracks") {
                Ok(items) => {
                    for track in items.as_array().into_iter().flatten() {
                        if let Some(id) = track.get("id").and_then(Value::as_u64) {
                            by_id.insert(id, track.clone());
                        }
                    }
                }
                Err(error) => kopuz_plugin_sdk::warn!(
                    "SoundCloud playlist hydration chunk failed; contents may be incomplete: {error}"
                ),
            }
        }
        Ok(ids
            .iter()
            .filter_map(|id| by_id.get(id))
            .filter_map(parse_track)
            .collect())
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
    fn parse_track_requires_a_playable_transcoding() {
        let mpegurl_only = serde_json::json!({
            "id": 1, "title": "t", "kind": "track",
            "user": {"username": "u"},
            "media": {"transcodings": [{"url": "x", "format": {"protocol": "hls", "mime_type": "audio/mpegurl"}}]}
        });
        assert!(parse_track(&mpegurl_only).is_none());

        // What api-v2 search answers for "snowgrave": AAC HLS only, no progressive.
        let aac_hls = serde_json::json!({
            "id": 7, "title": "SNOWGRAVE", "kind": "track",
            "user": {"username": "u"},
            "media": {"transcodings": [
                {"url": "x", "format": {"protocol": "hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\""}},
                {"url": "y", "format": {"protocol": "hls", "mime_type": "audio/mpegurl"}}
            ]}
        });
        assert_eq!(
            parse_track(&aac_hls).map(|track| track.title),
            Some("SNOWGRAVE".to_string())
        );

        let ok = serde_json::json!({
            "id": 42, "title": "Song", "kind": "track",
            "duration": 215000,
            "artwork_url": "https://i1.sndcdn.com/artworks-z-large.jpg",
            "user": {"username": "Artist", "id": 9},
            "media": {"transcodings": [{"url": "x", "format": {"protocol": "progressive"}}]}
        });
        let track = parse_track(&ok).expect("progressive track parses");
        assert_eq!(track.title, "Song");
        assert_eq!(track.artist, "Artist");
        assert_eq!(track.duration_secs, 215);
        assert_eq!(track.item_id, "42");
        assert_eq!(track.credits, [ArtistCredit::linked("Artist", "9")]);
        assert!(
            track
                .cover
                .as_deref()
                .is_some_and(|c| c.contains("t500x500"))
        );
    }
}
