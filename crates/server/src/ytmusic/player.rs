//! Resolve a video_id to a playable stream URL.
//!
//! - **Premium (cookies):** WEB_REMIX + native sig/n decipher → Premium itags
//!   (~270 kbps), no PO token — an authenticated session is its own
//!   proof-of-origin.
//! - **Anonymous:** ANDROID_VR + a content-bound PO token minted by the in-app
//!   WebView (`botguard`). Anon googlevideo URLs 403 on deep/seek ranges
//!   without it; ANDROID_VR's plain URLs + the pot sustain full tracks.
//! - **Last resort:** ANDROID_VR bare (no pot — won't survive deep ranges, but
//!   better than nothing if the minter is down).
//!
//! No yt-dlp, no external binary (issue #349).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::OnceCell;
use tracing::Instrument;

use super::botguard;
use super::clients::{ANDROID_VR_1_61_48, STREAM_FALLBACK_CLIENTS, WEB_REMIX, YouTubeClient};
use super::decipher;
use super::innertube::{self, PlayerExtras};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioFormat {
    Webm,
    M4a,
}

impl AudioFormat {
    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::Webm => "webm",
            AudioFormat::M4a => "m4a",
        }
    }

    fn from_mime(mime: &str) -> Option<AudioFormat> {
        if mime.contains("webm") {
            Some(AudioFormat::Webm)
        } else if mime.contains("mp4") {
            Some(AudioFormat::M4a)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct YtStreamInfo {
    pub url: String,
    pub format: AudioFormat,
    pub user_agent: String,
    pub content_length: Option<u64>,
    pub duration_secs: Option<u64>,
    /// Average bitrate of the chosen format, in bits/sec. Surfaced for the
    /// debug bitrate readout (itag 251 ≈ 128 kbps anon, 774 ≈ 270 kbps Premium).
    pub bitrate: Option<u32>,
    /// YouTube format id of the chosen stream.
    pub itag: Option<u32>,
    /// Whether arbitrary HTTP range requests are safe on this URL. False for
    /// the no-pot decipher fallback: googlevideo 403s deep ranges without a
    /// content pot, and symphonia's probe reads the webm tail (Cues) before
    /// playing — a range-backed source would fail outright instead of playing
    /// sequentially (issue #386).
    pub range_safe: bool,
}

/// Process-wide anonymous visitor_data cache (the ANDROID_VR + pot path).
/// Refetched on process restart.
static VISITOR_DATA: OnceCell<String> = OnceCell::const_new();

async fn visitor_data(cookies: Option<&str>) -> Result<&'static str, String> {
    VISITOR_DATA
        .get_or_try_init(|| async { innertube::visitor_id(cookies).await })
        .await
        .map(|s| s.as_str())
}

/// Resolve a YT video to a playable stream. Premium (cookies) → decipher;
/// anonymous → ANDROID_VR + a webview-minted content pot; last resort →
/// ANDROID_VR bare.
#[tracing::instrument(name = "yt.resolve", skip(cookies), fields(video_id = %video_id, anon = cookies.is_none()))]
pub async fn resolve(video_id: &str, cookies: Option<&str>) -> Result<YtStreamInfo, String> {
    // A Premium *subscription* — not merely being signed in — is what exempts a
    // stream from a PO token. The signal is the itag: subscribers get 774-class
    // Opus; a signed-in *free* account gets the same 251 as anon and still 403s
    // on deep ranges without a content pot. So only short-circuit on a Premium
    // itag; otherwise fall through to the pot path (which ignores cookies — free
    // accounts cap at 251 regardless, so nothing is lost).
    // Hold a non-Premium decipher result as a graceful fallback: if no pot can
    // be minted (e.g. minter not running / unported platform), this still plays
    // from the start — only deep seeks 403 — which beats total failure.
    let mut decipher_fallback: Option<YtStreamInfo> = None;
    if let Some(c) = cookies {
        let uid = super::derive_user_id(c);
        if let Some(u) = &uid {
            seed_tier_from_db(u).await;
        }
        // Skip the Premium decipher attempt for accounts already known to be
        // non-Premium — but only when a pot can actually be minted (the decipher
        // stream is our fallback when it can't). Saves a /player round-trip per
        // track once the account's tier is learned.
        let skip = uid.as_deref().is_some_and(known_non_premium) && botguard::is_available();
        if !skip {
            match try_native_decipher(video_id, cookies).await {
                Ok(info) if is_premium_itag(info.itag) => {
                    if let Some(u) = &uid {
                        remember_tier(u, true);
                    }
                    return Ok(info);
                }
                Ok(info) => {
                    if let Some(u) = &uid {
                        remember_tier(u, false);
                    }
                    tracing::debug!(itag = ?info.itag, "signed-in but non-Premium — needs a content pot, trying ANDROID_VR");
                    decipher_fallback = Some(info);
                }
                Err(e) => tracing::debug!(error = %e, "premium decipher failed — falling back"),
            }
        }
    }

    // Anonymous: ANDROID_VR + content_pot. Mint + visitor_data in parallel.
    let mut last_err = {
        let (pot, visitor) =
            tokio::join!(botguard::mint_content_pot(video_id), visitor_data(None));
        match (pot, visitor) {
            (Ok(pot), Ok(visitor)) => {
                let extras = PlayerExtras {
                    content_pot: Some(&pot),
                    visitor_data: Some(visitor),
                    signature_timestamp: None,
                };
                match innertube::player(ANDROID_VR_1_61_48, video_id, None, extras).await {
                    Ok(json) => {
                        let status = PlayabilityStatus::from_response(&json);
                        if status == PlayabilityStatus::Ok {
                            if let Some(info) = pick_plain_format(&json, ANDROID_VR_1_61_48) {
                                return Ok(info);
                            }
                            "ANDROID_VR+pot: no plain audio format".to_string()
                        } else {
                            format!(
                                "ANDROID_VR+pot playability {}: {}",
                                status.as_str(),
                                playability_reason(&json)
                            )
                        }
                    }
                    Err(e) => format!("ANDROID_VR+pot: {e}"),
                }
            }
            (Err(e), _) => format!("PO mint: {e}"),
            (_, Err(e)) => format!("visitor_data: {e}"),
        }
    };
    tracing::debug!(%last_err, "ANDROID_VR+pot failed — trying bare clients");

    for client in STREAM_FALLBACK_CLIENTS {
        let cookies_for = if client.login_supported { cookies } else { None };
        match innertube::player(*client, video_id, cookies_for, PlayerExtras::default()).await {
            Ok(json) => {
                let status = PlayabilityStatus::from_response(&json);
                if !status.is_attemptable() {
                    last_err = format!(
                        "{} playability {}: {}",
                        client.client_name,
                        status.as_str(),
                        playability_reason(&json)
                    );
                    continue;
                }
                if let Some(info) = pick_plain_format(&json, *client) {
                    return Ok(info);
                }
                last_err = format!("{} returned no plain audio formats", client.client_name);
            }
            Err(e) => last_err = format!("{}: {e}", client.client_name),
        }
    }
    if let Some(mut info) = decipher_fallback {
        tracing::warn!(
            "no content pot available (minter not running?) — using the non-Premium decipher \
             stream sequentially (range requests 403 without a pot, so seeking is disabled)"
        );
        info.range_safe = false;
        return Ok(info);
    }
    Err(format!("all stream paths failed; last error: {last_err}"))
}

/// A Premium *subscription* yields 774-class Opus and is PO-token-exempt. Any
/// lesser itag (251, etc.) — even from a signed-in account — needs a content
/// pot for deep ranges, exactly like anonymous.
fn is_premium_itag(itag: Option<u32>) -> bool {
    // Formats only a paid subscription unlocks: 774 (Opus ~256k), 141 (AAC
    // 256k), 256/258 (AAC 192/384k). A free/anon account never sees these — it
    // caps at 251/140 (~128k) — so any of them proves the account is Premium
    // and the deciphered stream is served directly, no content pot. Only the
    // free-tier itags fall through to the ANDROID_VR + pot path. (Crucially:
    // without 141 here, a Premium user playing a video that has no Opus format
    // gets mis-tagged as free, poisoning the per-account tier cache — and with
    // a flaky minter that breaks playback for the whole 5-min TTL window.)
    matches!(itag, Some(774 | 141 | 256 | 258))
}

/// Premium-tier memo, keyed by Google user id (so switching accounts re-learns)
/// and PERSISTED through the metadata cache so a restart doesn't re-probe.
/// Lets us skip the redundant Premium decipher attempt for accounts already
/// known to be non-Premium. The "free" verdict expires daily so an account
/// upgraded free→Premium (same id, no re-sign-in) is re-checked rather than
/// pinned to ANDROID_VR forever; "premium" is trusted until an itag contradicts
/// it (remember_tier overwrites).
static ACCOUNT_PREMIUM: OnceLock<Mutex<HashMap<String, (Instant, bool)>>> = OnceLock::new();
static TIER_DB: OnceLock<db::Db> = OnceLock::new();
const FREE_TIER_TTL: Duration = Duration::from_secs(24 * 60 * 60);
// v2: "yt_tier" rows were poisoned by the 774-only is_premium_itag (a Premium
// account deciphering an AAC-only track got a persisted "free" verdict, pinning
// it to anonymous 251 for a day). New kind orphans those rows.
const TIER_META_KIND: &str = "yt_tier_v2";

/// Register the database used to persist account tiers. Called once at startup.
pub fn init_tier_store(handle: db::Db) {
    let _ = TIER_DB.set(handle);
}

fn account_premium() -> &'static Mutex<HashMap<String, (Instant, bool)>> {
    ACCOUNT_PREMIUM.get_or_init(|| Mutex::new(HashMap::new()))
}

