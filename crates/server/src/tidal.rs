//! TIDAL integration via the **public** Developer API (`openapi.tidal.com/v2`,
//! [JSON:API]) + the OAuth2 Authorization-Code-with-PKCE flow.
//!
//! Sign-in uses the **user's own registered app** (developer.tidal.com) — its
//! client id is entered when adding the server; nothing is baked into the repo.
//! kopuz opens the TIDAL authorize page in the **user's default browser** (see
//! [`signin`]), the user logs in, and TIDAL redirects to a loopback callback
//! ([`REDIRECT_URI`]) that a short-lived local listener captures the
//! authorization code from. The code + PKCE verifier are exchanged for tokens.
//! No SPA, no cookie/localStorage scraping.
//!
//! For the authorize page to load instead of erroring (a 400 renders as TIDAL
//! error 11102, "Something went wrong"), the developer.tidal.com app **must**
//! have (1) [`REDIRECT_URI`] registered verbatim and (2) every scope in
//! [`SCOPE`] enabled. `login.tidal.com` is also fronted by DataDome, which can
//! reject privacy-hardened/extension-laden browsers as `UNSUPPORTED_OS` (same
//! 11102 error page).
//!
//! Access tokens are refreshed with the stored refresh token against the same
//! client. The token set + the client id/secret are packed into the single
//! `access_token` config column (see [`Creds`]); `user_id` holds TIDAL's numeric
//! userId, resolved from `/users/me` right after the grant (the public token
//! response doesn't carry it).
//!
//! Catalog/collection/search all go through the v2 JSON:API. **Playback is not
//! wired**: the v2 `trackManifests` endpoint returns Widevine-encrypted MPEG-DASH
//! for every format, and full-length streams additionally require an access tier
//! TIDAL grants selectively — see [`resolve_stream`]. Metadata browsing works;
//! audio bytes do not.
//!
//! Track encoding mirrors SoundCloud: `TrackId::Server { Tidal, <trackId> }`,
//! the artwork URL directly in `Track::cover`, and the album grid uses the
//! subsonic-style `tidal:<albumId>:urlhex_<hex>` ref the shared cover resolver
//! already understands.
//!
//! [JSON:API]: https://jsonapi.org/

use std::collections::HashMap;

use reader::models::Track;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Public Developer API base (JSON:API).
const API_V2: &str = "https://openapi.tidal.com/v2";
/// OAuth authorize page (browser) + token endpoint.
const AUTHORIZE_URL: &str = "https://login.tidal.com/authorize";
const TOKEN_URL: &str = "https://auth.tidal.com/v1/oauth2/token";
/// Loopback redirect the local listener binds; the user must register this
/// exact URI on their developer.tidal.com app.
const REDIRECT_URI: &str = "http://localhost:8765/callback";
const REDIRECT_PORT: u16 = 8765;
/// JSON:API content type — v2 rejects requests without it.
const JSON_API: &str = "application/vnd.api+json";

/// OAuth scopes requested at sign-in. **Every one of these must be enabled on
/// the developer.tidal.com app**, or the authorize page errors before login.
/// `collection.write` powers favoriting; `playback`/`entitlements.read` are for
/// the (currently DRM-blocked) stream endpoint.
const SCOPE: &str = "user.read collection.read collection.write playlists.read search.read entitlements.read playback";

/// A browser-shaped UA — the token/API hosts sit behind a WAF that drops
/// UA-less requests.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_default()
}

/// Everything a signed-in TIDAL server needs, packed into the `access_token`
/// config column: the tokens, the account country, and the user's own client
/// id/secret (so a background refresh works without the add-server form). Secret
/// is empty for a public PKCE client.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Creds {
    pub access: String,
    pub refresh: String,
    /// The account's `country` — v2 catalog endpoints take it as `countryCode`.
    pub country: String,
    pub client_id: String,
    pub client_secret: String,
}

pub fn pack_creds(c: &Creds) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        c.access, c.refresh, c.country, c.client_id, c.client_secret
    )
}

pub fn unpack_creds(packed: &str) -> Option<Creds> {
    let mut lines = packed.lines();
    let creds = Creds {
        access: lines.next()?.to_string(),
        refresh: lines.next()?.to_string(),
        country: lines.next().unwrap_or("US").to_string(),
        client_id: lines.next().unwrap_or_default().to_string(),
        client_secret: lines.next().unwrap_or_default().to_string(),
    };
    (!creds.access.is_empty()).then_some(creds)
}

