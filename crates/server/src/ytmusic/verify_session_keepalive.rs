//! Server-side session keepalive. One authenticated GET to
//! `music.youtube.com/verify_session` refreshes the session-state timer
//! that otherwise lets YouTube tear the session down after roughly
//! ten minutes of activity. The response rotates `SIDCC` /
//! `__Secure-1PSIDCC` / `__Secure-3PSIDCC`; those new values are
//! merged back into the jar so the next tick echoes them.

use std::collections::BTreeMap;
use std::time::SystemTime;

use super::clients::ORIGIN_YOUTUBE_MUSIC;
use super::innertube;

/// Hit `/verify_session` and merge any rotated cookies back into the
/// jar. Returns the updated jar if anything changed, `None` if not. A
/// transport or auth failure surfaces as `Err`; cookie-by-cookie
/// invalidation (tombstones) is reflected by a shrunk jar.
#[tracing::instrument(name = "yt.keepalive", skip(cookies))]
pub async fn tick(cookies: &str) -> Result<Option<String>, String> {
    let auth = innertube::sapisid_hash(cookies, ORIGIN_YOUTUBE_MUSIC)
        .ok_or_else(|| "SAPISID missing — cannot build SAPISIDHASH".to_string())?;

    let resp = super::innertube::http_client()
        .clone()
        .get(format!("{ORIGIN_YOUTUBE_MUSIC}/verify_session"))
        .header("User-Agent", super::clients::WEB_REMIX.user_agent)
        .header("Accept", "*/*")
        .header("Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"))
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Cookie", cookies)
        .header("Authorization", auth)
        .send()
        .await
        .map_err(|e| format!("verify_session HTTP: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("verify_session HTTP {}", resp.status()));
    }

    let mut jar = parse_jar(cookies);
    let mut changed = false;
    let now = SystemTime::now();
    for raw in resp.headers().get_all(reqwest::header::SET_COOKIE) {
        let Ok(s) = raw.to_str() else { continue };
        let Some((name, value, expired)) = parse_set_cookie(s, now) else {
            continue;
        };
        if expired {
            if jar.remove(&name).is_some() {
                changed = true;
            }
            continue;
        }
        if jar.get(&name).map(String::as_str) != Some(value.as_str()) {
            jar.insert(name, value);
            changed = true;
        }
    }
    Ok(changed.then(|| serialize_jar(&jar)))
}

fn parse_jar(header: &str) -> BTreeMap<String, String> {
    header
        .split(';')
        .filter_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

fn serialize_jar(jar: &BTreeMap<String, String>) -> String {
    jar.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Returns `(name, value, is_tombstone)` for one Set-Cookie line. A
/// cookie is a tombstone if `Expires=` parses to a past instant — YT
/// uses that pattern to delete cookies it considers invalid.
fn parse_set_cookie(raw: &str, now: SystemTime) -> Option<(String, String, bool)> {
    let mut parts = raw.split(';');
    let (name, value) = parts.next()?.trim().split_once('=')?;
    let mut expired = false;
    for attr in parts {
        let attr = attr.trim();
        let Some(exp) = attr
            .strip_prefix("Expires=")
            .or_else(|| attr.strip_prefix("expires="))
        else {
            continue;
        };
        if let Ok(t) = httpdate::parse_http_date(exp.trim())
            && t < now
        {
            expired = true;
        }
    }
    Some((name.trim().to_string(), value.trim().to_string(), expired))
}