fn known_non_premium(user_id: &str) -> bool {
    matches!(
        account_premium().lock().ok().and_then(|m| m.get(user_id).copied()),
        Some((at, false)) if at.elapsed() < FREE_TIER_TTL
    )
}

/// Warm the in-memory memo from the persisted tier, if this account hasn't been
/// seen this session. `"premium:<ts>"` seeds fresh; `"free:<ts>"` seeds with its
/// real age so the daily re-check still happens on schedule.
async fn seed_tier_from_db(user_id: &str) {
    {
        let Ok(m) = account_premium().lock() else { return };
        if m.contains_key(user_id) {
            return;
        }
    }
    let Some(handle) = TIER_DB.get() else { return };
    let Ok(Some(payload)) = handle.meta_get(user_id, TIER_META_KIND).await else {
        return;
    };
    let (verdict, ts) = match payload.split_once(':') {
        Some((v, t)) => (v.to_string(), t.parse::<u64>().unwrap_or(0)),
        None => (payload, 0),
    };
    let premium = verdict == "premium";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let age = Duration::from_secs(now.saturating_sub(ts));
    if !premium && age >= FREE_TIER_TTL {
        return; // stale free verdict — let the probe re-learn
    }
    let seeded_at = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
    if let Ok(mut m) = account_premium().lock() {
        m.entry(user_id.to_string()).or_insert((seeded_at, premium));
    }
}