/// A granted token set. `refresh` is absent on refresh-grant responses — the
/// caller keeps the old one.
#[derive(Debug, Clone)]
pub struct TokenGrant {
    pub access: String,
    pub refresh: Option<String>,
    pub user_id: String,
    pub country: String,
}

/// A PKCE verifier + its S256 challenge for one sign-in attempt.
struct Pkce {
    verifier: String,
    challenge: String,
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn new_pkce() -> Pkce {
    let mut raw = [0u8; 32];
    for b in raw.iter_mut() {
        *b = rand::random::<u8>();
    }
    let verifier = b64url(&raw);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    Pkce {
        verifier,
        challenge,
    }
}

/// The authorize URL kopuz opens in the isolated sign-in browser.
fn authorize_url(client_id: &str, challenge: &str, state: &str) -> String {
    reqwest::Url::parse_with_params(
        AUTHORIZE_URL,
        &[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPE),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge),
            ("state", state),
        ],
    )
    .map(String::from)
    .unwrap_or_default()
}

/// Legacy isolated-profile prefix. The public flow no longer creates one, but a
/// server removed after an older sign-in may still have it on disk — keep the
/// cleanup path so uninstalling a server tidies up.
const PROFILE_PREFIX: &str = "tidal-profile";

/// Remove any leftover isolated sign-in profile for a server (no-op if none).
pub fn delete_profile(server_id: &str) -> std::io::Result<()> {
    crate::cookies::delete_profile(PROFILE_PREFIX, server_id)
}

/// Run the PKCE sign-in in the **user's default browser**: open the authorize
/// page, then wait on the loopback listener for TIDAL's redirect carrying the
/// authorization code.
///
/// If the page shows TIDAL error 11102 ("Something went wrong") instead of the
/// login form, either the developer.tidal.com app is misconfigured (redirect
/// URI/scopes — see [`REDIRECT_URI`] and [`SCOPE`]) or DataDome rejected the
/// browser (`UNSUPPORTED_OS`).
#[tracing::instrument(name = "tidal.signin", skip_all)]
pub async fn signin(
    client_id: &str,
    client_secret: &str,
    timeout: std::time::Duration,
) -> Result<TokenGrant, String> {
    let pkce = new_pkce();
    let state = b64url(&rand::random::<[u8; 12]>());
    let listener = bind_loopback().await?;

    let url = authorize_url(client_id, &pkce.challenge, &state);
    webbrowser::open(&url)
        .map_err(|e| format!("TIDAL sign-in: couldn't open the default browser: {e}"))?;

    let code = capture_redirect_code(listener, &state, timeout).await?;
    exchange_code(client_id, client_secret, &code, &pkce.verifier).await
}

async fn bind_loopback() -> Result<tokio::net::TcpListener, String> {
    tokio::net::TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .await
        .map_err(|e| {
            format!(
                "TIDAL sign-in: couldn't bind the loopback callback on port {REDIRECT_PORT} ({e}) \
                 — another sign-in may be in progress"
            )
        })
}

/// Accept one loopback request, parse `GET /callback?code=…&state=…`, verify the
/// state, send a small close-me page, and return the code. Times out if the user
/// never completes the browser login.
async fn capture_redirect_code(
    listener: tokio::net::TcpListener,
    expect_state: &str,
    timeout: std::time::Duration,
) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let accept = tokio::time::timeout_at(deadline, listener.accept()).await;
        let (mut stream, _) = match accept {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => return Err(format!("TIDAL sign-in: loopback accept failed: {e}")),
            Err(_) => return Err("TIDAL sign-in timed out waiting for the browser".to_string()),
        };

        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let target = req
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("");

        if !target.starts_with("/callback") {
            let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\n\r\n").await;
            continue;
        }

        let (body, result) = match parse_callback(target, expect_state) {
            Ok(code) => ("TIDAL sign-in complete — you can close this tab.", Ok(code)),
            Err(e) => ("TIDAL sign-in failed — return to kopuz.", Err(e)),
        };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n<html><body>{body}</body></html>",
            body.len() + 26
        );
        let _ = stream.write_all(resp.as_bytes()).await;
        let _ = stream.flush().await;
        return result;
    }
}

