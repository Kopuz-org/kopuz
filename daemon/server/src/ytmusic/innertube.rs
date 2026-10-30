//! Raw InnerTube HTTP transport — one function per endpoint, returns
//! parsed JSON.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha1::{Digest, Sha1};

use super::clients::{ORIGIN_YOUTUBE_MUSIC, YouTubeClient};

/// Shared HTTP client for all InnerTube + keepalive calls. Keeps the
/// TLS session and connection pool warm across the dozens of
/// /youtubei/v1 hits a typical library sync makes.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(utils::http::client)
}

/// Builds the `Authorization` header a browser sends: a
/// `<scheme> <ts>_<sha1(ts " " sid " " origin)>` for each SAPISID cookie held.
/// `None` without SAPISID (or its `__Secure-3PAPISID` stand-in).
pub fn sapisid_hash(cookies: &str, origin: &str) -> Option<String> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let user = page_id().and_then(|page| manager(cookies, &page)).flatten();
    sid_authorization(cookies, origin, ts, user.as_deref())
}

/// A brand account's hashes are bound to the Google account managing it
/// (`<ts>_<sha1(user " " ts " " sid " " origin)>_u`). Its requests with
/// unbound hashes are refused with HTTP 401, as if no cookie were sent.
fn sid_authorization(cookies: &str, origin: &str, ts: u64, user: Option<&str>) -> Option<String> {
    let third = cookie_value(cookies, "__Secure-3PAPISID");
    let sapisid = cookie_value(cookies, "SAPISID").or_else(|| third.clone())?;
    let schemes = [
        ("SAPISIDHASH", Some(sapisid)),
        ("SAPISID1PHASH", cookie_value(cookies, "__Secure-1PAPISID")),
        ("SAPISID3PHASH", third),
    ];
    let hashes: Vec<String> = schemes
        .into_iter()
        .filter_map(|(scheme, sid)| {
            let sid = sid?;
            let mut hasher = Sha1::new();
            match user {
                Some(user) => hasher.update(format!("{user} {ts} {sid} {origin}").as_bytes()),
                None => hasher.update(format!("{ts} {sid} {origin}").as_bytes()),
            }
            let hash = hex::encode(hasher.finalize());
            Some(match user {
                Some(_) => format!("{scheme} {ts}_{hash}_u"),
                None => format!("{scheme} {ts}_{hash}"),
            })
        })
        .collect();
    Some(hashes.join(" "))
}

/// The Google account (its user session id) managing each brand account, by
/// login and page id. `None` once the accounts list was asked and named no
/// manager, so a stale choice is not listed again on every call.
type Managers = HashMap<(String, String), Option<String>>;

fn managers() -> &'static Mutex<Managers> {
    static MANAGERS: OnceLock<Mutex<Managers>> = OnceLock::new();
    MANAGERS.get_or_init(Mutex::default)
}

pub(super) fn remember_manager(cookies: &str, page_id: &str, user_session_id: Option<&str>) {
    if let (Some(login), Ok(mut managers)) = (super::derive_user_id(cookies), managers().lock()) {
        managers.insert(
            (login, page_id.to_owned()),
            user_session_id.map(str::to_owned),
        );
    }
}

/// `None` until the accounts list has been asked about this login and page id.
pub(super) fn manager(cookies: &str, page_id: &str) -> Option<Option<String>> {
    let login = super::derive_user_id(cookies)?;
    managers()
        .lock()
        .ok()?
        .get(&(login, page_id.to_owned()))
        .cloned()
}

tokio::task_local! {
    /// Unscoped calls act as the Google account itself.
    static PAGE_ID: Option<String>;
}

/// Run `call` with every signed-in request in it acting as `page_id`.
pub(crate) async fn as_account<F: std::future::Future>(
    page_id: Option<String>,
    call: F,
) -> F::Output {
    PAGE_ID.scope(page_id, call).await
}

