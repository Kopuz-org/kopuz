//! Raw InnerTube HTTP transport — one function per endpoint, returns
//! parsed JSON.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha1::{Digest, Sha1};

use super::clients::{ORIGIN_YOUTUBE_MUSIC, YouTubeClient};

/// Shared HTTP client for all InnerTube + keepalive calls. Keeps the
/// TLS session and connection pool warm across the dozens of
/// /youtubei/v1 hits a typical library sync makes.
pub(super) fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Builds the `Authorization: SAPISIDHASH <ts>_<sha1(ts " " SAPISID " " origin)>` header.
pub fn sapisid_hash(cookies: &str, origin: &str) -> Option<String> {
    let sapisid = cookie_value(cookies, "SAPISID")
        .or_else(|| cookie_value(cookies, "__Secure-3PAPISID"))?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    let mut hasher = Sha1::new();
    hasher.update(format!("{ts} {sapisid} {origin}").as_bytes());
    Some(format!("SAPISIDHASH {ts}_{}", hex::encode(hasher.finalize())))
}

fn cookie_value(header: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    for part in header.split(';') {
        let p = part.trim();
        if let Some(v) = p.strip_prefix(&prefix) {
            return Some(v.to_string());
        }
    }
    None
}

fn build_context(client: YouTubeClient) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("clientName".into(), Value::String(client.client_name.into()));
    obj.insert(
        "clientVersion".into(),
        Value::String(client.client_version.into()),
    );
    obj.insert("hl".into(), Value::String("en".into()));
    obj.insert("gl".into(), Value::String("US".into()));
    if !client.os_name.is_empty() {
        obj.insert("osName".into(), Value::String(client.os_name.into()));
    }
    if !client.os_version.is_empty() {
        obj.insert("osVersion".into(), Value::String(client.os_version.into()));
    }
    if !client.device_make.is_empty() {
        obj.insert("deviceMake".into(), Value::String(client.device_make.into()));
    }
    if !client.device_model.is_empty() {
        obj.insert(
            "deviceModel".into(),
            Value::String(client.device_model.into()),
        );
    }
    if let Some(sdk) = client.android_sdk_version {
        obj.insert("androidSdkVersion".into(), Value::Number(sdk.into()));
    }
    Value::Object(obj)
}

/// Optional extras for `/player`: content-bound PO token (goes in
/// `serviceIntegrityDimensions.poToken`) and visitor_data (goes in
/// `context.client.visitorData`). ANDROID_VR + both of these is what
/// unlocks plain (non-signature-cipher) URLs that aren't rate-capped at
/// the first MiB.
#[derive(Default, Clone, Copy)]
pub struct PlayerExtras<'a> {
    pub content_pot: Option<&'a str>,
    pub visitor_data: Option<&'a str>,
}