fn remember_tier(user_id: &str, premium: bool) {
    if let Ok(mut m) = account_premium().lock() {
        m.insert(user_id.to_string(), (Instant::now(), premium));
    }
    if let Some(handle) = TIER_DB.get() {
        let handle = handle.clone();
        let uid = user_id.to_string();
        tokio::spawn(async move {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let payload = format!("{}:{now}", if premium { "premium" } else { "free" });
            let _ = handle.meta_put(&uid, TIER_META_KIND, &payload).await;
        }.instrument(tracing::info_span!("yt.tier_persist")));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayabilityStatus {
    Ok,
    Unknown,
    LoginRequired,
    Unplayable,
    Error,
    AgeCheck,
    /// Any future YT-side status we haven't enumerated yet — caller
    /// treats it as non-OK like the others.
    Other,
}

impl PlayabilityStatus {
    fn from_response(json: &Value) -> Self {
        match json
            .pointer("/playabilityStatus/status")
            .and_then(|v| v.as_str())
        {
            Some("OK") => PlayabilityStatus::Ok,
            Some("LOGIN_REQUIRED") => PlayabilityStatus::LoginRequired,
            Some("UNPLAYABLE") => PlayabilityStatus::Unplayable,
            Some("ERROR") => PlayabilityStatus::Error,
            Some("AGE_CHECK_REQUIRED") | Some("CONTENT_CHECK_REQUIRED") => {
                PlayabilityStatus::AgeCheck
            }
            Some(_) => PlayabilityStatus::Other,
            None => PlayabilityStatus::Unknown,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            PlayabilityStatus::Ok => "OK",
            PlayabilityStatus::Unknown => "UNKNOWN",
            PlayabilityStatus::LoginRequired => "LOGIN_REQUIRED",
            PlayabilityStatus::Unplayable => "UNPLAYABLE",
            PlayabilityStatus::Error => "ERROR",
            PlayabilityStatus::AgeCheck => "AGE_CHECK_REQUIRED",
            PlayabilityStatus::Other => "OTHER",
        }
    }

    /// Whether this status should be treated as "try the fallback chain"
    /// — covers both the explicit `UNKNOWN` we infer when YT omits the
    /// field entirely (we want to be permissive there) and `Ok`.
    fn is_attemptable(self) -> bool {
        matches!(self, PlayabilityStatus::Ok | PlayabilityStatus::Unknown)
    }
}

fn playability_reason(json: &Value) -> &str {
    json.pointer("/playabilityStatus/reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// Walks `streamingData.adaptiveFormats[]` for the best audio entry whose
/// `url` field is populated (i.e. unsigned). Returns `None` if every format
/// uses `signatureCipher` — caller falls through to the next client.
fn pick_plain_format(json: &Value, client: YouTubeClient) -> Option<YtStreamInfo> {
    let formats = json
        .pointer("/streamingData/adaptiveFormats")
        .and_then(|v| v.as_array())?;

    let mut best_webm: Option<(&Value, u64)> = None;
    let mut best_m4a: Option<(&Value, u64)> = None;
    for f in formats {
        let mime = f.get("mimeType").and_then(|v| v.as_str()).unwrap_or("");
        if !mime.starts_with("audio/") {
            continue;
        }
        if f.get("url").and_then(|v| v.as_str()).is_none() {
            continue;
        }
        let bitrate = f.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0);
        if mime.contains("webm") && best_webm.map(|(_, b)| bitrate > b).unwrap_or(true) {
            best_webm = Some((f, bitrate));
        }
        if mime.contains("mp4") && best_m4a.map(|(_, b)| bitrate > b).unwrap_or(true) {
            best_m4a = Some((f, bitrate));
        }
    }

    // Prefer webm (symphonia + libopus path) over m4a (symphonia fMP4
    // probe walks the whole file which kills startup latency).
    let (fmt, bitrate) = best_webm.or(best_m4a)?;
    let url = fmt.get("url")?.as_str()?.to_string();
    let mime = fmt.get("mimeType")?.as_str()?;
    let format = AudioFormat::from_mime(mime)?;
    let itag = fmt.get("itag").and_then(|v| v.as_u64()).map(|v| v as u32);
    let vid = json
        .pointer("/videoDetails/videoId")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    tracing::info!(video_id = %vid, itag = itag.unwrap_or(0), kbps = bitrate / 1000, mime, client = client.client_name, "stream resolved (plain)");
    // `contentLength` ships as a numeric string in adaptiveFormats.
    let content_length = fmt
        .get("contentLength")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok());
    let duration_secs = json
        .pointer("/videoDetails/lengthSeconds")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            fmt.get("approxDurationMs")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|ms| (ms + 500) / 1000)
        });

    Some(YtStreamInfo {
        url,
        format,
        user_agent: client.user_agent.to_string(),
        content_length,
        duration_secs,
        bitrate: Some(bitrate as u32),
        itag,
        range_safe: true,
    })
}