pub(super) fn page_id() -> Option<String> {
    PAGE_ID.try_with(Clone::clone).ok().flatten()
}

/// A brand account needs both `X-Goog-PageId` and `context.user.onBehalfOfUser`;
/// YouTube ignores either one alone. `X-Goog-AuthUser` picks the cookie jar's
/// session the delegation hangs off, as the web client sends it.
pub(super) fn signed(
    mut req: reqwest::RequestBuilder,
    body: &mut Value,
    cookies: &str,
    auth: String,
) -> reqwest::RequestBuilder {
    req = req.header("Cookie", cookies).header("Authorization", auth);
    if let Some(page_id) = page_id() {
        req = req
            .header("X-Goog-PageId", page_id.as_str())
            .header("X-Goog-AuthUser", "0");
        if let Some(context) = body.get_mut("context").and_then(Value::as_object_mut) {
            let user = context.entry("user").or_insert_with(|| json!({}));
            if let Some(user) = user.as_object_mut() {
                user.insert("onBehalfOfUser".into(), Value::String(page_id));
            }
        }
    }
    req
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
    obj.insert(
        "clientName".into(),
        Value::String(client.client_name.into()),
    );
    obj.insert(
        "clientVersion".into(),
        Value::String(client.version().into()),
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
        obj.insert(
            "deviceMake".into(),
            Value::String(client.device_make.into()),
        );
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
    /// `playbackContext.contentPlaybackContext.signatureTimestamp`, sourced
    /// from the player `base.js` we'll decipher against. Required for
    /// WEB_REMIX to return a `signatureCipher` matching that `base.js`.
    pub signature_timestamp: Option<u64>,
}

/// The error prefix for a request Google's abuse detection refused outright,
/// with its "Sorry..." page instead of an API response. It is sampled and
/// short-lived, so a caller that can afford to wait should retry rather than
/// treat it as an answer about the track.
pub const GOOGLE_BLOCK: &str = "google abuse block";

pub fn is_google_block(error: &str) -> bool {
    error.starts_with(GOOGLE_BLOCK)
}

/// The block page is HTML on a 403 where an API response was expected; the
/// title is the one stable thing about it.
fn is_google_block_page(status: reqwest::StatusCode, body: &str) -> bool {
    status == reqwest::StatusCode::FORBIDDEN
        && body.trim_start().starts_with("<html")
        && body.contains("<title>Sorry...</title>")
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
    if let Some(sts) = extras.signature_timestamp {
        body["playbackContext"] = json!({
            "contentPlaybackContext": { "signatureTimestamp": sts }
        });
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
        .header("X-YouTube-Client-Version", client.version());
    if client.client_name.starts_with("WEB") {
        req = req
            .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
            .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    }
    // A browser sends the visitor id as a header as well as in the context.
    if let Some(visitor) = extras.visitor_data {
        req = req.header("X-Goog-Visitor-Id", visitor);
    }
    if client.login_supported
        && let Some(c) = cookies
    {
        let auth =
            sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC).ok_or_else(|| "SAPISID missing".to_string())?;
        req = signed(req, &mut body, c, auth);
    }

    let resp = match req.json(&body).send().await {
        Ok(resp) => resp,
        Err(error) => {
            // On the span, so a failed call is not just a slow one with no
            // outcome recorded next to it.
            tracing::warn!(client = client.client_name, %error, "player request failed");
            return Err(format!("player HTTP: {error}"));
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if is_google_block_page(status, &text) {
            tracing::warn!(
                client = client.client_name,
                "player request hit Google's abuse block"
            );
            return Err(format!("{GOOGLE_BLOCK}: HTTP {status}"));
        }
        let snippet: String = text.chars().take(300).collect();
        tracing::warn!(
            client = client.client_name,
            %status,
            body = %snippet,
            "player request rejected"
        );
        return Err(format!("player HTTP {status}: {snippet}"));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| format!("player JSON parse: {e}"))
}

/// Any `/youtubei/v1/{endpoint}` call on music.youtube.com as `client`, with
/// `payload`'s fields beside the client context. Cookies are sent only to a
/// client that can sign in.
pub async fn post(
    client: YouTubeClient,
    endpoint: &str,
    payload: Value,
    cookies: Option<&str>,
) -> Result<Value, String> {
    post_as_visitor(client, endpoint, payload, cookies, None).await
}

/// Anonymous continuation tokens only answer the visitor they were issued to;
/// anyone else gets an empty page.
pub async fn post_as_visitor(
    client: YouTubeClient,
    endpoint: &str,
    payload: Value,
    cookies: Option<&str>,
    visitor: Option<&str>,
) -> Result<Value, String> {
    let mut context = build_context(client);
    if let (Some(visitor), Some(fields)) = (visitor, context.as_object_mut()) {
        fields.insert("visitorData".into(), Value::String(visitor.into()));
    }
    let mut body = json!({
        "context": {
            "client": context,
            "user": { "lockedSafetyMode": false }
        },
    });
    if let (Some(body), Value::Object(fields)) = (body.as_object_mut(), payload) {
        body.extend(fields);
    }
    let mut req = http_client()
        .post(format!(
            "{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/{endpoint}?prettyPrint=false"
        ))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.version());
    if client.client_name.starts_with("WEB") {
        req = req
            .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
            .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    }
    if let Some(visitor) = visitor {
        req = req.header("X-Goog-Visitor-Id", visitor);
    }
    if client.login_supported
        && let Some(c) = cookies.filter(|c| !c.is_empty())
    {
        let auth =
            sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC).ok_or_else(|| "SAPISID missing".to_string())?;
        req = signed(req, &mut body, c, auth);
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("{endpoint} HTTP: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{endpoint} HTTP {}", resp.status()));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| format!("{endpoint} JSON parse: {e}"))
}

