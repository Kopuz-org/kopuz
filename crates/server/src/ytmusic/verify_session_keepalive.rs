//! Server-side session keepalive. Real Brave fires GET /verify_session
//! periodically against music.youtube.com; the response carries fresh
//! SIDCC family Set-Cookies and — more importantly — refreshes the
//! server-side session-state timer that otherwise expires after about
//! ten minutes of activity.
//!
//! Discovered while watching kopuz sessions die at exactly tick 11 of a
//! 60-second InnerTube poll. Adding a /verify_session call per tick
//! kept the session alive past tick 30, regardless of which headers
//! the polling request used. POC: `yttools session-death-watch-v3`.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::clients::ORIGIN_YOUTUBE_MUSIC;
use super::innertube;

/// Hit /verify_session and merge any rotated cookies (SIDCC family,
/// __Secure-YEC, etc.) back into the jar. Returns the updated jar if
/// anything changed, otherwise None. Returns Err only on transport or
/// auth failures — a tombstone-style invalidation surfaces as a smaller
/// jar.
pub async fn tick(cookies: &str) -> Result<Option<String>, String> {
    let auth = innertube::sapisid_hash(cookies, ORIGIN_YOUTUBE_MUSIC)
        .ok_or_else(|| "SAPISID missing — cannot build SAPISIDHASH".to_string())?;

    let resp = reqwest::Client::new()
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
    let mut rotated = false;
    for raw in resp.headers().get_all(reqwest::header::SET_COOKIE) {
        let Ok(s) = raw.to_str() else { continue };
        let Some((name, value, expired)) = parse_set_cookie(s) else {
            continue;
        };
        if expired {
            if jar.remove(&name).is_some() {
                rotated = true;
            }
            continue;
        }
        if jar.get(&name).map(|v| v.as_str()) != Some(value.as_str()) {
            jar.insert(name, value);
            rotated = true;
        }
    }
    if rotated {
        Ok(Some(serialize_jar(&jar)))
    } else {
        Ok(None)
    }
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

/// Parse one Set-Cookie header. Returns (name, value, is_expired). A
/// cookie is considered expired (tombstone) if Expires is in the past.
fn parse_set_cookie(raw: &str) -> Option<(String, String, bool)> {
    let mut parts = raw.split(';');
    let first = parts.next()?.trim();
    let (name, value) = first.split_once('=')?;
    let name = name.trim().to_string();
    let value = value.trim().to_string();
    let mut expired = false;
    for attr in parts {
        let attr = attr.trim();
        if let Some(exp) = attr.strip_prefix("Expires=").or_else(|| attr.strip_prefix("expires=")) {
            if let Some(t) = parse_http_date(exp) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if t < now {
                    expired = true;
                }
            }
        }
    }
    Some((name, value, expired))
}

fn parse_http_date(s: &str) -> Option<u64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(|c: char| c == ' ' || c == '-' || c == ':').collect();
    if parts.len() < 8 {
        return None;
    }
    let day: u32 = parts[1].parse().ok()?;
    let month = match parts[2] {
        "Jan" => 1, "Feb" => 2, "Mar" => 3, "Apr" => 4, "May" => 5, "Jun" => 6,
        "Jul" => 7, "Aug" => 8, "Sep" => 9, "Oct" => 10, "Nov" => 11, "Dec" => 12,
        _ => return None,
    };
    let year: i32 = parts[3].parse().ok()?;
    let hour: u32 = parts[4].parse().ok()?;
    let minute: u32 = parts[5].parse().ok()?;
    let second: u32 = parts[6].parse().ok()?;
    Some(epoch_seconds(year, month, day, hour, minute, second))
}

fn epoch_seconds(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> u64 {
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let mdays = [31, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 0..(month as usize - 1) {
        days += mdays[m] as i64;
    }
    days += day as i64 - 1;
    (days as u64) * 86400 + (hour as u64) * 3600 + (minute as u64) * 60 + second as u64
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