/// Pull the `code` from a `/callback?…` target, verifying `state` and surfacing
/// an OAuth `error` param. Values are percent-decoded — a still-encoded code
/// would get encoded a second time in the token exchange form.
fn parse_callback(target: &str, expect_state: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(&format!("http://localhost{target}"))
        .map_err(|e| format!("TIDAL sign-in: unparsable callback ({e})"))?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }
    if let Some(err) = error {
        return Err(format!("TIDAL denied the sign-in: {err}"));
    }
    if state.as_deref() != Some(expect_state) {
        return Err("TIDAL sign-in: state mismatch (possible CSRF) — try again".to_string());
    }
    code.filter(|c| !c.is_empty())
        .ok_or_else(|| "TIDAL sign-in: redirect carried no code".to_string())
}

/// Exchange a redirect authorization code + its verifier for tokens, then fill
/// in the account identity.
async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
) -> Result<TokenGrant, String> {
    let (status, json) = post_token_form(
        client_id,
        client_secret,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    if !status.is_success() {
        return Err(token_error(status, &json));
    }
    grant_with_identity(&json, None).await
}

/// Exchange a refresh token for a fresh access token, against the user's client.
#[tracing::instrument(name = "tidal.refresh", skip_all)]
pub async fn refresh_access(
    client_id: &str,
    client_secret: &str,
    refresh: &str,
) -> Result<TokenGrant, String> {
    if refresh.is_empty() {
        return Err("TIDAL: no refresh token stored".to_string());
    }
    let (status, json) = post_token_form(
        client_id,
        client_secret,
        &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
    )
    .await?;
    if !status.is_success() {
        return Err(token_error(status, &json));
    }
    grant_with_identity(&json, Some(refresh.to_string())).await
}

fn token_error(status: reqwest::StatusCode, json: &Value) -> String {
    let err = str_field(json, "error").unwrap_or_else(|| status.to_string());
    let detail = str_field(json, "error_description").unwrap_or_default();
    format!("TIDAL token {status}: {err} {detail}")
        .trim()
        .to_string()
}

/// POST the token endpoint (client id + optional secret as form params — a
/// public PKCE client sends an empty/absent secret) and return status + JSON.
async fn post_token_form(
    client_id: &str,
    client_secret: &str,
    extra: &[(&str, &str)],
) -> Result<(reqwest::StatusCode, Value), String> {
    let mut form: Vec<(&str, &str)> = vec![("client_id", client_id), ("scope", SCOPE)];
    if !client_secret.is_empty() {
        form.push(("client_secret", client_secret));
    }
    form.extend_from_slice(extra);
    let resp = http_client()
        .post(TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("TIDAL token HTTP: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("TIDAL token body: {e}"))?;
    let json: Value = serde_json::from_str(&body)
        .map_err(|e| format!("TIDAL token JSON ({status}): {e}; body: {body}"))?;
    Ok((status, json))
}

/// Turn a raw token response into a [`TokenGrant`], resolving the numeric userId
/// and account country from `/users/me` (the public token doesn't carry them).
async fn grant_with_identity(
    json: &Value,
    keep_refresh: Option<String>,
) -> Result<TokenGrant, String> {
    let access = str_field(json, "access_token").ok_or("TIDAL grant: no access_token")?;
    let refresh = str_field(json, "refresh_token").or(keep_refresh);
    let (user_id, country) = fetch_me(&access).await?;
    tracing::info!(user_id = %user_id, country = %country, "TIDAL grant parsed");
    Ok(TokenGrant {
        access,
        refresh,
        user_id,
        country,
    })
}

/// `GET /users/me` → `(numeric user id, country code)`. This is the first
/// authenticated call, so a failure here is the earliest place a
/// missing-scope/insufficient-tier problem surfaces.
async fn fetch_me(access: &str) -> Result<(String, String), String> {
    let doc = http_client()
        .get(format!("{API_V2}/users/me"))
        .bearer_auth(access)
        .header(reqwest::header::ACCEPT, JSON_API)
        .send()
        .await
        .map_err(|e| format!("TIDAL /users/me HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| {
            format!(
                "TIDAL /users/me: {e} — is `user.read` enabled on your developer.tidal.com app?"
            )
        })?
        .json::<Value>()
        .await
        .map_err(|e| format!("TIDAL /users/me JSON: {e}"))?;
    let data = doc.get("data").cloned().unwrap_or_default();
    let user_id = str_field(&data, "id")
        .filter(|s| !s.is_empty())
        .ok_or("TIDAL /users/me carried no user id")?;
    let country = data
        .get("attributes")
        .and_then(|a| str_field(a, "country"))
        .unwrap_or_else(|| "US".to_string());
    Ok((user_id, country))
}

/// Build a `openapi.tidal.com/v2` URL from path segments (each percent-encoded),
/// used for the search endpoint whose id is a raw query string.
fn v2_url(segments: &[&str]) -> reqwest::Url {
    let mut url = reqwest::Url::parse(API_V2).expect("valid base");
    if let Ok(mut seg) = url.path_segments_mut() {
        seg.extend(segments);
    }
    url
}

/// GET a JSON:API document. `segments` are appended (encoded) to `/v2`;
/// `countryCode` + caller `query` are added. Returns the parsed document.
async fn api_get(
    creds: &Creds,
    segments: &[&str],
    query: &[(&str, &str)],
) -> Result<Value, String> {
    let url = v2_url(segments);
    http_client()
        .get(url)
        .bearer_auth(&creds.access)
        .header(reqwest::header::ACCEPT, JSON_API)
        .query(&[("countryCode", creds.country.as_str())])
        .query(query)
        .send()
        .await
        .map_err(|e| format!("TIDAL API HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("TIDAL API HTTP: {e}"))?
        .json::<Value>()
        .await
        .map_err(|e| format!("TIDAL API JSON: {e}"))
}

/// Validate the access token via `/users/me`. An error containing `401` means
/// the token expired.
pub(crate) async fn get_session(creds: &Creds) -> Result<Value, String> {
    fetch_me(&creds.access)
        .await
        .map(|(id, country)| serde_json::json!({ "userId": id, "countryCode": country }))
}

/// A `(type, id)` key into a document's resource index.
type ResKey = (String, String);

fn ident(v: &Value) -> Option<ResKey> {
    Some((
        v.get("type")?.as_str()?.to_string(),
        v.get("id")?.as_str()?.to_string(),
    ))
}

/// Index every full resource (has `attributes`) in `data` + `included` by
/// `(type, id)` so relationships can be resolved.
fn index_resources(doc: &Value) -> HashMap<ResKey, Value> {
    let mut map = HashMap::new();
    for key in ["data", "included"] {
        let arr = match doc.get(key) {
            Some(Value::Array(a)) => a.clone(),
            Some(v @ Value::Object(_)) => vec![v.clone()],
            _ => continue,
        };
        for r in arr {
            if r.get("attributes").is_none() {
                continue;
            }
            if let Some(k) = ident(&r) {
                map.insert(k, r);
            }
        }
    }
    map
}

/// The `data` resource identifiers of a document, in order.
fn data_idents(doc: &Value) -> Vec<ResKey> {
    match doc.get("data") {
        Some(Value::Array(a)) => a.iter().filter_map(ident).collect(),
        Some(v @ Value::Object(_)) => ident(v).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Resource identifiers of a to-one/to-many relationship on `res`.
fn rel_idents(res: &Value, name: &str) -> Vec<ResKey> {
    let data = res
        .get("relationships")
        .and_then(|r| r.get(name))
        .and_then(|r| r.get("data"));
    match data {
        Some(Value::Array(a)) => a.iter().filter_map(ident).collect(),
        Some(v @ Value::Object(_)) => ident(v).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// The `page[cursor]` value from `links.next`, if the listing continues.
fn next_cursor(doc: &Value) -> Option<String> {
    let next = doc.get("links").and_then(|l| l.get("next"))?.as_str()?;
    let query = next.split_once('?').map(|(_, q)| q).unwrap_or(next);
    for pair in query.split('&') {
        if let Some(v) = pair
            .strip_prefix("page%5Bcursor%5D=")
            .or_else(|| pair.strip_prefix("page[cursor]="))
        {
            return Some(v.to_string());
        }
    }
    None
}

/// Parse an ISO-8601 duration (`PT3M35S`) into whole seconds.
fn parse_iso_duration(s: &str) -> u64 {
    let s = s.strip_prefix("PT").unwrap_or(s);
    let (mut total, mut num) = (0u64, 0u64);
    for c in s.chars() {
        match c {
            '0'..='9' => num = num * 10 + (c as u64 - '0' as u64),
            'H' => {
                total += num * 3600;
                num = 0;
            }
            'M' => {
                total += num * 60;
                num = 0;
            }
            'S' => {
                total += num;
                num = 0;
            }
            _ => num = 0,
        }
    }
    total
}

/// The largest artwork file URL from a resolved `artworks` resource.
fn artwork_url(idx: &HashMap<ResKey, Value>, res: &Value, rel: &str) -> Option<String> {
    let art_key = rel_idents(res, rel).into_iter().next()?;
    let art = idx.get(&art_key)?;
    let files = art.get("attributes")?.get("files")?.as_array()?;
    files
        .iter()
        .max_by_key(|f| {
            f.get("meta")
                .and_then(|m| m.get("width"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .and_then(|f| str_field(f, "href"))
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The album-grid ref for the shared cover resolver: subsonic-style
/// `tidal:<albumId>:urlhex_<hex-of-url>` (or `:none` without art).
fn album_ref(album_id: &str, cover_url: Option<&str>) -> String {
    match cover_url {
        Some(url) => format!("tidal:{album_id}:urlhex_{}", hex::encode(url.as_bytes())),
        None => format!("tidal:{album_id}:none"),
    }
}

/// Parse a v2 `tracks` resource (with its `artists`/`albums`/cover art already
/// in `idx`) into the domain model.
fn parse_track(res: &Value, idx: &HashMap<ResKey, Value>) -> Option<Track> {
    let track_id = res.get("id")?.as_str()?.to_string();
    let attrs = res.get("attributes")?;
    let title = str_field(attrs, "title").unwrap_or_default();
    let duration = str_field(attrs, "duration")
        .map(|d| parse_iso_duration(&d))
        .unwrap_or(0);

    let artists: Vec<String> = rel_idents(res, "artists")
        .iter()
        .filter_map(|k| idx.get(k))
        .filter_map(|a| a.get("attributes").and_then(|at| str_field(at, "name")))
        .collect();

    let (album_title, album_id, cover) = match rel_idents(res, "albums")
        .into_iter()
        .next()
        .and_then(|k| idx.get(&k).cloned())
    {
        Some(album) => {
            let a_id = album
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let a_title = album
                .get("attributes")
                .and_then(|at| str_field(at, "title"))
                .unwrap_or_default();
            let cover = artwork_url(idx, &album, "coverArt");
            (a_title, a_id, cover)
        }
        None => (String::new(), String::new(), None),
    };

    Some(Track {
        id: reader::models::TrackId::Server {
            service: config::MusicService::Tidal,
            item_id: track_id,
        },
        cover: cover.clone(),
        album_id: if album_id.is_empty() {
            String::new()
        } else {
            album_ref(&album_id, cover.as_deref())
        },
        title,
        artist: artists.first().cloned().unwrap_or_default(),
        album: album_title,
        duration,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists,
    })
}

/// Include paths that pull a track's artists + album + album cover into the same
/// document, prefixed by the primary relationship name (`tracks`/`items`).
fn track_includes(prefix: &str) -> String {
    format!("{prefix}.artists,{prefix}.albums,{prefix}.albums.coverArt")
}

/// Resolve every `tracks` identifier in `doc`'s `data` to a domain [`Track`].
fn tracks_from_doc(doc: &Value) -> Vec<Track> {
    let idx = index_resources(doc);
    data_idents(doc)
        .iter()
        .filter(|(t, _)| t == "tracks")
        .filter_map(|k| idx.get(k))
        .filter_map(|res| parse_track(res, &idx))
        .collect()
}

/// One page of the user's collection tracks. `cursor` is the opaque
/// `page[cursor]`; pass `None` first, then the returned value until `None`.
#[tracing::instrument(name = "tidal.favorite_tracks_page", skip(creds, user_id))]
pub(crate) async fn favorite_tracks_page(
    creds: &Creds,
    user_id: &str,
    cursor: Option<&str>,
) -> Result<(Vec<Track>, Option<String>), String> {
    let includes = track_includes("tracks");
    let mut query: Vec<(&str, &str)> = vec![("include", includes.as_str())];
    if let Some(c) = cursor {
        query.push(("page[cursor]", c));
    }
    let doc = api_get(
        creds,
        &["userCollections", user_id, "relationships", "tracks"],
        &query,
    )
    .await?;
    Ok((tracks_from_doc(&doc), next_cursor(&doc)))
}

/// Favorite / unfavorite one track on the account. Needs the `collection.write`
/// scope enabled on the app.
pub(crate) async fn set_track_favorite(
    creds: &Creds,
    user_id: &str,
    track_id: &str,
    on: bool,
) -> Result<(), String> {
    let url = v2_url(&["userCollections", user_id, "relationships", "tracks"]);
    let body = serde_json::json!({ "data": [{ "type": "tracks", "id": track_id }] });
    let http = http_client();
    let req = if on { http.post(url) } else { http.delete(url) };
    req.bearer_auth(&creds.access)
        .header(reqwest::header::CONTENT_TYPE, JSON_API)
        .header(reqwest::header::ACCEPT, JSON_API)
        .query(&[("countryCode", creds.country.as_str())])
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("TIDAL favorite HTTP: {e}"))?
        .error_for_status()
        .map_err(|e| format!("TIDAL favorite HTTP: {e}"))?;
    Ok(())
}

/// `(collected albums, their tracks, (artist name, photo URL) pairs)`.
type LibrarySnapshot = (Vec<reader::Album>, Vec<Track>, Vec<(String, String)>);

/// The user's library snapshot: collected albums (with their full track
/// listings) + collected-artist photos. Collection *tracks* stay in the
/// favorites partition (see [`favorite_tracks_page`]), matching how the other
/// backends split library vs. likes.
#[tracing::instrument(name = "tidal.fetch_library", skip_all)]
pub(crate) async fn fetch_library(creds: &Creds, user_id: &str) -> Result<LibrarySnapshot, String> {
    let mut albums = Vec::new();
    let mut tracks = Vec::new();

    let mut cursor: Option<String> = None;
    loop {
        let mut query: Vec<(&str, &str)> = vec![("include", "albums.artists,albums.coverArt")];
        if let Some(c) = &cursor {
            query.push(("page[cursor]", c));
        }
        let doc = api_get(
            creds,
            &["userCollections", user_id, "relationships", "albums"],
            &query,
        )
        .await?;
        let idx = index_resources(&doc);
        for key in data_idents(&doc).into_iter().filter(|(t, _)| t == "albums") {
            let Some(album) = idx.get(&key) else { continue };
            let album_id = key.1.clone();
            let attrs = album.get("attributes").cloned().unwrap_or_default();
            let cover = artwork_url(&idx, album, "coverArt");
            let id_ref = album_ref(&album_id, cover.as_deref());
            let year = str_field(&attrs, "releaseDate")
                .and_then(|d| d.get(..4).and_then(|y| y.parse::<u16>().ok()))
                .unwrap_or(0);
            let artist = rel_idents(album, "artists")
                .iter()
                .filter_map(|k| idx.get(k))
                .find_map(|a| a.get("attributes").and_then(|at| str_field(at, "name")))
                .unwrap_or_default();
            albums.push(reader::Album {
                id: id_ref.clone(),
                title: str_field(&attrs, "title").unwrap_or_default(),
                artist,
                genre: String::new(),
                year,
                cover_path: Some(std::path::PathBuf::from(&id_ref)),
                manual_cover: false,
            });
            match album_tracks(creds, &album_id).await {
                Ok(items) => tracks.extend(items),
                Err(e) => {
                    tracing::warn!(album_id, error = %e, "TIDAL album tracks fetch failed; skipping")
                }
            }
        }
        match next_cursor(&doc) {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }

    let artist_images = collected_artist_images(creds, user_id)
        .await
        .unwrap_or_default();
    Ok((albums, tracks, artist_images))
}

/// A collected album's full track list, in order (paginated).
async fn album_tracks(creds: &Creds, album_id: &str) -> Result<Vec<Track>, String> {
    let includes = track_includes("items");
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut query: Vec<(&str, &str)> = vec![("include", includes.as_str())];
        if let Some(c) = &cursor {
            query.push(("page[cursor]", c));
        }
        let doc = api_get(
            creds,
            &["albums", album_id, "relationships", "items"],
            &query,
        )
        .await?;
        out.extend(tracks_from_doc(&doc));
        match next_cursor(&doc) {
            Some(c) => cursor = Some(c),
            None => return Ok(out),
        }
    }
}

/// `(artist name, photo URL)` for each collected artist.
async fn collected_artist_images(
    creds: &Creds,
    user_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut query: Vec<(&str, &str)> = vec![("include", "artists.profileArt")];
        if let Some(c) = &cursor {
            query.push(("page[cursor]", c));
        }
        let doc = api_get(
            creds,
            &["userCollections", user_id, "relationships", "artists"],
            &query,
        )
        .await?;
        let idx = index_resources(&doc);
        for key in data_idents(&doc)
            .into_iter()
            .filter(|(t, _)| t == "artists")
        {
            let Some(artist) = idx.get(&key) else {
                continue;
            };
            let Some(name) = artist.get("attributes").and_then(|a| str_field(a, "name")) else {
                continue;
            };
            if let Some(photo) = artwork_url(&idx, artist, "profileArt") {
                out.push((name, photo));
            }
        }
        match next_cursor(&doc) {
            Some(c) => cursor = Some(c),
            None => return Ok(out),
        }
    }
}

/// A playlist as listed in the user's collection.
pub(crate) struct PlaylistSummary {
    pub id: String,
    pub title: String,
    pub image_url: Option<String>,
}

/// The user's collected playlists (created + followed).
#[tracing::instrument(name = "tidal.list_playlists", skip_all)]
pub(crate) async fn list_playlists(
    creds: &Creds,
    user_id: &str,
) -> Result<Vec<PlaylistSummary>, String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut query: Vec<(&str, &str)> = vec![("include", "playlists.coverArt")];
        if let Some(c) = &cursor {
            query.push(("page[cursor]", c));
        }
        let doc = api_get(
            creds,
            &["userCollections", user_id, "relationships", "playlists"],
            &query,
        )
        .await?;
        let idx = index_resources(&doc);
        for key in data_idents(&doc)
            .into_iter()
            .filter(|(t, _)| t == "playlists")
        {
            let Some(p) = idx.get(&key) else { continue };
            out.push(PlaylistSummary {
                id: key.1.clone(),
                title: p
                    .get("attributes")
                    .and_then(|a| str_field(a, "name"))
                    .unwrap_or_default(),
                image_url: artwork_url(&idx, p, "coverArt"),
            });
        }
        match next_cursor(&doc) {
            Some(c) => cursor = Some(c),
            None => return Ok(out),
        }
    }
}

/// A playlist's full track list, in order (paginated).
#[tracing::instrument(name = "tidal.playlist_entries", skip(creds))]
pub(crate) async fn get_playlist_entries(
    creds: &Creds,
    playlist_id: &str,
) -> Result<Vec<Track>, String> {
    let includes = track_includes("items");
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut query: Vec<(&str, &str)> = vec![("include", includes.as_str())];
        if let Some(c) = &cursor {
            query.push(("page[cursor]", c));
        }
        let doc = api_get(
            creds,
            &["playlists", playlist_id, "relationships", "items"],
            &query,
        )
        .await?;
        out.extend(tracks_from_doc(&doc));
        match next_cursor(&doc) {
            Some(c) => cursor = Some(c),
            None => return Ok(out),
        }
    }
}

/// Search the TIDAL catalog for tracks. The query string is the search
/// resource's id.
#[tracing::instrument(name = "tidal.search", skip(creds), fields(query = %query))]
pub(crate) async fn search_tracks(creds: &Creds, query: &str) -> Result<Vec<Track>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let includes = track_includes("tracks");
    let doc = api_get(
        creds,
        &["searchResults", query, "relationships", "tracks"],
        &[("include", includes.as_str())],
    )
    .await?;
    Ok(tracks_from_doc(&doc))
}

/// Resolve a track to a playable stream URL.
///
/// **Not supported on the public API.** `trackManifests` returns
/// Widevine-encrypted MPEG-DASH for every format, and full-length streams
/// additionally require a higher developer access tier than THIRD_PARTY. We
/// fetch the manifest only to surface the specific reason, then error — kopuz's
/// player can neither decrypt Widevine nor demux DASH.
#[tracing::instrument(name = "tidal.resolve_stream", skip(creds), fields(track_id = %track_id))]
pub(crate) async fn resolve_stream(creds: &Creds, track_id: &str) -> Result<String, String> {
    let doc = api_get(
        creds,
        &["trackManifests", track_id],
        &[
            ("manifestType", "MPEG_DASH"),
            ("formats", "FLAC"),
            ("uriScheme", "DATA"),
            ("usage", "PLAYBACK"),
            ("adaptive", "false"),
        ],
    )
    .await?;
    let attrs = doc.get("data").and_then(|d| d.get("attributes"));
    if let Some(reason) = attrs.and_then(|a| str_field(a, "previewReason")) {
        return Err(format!(
            "TIDAL playback unavailable: only a preview is offered ({reason}). \
             Full streaming needs a higher developer access tier."
        ));
    }
    let drm = attrs
        .and_then(|a| a.get("drmData"))
        .and_then(|d| str_field(d, "drmSystem"));
    Err(format!(
        "TIDAL playback isn't supported through the public API: the stream is \
         {} MPEG-DASH, which kopuz can't decrypt or demux.",
        drm.as_deref().unwrap_or("DRM-protected")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creds_pack_roundtrip() {
        let creds = Creds {
            access: "a-token".into(),
            refresh: "r-token".into(),
            country: "DE".into(),
            client_id: "cid".into(),
            client_secret: "csecret".into(),
        };
        assert_eq!(unpack_creds(&pack_creds(&creds)).as_ref(), Some(&creds));
        assert!(unpack_creds("").is_none());
    }

    #[test]
    fn parse_callback_extracts_code() {
        assert_eq!(
            parse_callback("/callback?code=ABC123&state=xy", "xy").as_deref(),
            Ok("ABC123")
        );
        assert!(parse_callback("/callback?code=ABC&state=bad", "xy").is_err());
        assert!(parse_callback("/callback?error=access_denied&state=xy", "xy").is_err());
    }

    #[test]
    fn parse_callback_percent_decodes() {
        assert_eq!(
            parse_callback("/callback?code=eyJ%2Fab%3D%3D&state=xy", "xy").as_deref(),
            Ok("eyJ/ab==")
        );
        assert_eq!(
            parse_callback("/callback?error=access%20denied&state=xy", "xy"),
            Err("TIDAL denied the sign-in: access denied".to_string())
        );
    }

    #[test]
    fn pkce_challenge_is_sha256_of_verifier() {
        let p = new_pkce();
        assert_eq!(p.challenge, b64url(&Sha256::digest(p.verifier.as_bytes())));
        assert!(!p.verifier.contains('=') && !p.verifier.contains('+'));
    }

    #[test]
    fn iso_duration_parses() {
        assert_eq!(parse_iso_duration("PT3M35S"), 215);
        assert_eq!(parse_iso_duration("PT1H2M3S"), 3723);
        assert_eq!(parse_iso_duration("PT45S"), 45);
        assert_eq!(parse_iso_duration("PT0S"), 0);
    }

    #[test]
    fn next_cursor_extracts_page_cursor() {
        let doc = serde_json::json!({
            "links": { "next": "/v2/userCollections/1/relationships/tracks?page%5Bcursor%5D=abc123&countryCode=US" }
        });
        assert_eq!(next_cursor(&doc).as_deref(), Some("abc123"));
        let none = serde_json::json!({ "links": { "self": "x" } });
        assert!(next_cursor(&none).is_none());
    }

    #[test]
    fn parse_track_resolves_jsonapi_relationships() {
        let doc = serde_json::json!({
            "data": [{
                "type": "tracks", "id": "77646437",
                "attributes": { "title": "Song", "duration": "PT3M35S" },
                "relationships": {
                    "artists": { "data": [{"type":"artists","id":"a1"},{"type":"artists","id":"a2"}] },
                    "albums": { "data": [{"type":"albums","id":"al1"}] }
                }
            }],
            "included": [
                {"type":"artists","id":"a1","attributes":{"name":"Main Artist"}},
                {"type":"artists","id":"a2","attributes":{"name":"Feature"}},
                {"type":"albums","id":"al1","attributes":{"title":"Album"},
                 "relationships":{"coverArt":{"data":{"type":"artworks","id":"art1"}}}},
                {"type":"artworks","id":"art1","attributes":{"files":[
                    {"href":"https://resources.tidal.com/images/x/80.jpg","meta":{"width":80}},
                    {"href":"https://resources.tidal.com/images/x/640.jpg","meta":{"width":640}}
                ]}}
            ]
        });
        let tracks = tracks_from_doc(&doc);
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.id.key(), "77646437");
        assert_eq!(t.id.service(), Some(config::MusicService::Tidal));
        assert_eq!(t.title, "Song");
        assert_eq!(t.artist, "Main Artist");
        assert_eq!(t.artists, vec!["Main Artist", "Feature"]);
        assert_eq!(t.album, "Album");
        assert_eq!(t.duration, 215);
        assert_eq!(
            t.cover.as_deref(),
            Some("https://resources.tidal.com/images/x/640.jpg")
        );
        assert!(t.album_id.starts_with("tidal:al1:urlhex_"));
    }

    #[test]
    fn parse_track_without_album_has_empty_ref() {
        let doc = serde_json::json!({
            "data": [{ "type": "tracks", "id": "1",
                "attributes": { "title": "Lonely", "duration": "PT10S" } }]
        });
        let tracks = tracks_from_doc(&doc);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].album_id, "");
        assert_eq!(tracks[0].duration, 10);
    }
}