/// Hits `/youtubei/v1/browse` (used for Liked Music validation and library
/// fetches). Always WEB_REMIX with cookies.
pub async fn browse(browse_id: &str, cookies: &str) -> Result<Value, String> {
    browse_maybe_auth(browse_id, Some(cookies)).await
}

/// Anonymous-friendly browse. Skips SAPISID + Cookie headers when no
/// cookies are provided so anonymous YT mode can hit public surfaces
/// (artists, albums, public playlists, discover home with generic
/// recs). Private surfaces (Liked, user library) will return a
/// sign-in shelf for anonymous callers — caller has to detect that.
#[tracing::instrument(name = "yt.browse", skip(cookies), fields(browse_id = %browse_id, anon = cookies.is_none()))]
pub async fn browse_maybe_auth(browse_id: &str, cookies: Option<&str>) -> Result<Value, String> {
    let client = super::clients::WEB_REMIX;
    let context = build_context(client);
    let mut body = json!({
        "context": { "client": context, "user": { "lockedSafetyMode": false } },
        "browseId": browse_id,
    });
    let mut req = http_client()
        .post(format!(
            "{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/browse?prettyPrint=false"
        ))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.version())
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    if let Some(c) = cookies {
        let auth =
            sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC).ok_or_else(|| "SAPISID missing".to_string())?;
        req = signed(req, &mut body, c, auth);
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
/// What a signed-in request fails with when YouTube answers it as signed out.
pub const SIGNED_OUT: &str = "YouTube answered this session as signed out";