/// Best audio format by bitrate, regardless of whether it's `signatureCipher`
/// or plain — the native decipher path handles either.
fn pick_best_audio(json: &Value) -> Option<&Value> {
    json.pointer("/streamingData/adaptiveFormats")
        .and_then(|v| v.as_array())?
        .iter()
        .filter(|f| {
            f.get("mimeType")
                .and_then(|v| v.as_str())
                .map(|m| m.starts_with("audio/"))
                .unwrap_or(false)
        })
        .max_by_key(|f| f.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0))
}

/// Build a `YtStreamInfo` from an already-resolved (deciphered) URL plus the
/// format + player JSON it came from.
fn stream_info_from(
    json: &Value,
    fmt: &Value,
    url: String,
    client: YouTubeClient,
) -> Option<YtStreamInfo> {
    let mime = fmt.get("mimeType")?.as_str()?;
    let format = AudioFormat::from_mime(mime)?;
    let content_length = fmt
        .get("contentLength")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok());
    let duration_secs = json
        .pointer("/videoDetails/lengthSeconds")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            fmt.get("approxDurationMs")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|ms| (ms + 500) / 1000)
        });
    let bitrate = fmt.get("bitrate").and_then(|v| v.as_u64()).map(|v| v as u32);
    let itag = fmt.get("itag").and_then(|v| v.as_u64()).map(|v| v as u32);
    let vid = json
        .pointer("/videoDetails/videoId")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    tracing::info!(video_id = %vid, itag = itag.unwrap_or(0), kbps = bitrate.unwrap_or(0) / 1000, mime, client = client.client_name, "stream resolved (decipher)");
    Some(YtStreamInfo {
        url,
        format,
        user_agent: client.user_agent.to_string(),
        content_length,
        duration_secs,
        bitrate,
        itag,
        range_safe: true,
    })
}