/// Hits `/youtubei/v1/player`. For WEB_REMIX we go via music.youtube.com,
/// everything else uses www.youtube.com.
#[tracing::instrument(name = "yt.player_http", skip(cookies, extras), fields(client = client.client_name, video_id = %video_id))]
pub async fn player(
    client: YouTubeClient,
    video_id: &str,
    cookies: Option<&str>,
    extras: PlayerExtras<'_>,
) -> Result<Value, String> {
    let mut context_client = build_context(client);
    if let Some(vd) = extras.visitor_data
        && let Value::Object(ref mut m) = context_client
    {
        m.insert("visitorData".into(), Value::String(vd.to_string()));
    }

    let mut body = json!({
        "context": {
            "client": context_client,
            "user": { "lockedSafetyMode": false }
        },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    if let Some(pot) = extras.content_pot {
        body["serviceIntegrityDimensions"] = json!({ "poToken": pot });
    }
    if client.is_embedded {
        body["context"]["thirdParty"] = json!({
            "embedUrl": format!("https://www.youtube.com/watch?v={video_id}")
        });
    }

    let host = if client.client_name == "WEB_REMIX" {
        ORIGIN_YOUTUBE_MUSIC
    } else {
        "https://www.youtube.com"
    };
    let url = format!("{host}/youtubei/v1/player?prettyPrint=false");

    let mut req = http_client()
        .post(&url)
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version);
    if client.client_name.starts_with("WEB") {
        req = req
            .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
            .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    }
    if client.login_supported
        && let Some(c) = cookies
    {
        let auth = sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC)
            .ok_or_else(|| "SAPISID missing".to_string())?;
        req = req.header("Cookie", c).header("Authorization", auth);
    }

    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("player HTTP: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("player HTTP {status}: {snippet}"));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| format!("player JSON parse: {e}"))
}

/// Hits `/youtubei/v1/browse` (used for Liked Music validation and library
/// fetches). Always WEB_REMIX with cookies.
pub async fn browse(
    browse_id: &str,
    cookies: &str,
) -> Result<Value, String> {
    browse_maybe_auth(browse_id, Some(cookies)).await
}

/// Anonymous-friendly browse. Skips SAPISID + Cookie headers when no
/// cookies are provided so anonymous YT mode can hit public surfaces
/// (artists, albums, public playlists, discover home with generic
/// recs). Private surfaces (Liked, user library) will return a
/// sign-in shelf for anonymous callers — caller has to detect that.
#[tracing::instrument(name = "yt.browse", skip(cookies), fields(browse_id = %browse_id, anon = cookies.is_none()))]
pub async fn browse_maybe_auth(
    browse_id: &str,
    cookies: Option<&str>,
) -> Result<Value, String> {
    let client = super::clients::WEB_REMIX;
    let context = build_context(client);
    let body = json!({
        "context": { "client": context, "user": { "lockedSafetyMode": false } },
        "browseId": browse_id,
    });
    let mut req = http_client()
        .post(format!("{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/browse?prettyPrint=false"))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    if let Some(c) = cookies {
        let auth = sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC)
            .ok_or_else(|| "SAPISID missing".to_string())?;
        req = req.header("Cookie", c).header("Authorization", auth);
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("browse HTTP: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("browse HTTP {}", resp.status()));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| format!("browse JSON parse: {e}"))
}

/// Pull `responseContext.visitorData` out of any InnerTube response. The
/// server hands one back on every call; we just need to hold onto it across
/// `/player` calls so the content PO token's binding stays consistent.
pub fn extract_visitor_data(resp: &Value) -> Option<String> {
    resp.pointer("/responseContext/visitorData")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Hit `/browse` with a continuation token instead of a browseId — used
/// to walk paginated playlist shelves (Liked Music returns ~100 tracks
/// per page).
pub async fn browse_continuation(
    continuation: &str,
    cookies: &str,
) -> Result<Value, String> {
    browse_continuation_maybe_auth(continuation, Some(cookies)).await
}

pub async fn browse_continuation_maybe_auth(
    continuation: &str,
    cookies: Option<&str>,
) -> Result<Value, String> {
    let client = super::clients::WEB_REMIX;
    let context = build_context(client);
    let body = json!({
        "context": { "client": context, "user": { "lockedSafetyMode": false } },
    });
    let mut req = http_client()
        .post(format!(
            "{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/browse?ctoken={continuation}&continuation={continuation}&prettyPrint=false"
        ))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    if let Some(c) = cookies {
        let auth = sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC)
            .ok_or_else(|| "SAPISID missing".to_string())?;
        req = req.header("Cookie", c).header("Authorization", auth);
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("browse continuation HTTP: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("browse continuation HTTP {}", resp.status()));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| format!("browse continuation JSON parse: {e}"))
}

/// Fetch a fresh visitor_data via the lightweight `/visitor_id` endpoint.
/// Used when no prior InnerTube call has happened in this session.
pub async fn visitor_id(cookies: Option<&str>) -> Result<String, String> {
    let client = super::clients::WEB_REMIX;
    let context = build_context(client);
    let body = json!({ "context": { "client": context } });
    let mut req = http_client()
        .post(format!(
            "{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/visitor_id?prettyPrint=false"
        ))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.client_version)
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    if let Some(c) = cookies {
        let auth =
            sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC).ok_or_else(|| "SAPISID missing".to_string())?;
        req = req.header("Cookie", c).header("Authorization", auth);
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("visitor_id HTTP: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("visitor_id HTTP {}", resp.status()));
    }
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("visitor_id JSON: {e}"))?;
    extract_visitor_data(&json).ok_or_else(|| "no visitorData in response".to_string())
}