/// A dead session still gets a 200 from most endpoints, with the content a
/// signed-out visitor sees; only the `logged_in` tracking param tells.
pub fn answered_signed_out(resp: &Value) -> bool {
    resp.pointer("/responseContext/serviceTrackingParams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|service| service["params"].as_array())
        .flatten()
        .any(|param| param["key"] == "logged_in" && param["value"] == "0")
}

pub fn extract_visitor_data(resp: &Value) -> Option<String> {
    resp.pointer("/responseContext/visitorData")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Hit `/browse` with a continuation token instead of a browseId — used
/// to walk paginated playlist shelves (Liked Music returns ~100 tracks
/// per page).
pub async fn browse_continuation(continuation: &str, cookies: &str) -> Result<Value, String> {
    browse_continuation_maybe_auth(continuation, Some(cookies)).await
}

#[tracing::instrument(name = "yt.browse_continuation", skip(continuation, cookies), fields(anon = cookies.is_none()))]
pub async fn browse_continuation_maybe_auth(
    continuation: &str,
    cookies: Option<&str>,
) -> Result<Value, String> {
    let client = super::clients::WEB_REMIX;
    let context = build_context(client);
    let mut body = json!({
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
        .header("X-YouTube-Client-Version", client.version())
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    if let Some(c) = cookies {
        let auth =
            sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC).ok_or_else(|| "SAPISID missing".to_string())?;
        req = signed(req, &mut body, c, auth);
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
    let mut body = json!({ "context": { "client": context } });
    let mut req = http_client()
        .post(format!(
            "{ORIGIN_YOUTUBE_MUSIC}/youtubei/v1/visitor_id?prettyPrint=false"
        ))
        .header("User-Agent", client.user_agent)
        .header("Content-Type", "application/json")
        .header("X-YouTube-Client-Name", client.client_id)
        .header("X-YouTube-Client-Version", client.version())
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"));
    if let Some(c) = cookies {
        let auth =
            sapisid_hash(c, ORIGIN_YOUTUBE_MUSIC).ok_or_else(|| "SAPISID missing".to_string())?;
        req = signed(req, &mut body, c, auth);
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

#[cfg(test)]
mod tests {
    use super::*;

    const SORRY_PAGE: &str = "<html><head><meta http-equiv=\"content-type\" \
        content=\"text/html; charset=utf-8\"/><title>Sorry...</title><style> body \
        { font-family: verdana, arial, sans-serif; }</style></head><body>...";

    fn sign(body: &mut Value) -> reqwest::Request {
        let req = http_client().post("https://music.youtube.com/youtubei/v1/browse");
        signed(req, body, "SAPISID=abc", "SAPISIDHASH 1_x".into())
            .build()
            .expect("request")
    }

    #[tokio::test]
    async fn a_brand_account_rides_on_the_header_and_the_context() {
        let mut body =
            json!({ "context": { "client": {}, "user": { "lockedSafetyMode": false } } });
        let req = as_account(Some("4242".into()), async { sign(&mut body) }).await;
        assert_eq!(req.headers()["X-Goog-PageId"], "4242");
        assert_eq!(req.headers()["X-Goog-AuthUser"], "0");
        assert_eq!(req.headers()["Cookie"], "SAPISID=abc");
        assert_eq!(body["context"]["user"]["onBehalfOfUser"], "4242");
        assert_eq!(body["context"]["user"]["lockedSafetyMode"], false);

        let mut bare = json!({ "context": { "client": {} } });
        as_account(Some("4242".into()), async { sign(&mut bare) }).await;
        assert_eq!(bare["context"]["user"]["onBehalfOfUser"], "4242");
    }

    #[tokio::test]
    async fn the_google_account_itself_sends_no_page_id() {
        for scoped in [false, true] {
            let mut body = json!({ "context": { "client": {}, "user": {} } });
            let req = if scoped {
                as_account(None, async { sign(&mut body) }).await
            } else {
                sign(&mut body)
            };
            assert!(req.headers().get("X-Goog-PageId").is_none());
            assert!(req.headers().get("X-Goog-AuthUser").is_none());
            assert!(body["context"]["user"].get("onBehalfOfUser").is_none());
            assert_eq!(req.headers()["Authorization"], "SAPISIDHASH 1_x");
        }
    }

    // Expected hashes are sha1 of the documented inputs, computed outside this code.
    #[test]
    fn the_google_account_signs_with_an_unbound_hash() {
        assert_eq!(
            sid_authorization("SAPISID=abc", ORIGIN_YOUTUBE_MUSIC, 1_700_000_000, None).unwrap(),
            "SAPISIDHASH 1700000000_2f3ec011e870f3fbd0238c090c2062c208cead32"
        );
        assert!(sid_authorization("SID=x", ORIGIN_YOUTUBE_MUSIC, 1, None).is_none());
    }

    #[test]
    fn a_brand_account_binds_every_hash_to_its_manager() {
        let cookies = "SAPISID=abc; __Secure-1PAPISID=one; __Secure-3PAPISID=three";
        let auth = sid_authorization(
            cookies,
            ORIGIN_YOUTUBE_MUSIC,
            1_700_000_000,
            Some("100000000000000000001"),
        )
        .unwrap();
        assert_eq!(
            auth,
            "SAPISIDHASH 1700000000_f0bd48bef376b84351fca7f94655ff60bed5cd04_u \
             SAPISID1PHASH 1700000000_c2a67dc6e6fb9173caa977f080ffbc131da2d4dc_u \
             SAPISID3PHASH 1700000000_25ea2be4f2475e0d972330fc15f33fd4e7a88680_u"
        );
    }

    #[tokio::test]
    async fn only_the_known_brand_account_of_this_login_gets_a_bound_hash() {
        let cookies = "SAPISID=bound-test";
        remember_manager(cookies, "777", Some("100000000000000000001"));
        remember_manager(cookies, "779", None);
        let hash = |page: Option<&str>, cookies: &'static str| {
            as_account(page.map(str::to_owned), async move {
                sapisid_hash(cookies, ORIGIN_YOUTUBE_MUSIC).unwrap()
            })
        };
        assert!(hash(Some("777"), cookies).await.ends_with("_u"));
        assert!(!hash(None, cookies).await.ends_with("_u"));
        assert!(!hash(Some("778"), cookies).await.ends_with("_u"));
        assert!(!hash(Some("779"), cookies).await.ends_with("_u"));
        assert!(
            !hash(Some("777"), "SAPISID=other-login")
                .await
                .ends_with("_u")
        );
    }

    #[test]
    fn the_abuse_page_is_recognised_and_nothing_else_is() {
        let forbidden = reqwest::StatusCode::FORBIDDEN;
        assert!(is_google_block_page(forbidden, SORRY_PAGE));
        assert!(is_google_block_page(
            forbidden,
            &format!("\n  {SORRY_PAGE}")
        ));

        // A 403 with an API body is a real answer about the request.
        assert!(!is_google_block_page(
            forbidden,
            r#"{"error":{"code":403,"message":"The caller does not have permission"}}"#
        ));
        // The page on any other status is not the block either.
        assert!(!is_google_block_page(reqwest::StatusCode::OK, SORRY_PAGE));
        assert!(!is_google_block_page(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            SORRY_PAGE
        ));
    }

    #[test]
    fn a_block_error_is_distinguishable_from_every_other_player_error() {
        assert!(is_google_block(&format!(
            "{GOOGLE_BLOCK}: HTTP 403 Forbidden"
        )));
        assert!(!is_google_block(
            "player HTTP 403 Forbidden: {\"error\":...}"
        ));
        assert!(!is_google_block(
            "WEB_REMIX playability LOGIN_REQUIRED: Sign in"
        ));
    }

    #[test]
    fn only_a_logged_in_zero_is_a_signed_out_answer() {
        let answer = |value: &str| {
            json!({"responseContext": {"serviceTrackingParams": [
                {"service": "CSI", "params": [{"key": "c", "value": "WEB_REMIX"}]},
                {"service": "GFEEDBACK", "params": [{"key": "logged_in", "value": value}]}
            ]}})
        };
        assert!(answered_signed_out(&answer("0")));
        assert!(!answered_signed_out(&answer("1")));
        assert!(!answered_signed_out(&json!({"responseContext": {}})));
    }
}