/// WEB_REMIX + native sig/n decipher. Authenticated cookies (when present)
/// unlock Premium itags; **no PO token is sent** — an authenticated session is
/// its own proof-of-origin (issue #349). Anonymous callers still resolve here,
/// at the standard ~128 kbps ceiling.
async fn try_native_decipher(
    video_id: &str,
    cookies: Option<&str>,
) -> Result<YtStreamInfo, String> {
    let player = decipher::player_js(video_id).await?;
    let extras = PlayerExtras {
        signature_timestamp: Some(player.1),
        ..Default::default()
    };
    let json = innertube::player(WEB_REMIX, video_id, cookies, extras).await?;
    let status = PlayabilityStatus::from_response(&json);
    if status != PlayabilityStatus::Ok {
        return Err(format!(
            "WEB_REMIX playability {}: {}",
            status.as_str(),
            playability_reason(&json)
        ));
    }
    let fmt = pick_best_audio(&json).ok_or("WEB_REMIX returned no audio format")?;
    let url = decipher::deciphered_url(&player.0, fmt).await?;
    stream_info_from(&json, fmt, url, WEB_REMIX)
        .ok_or_else(|| "deciphered format missing fields".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pick_plain_format_carries_bitrate_and_itag() {
        let json = json!({
            "streamingData": { "adaptiveFormats": [
                { "itag": 251, "mimeType": "audio/webm; codecs=\"opus\"",
                  "bitrate": 136544, "contentLength": "3433755",
                  "url": "https://r.googlevideo.com/v?n=N" }
            ]},
            "videoDetails": { "lengthSeconds": "212" }
        });
        let info = pick_plain_format(&json, WEB_REMIX).expect("should pick a plain format");
        assert_eq!(info.itag, Some(251));
        assert_eq!(info.bitrate, Some(136544));
        assert_eq!(info.duration_secs, Some(212));
    }

    #[test]
    fn stream_info_from_carries_bitrate_and_itag() {
        let json = json!({ "videoDetails": { "lengthSeconds": "212" } });
        let fmt = json!({ "itag": 774, "mimeType": "audio/webm; codecs=\"opus\"",
                          "bitrate": 270204, "contentLength": "6852699" });
        let info = stream_info_from(&json, &fmt, "https://x/y".into(), WEB_REMIX)
            .expect("should build stream info");
        assert_eq!(info.itag, Some(774));
        assert_eq!(info.bitrate, Some(270204));
        assert_eq!(info.duration_secs, Some(212));
    }

    /// End-to-end: resolve a public track (decipher via the SubprocessEngine)
    /// and assert the resolved stream carries a real bitrate + itag — the same
    /// `YtStreamInfo` the player controller stamps onto the bottom bar.
    #[tokio::test]
    #[ignore = "hits live YouTube + needs a system JS runtime"]
    #[expect(clippy::print_stderr, reason = "test diagnostic output")]
    async fn resolve_populates_bitrate_itag_duration() {
        let info = resolve("dQw4w9WgXcQ", None).await.expect("resolve should succeed");
        eprintln!(
            "[test] resolved itag={:?} bitrate={:?} kbps duration={:?}s",
            info.itag,
            info.bitrate.map(|b| b / 1000),
            info.duration_secs,
        );
        assert!(info.itag.is_some(), "itag must be set");
        assert!(
            info.bitrate.unwrap_or(0) > 0,
            "bitrate must be > 0, got {:?}",
            info.bitrate
        );
        assert!(info.duration_secs.unwrap_or(0) > 0, "duration must be set");
    }
}
