//! Resolve a video_id to a playable stream URL.
//!
//! Primary path: ANDROID_VR_1_61_48 + a content-bound PO token in the
//! request body. The PO token is minted via `rustypipe-botguard`, a
//! `visitor_data` is fetched once per process and reused. The URL that
//! comes back is plain (no `signatureCipher`) and accepts arbitrary Range
//! fetches — symphonia can seek inside the track normally.
//!
//! Fallback path: if anything in the primary fails (PO mint, visitor_data
//! fetch, or `/player` itself), walk `STREAM_FALLBACK_CLIENTS` from
//! `clients.rs` without a PO token. Most of those don't accept one. This
//! is a safety net for the day Google changes ANDROID_VR's behavior.

use serde_json::Value;
use tokio::sync::OnceCell;

use super::botguard;
use super::clients::{
    ANDROID_VR_1_61_48, MAIN_CLIENT, STREAM_FALLBACK_CLIENTS, YouTubeClient,
};
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
}

/// Process-wide visitor_data cache. Bound to whatever auth state was
/// active on first fetch; for our case (single signed-in user per process)
/// that's fine. Stale data is benign — it just means slightly different
/// per-IP heuristics. Replaced naturally on process restart.
static VISITOR_DATA: OnceCell<String> = OnceCell::const_new();

async fn visitor_data(cookies: &str) -> Result<&'static str, String> {
    VISITOR_DATA
        .get_or_try_init(|| async { innertube::visitor_id(Some(cookies)).await })
        .await
        .map(|s| s.as_str())
}

pub async fn resolve(video_id: &str, cookies: &str) -> Result<YtStreamInfo, String> {
    // Primary: ANDROID_VR + content_pot. Mint and visitor_data in parallel.
    let (pot_result, visitor_result) = tokio::join!(
        botguard::mint_content_pot(video_id),
        visitor_data(cookies),
    );

    let mut primary_err: Option<String> = match (pot_result.as_ref(), visitor_result) {
        (Ok(pot), Ok(visitor)) => {
            let extras = PlayerExtras {
                content_pot: Some(pot.as_str()),
                visitor_data: Some(visitor),
            };
            match innertube::player(ANDROID_VR_1_61_48, video_id, None, extras).await {
                Ok(json) => {
                    let status = PlayabilityStatus::from_response(&json);
                    if status == PlayabilityStatus::Ok {
                        if let Some(info) = pick_plain_format(&json, ANDROID_VR_1_61_48) {
                            return Ok(info);
                        }
                        Some("ANDROID_VR+pot: no plain audio format".to_string())
                    } else {
                        Some(format!(
                            "ANDROID_VR+pot playability {}: {}",
                            status.as_str(),
                            playability_reason(&json)
                        ))
                    }
                }
                Err(e) => Some(format!("ANDROID_VR+pot: {e}")),
            }
        }
        (Err(e), _) => {
            // Specifically detect "binary missing" — the fallback chain
            // will return URLs that 403 on chunked Range fetches past the
            // first MiB, so degrading to it leaves the user with a track
            // that plays for 5 s then dies. Better to surface the real
            // problem immediately.
            if e.contains("not found") || e.contains("No such file") {
                return Err(
                    "rustypipe-botguard not installed — run:\n  cargo install rustypipe-botguard --version 0.1.2".to_string()
                );
            }
            Some(format!("PO mint failed: {e}"))
        }
        (_, Err(e)) => Some(format!("visitor_data fetch failed: {e}")),
    };

    if let Some(reason) = primary_err.as_deref() {
        eprintln!("[yt-player] primary path failed ({reason}) — falling back to client chain");
    }

    // Fallback chain — kept around in case Google changes ANDROID_VR's
    // behavior. None of these accept a PO token, so they're sent bare.
    let main =
        innertube::player(MAIN_CLIENT, video_id, Some(cookies), PlayerExtras::default()).await;
    if let Ok(json) = &main {
        if PlayabilityStatus::from_response(json) == PlayabilityStatus::Ok
            && let Some(info) = pick_plain_format(json, MAIN_CLIENT)
        {
            return Ok(info);
        }
    } else if let Err(e) = main {
        primary_err = Some(format!("{}: {e}", MAIN_CLIENT.client_name));
    }
    for client in STREAM_FALLBACK_CLIENTS {
        let cookies_for = if client.login_supported { Some(cookies) } else { None };
        match innertube::player(*client, video_id, cookies_for, PlayerExtras::default()).await {
            Ok(json) => {
                let status = PlayabilityStatus::from_response(&json);
                if !status.is_attemptable() {
                    primary_err = Some(format!(
                        "{} playability {}: {}",
                        client.client_name,
                        status.as_str(),
                        playability_reason(&json)
                    ));
                    continue;
                }
                if let Some(info) = pick_plain_format(&json, *client) {
                    return Ok(info);
                }
                primary_err = Some(format!(
                    "{} returned no plain audio formats",
                    client.client_name
                ));
            }
            Err(e) => {
                primary_err = Some(format!("{}: {e}", client.client_name));
            }
        }
    }
    let primary_err = primary_err.unwrap_or_else(|| "no client returned a usable stream URL".to_string());
    eprintln!("[yt-player] InnerTube chain exhausted ({primary_err}) — trying yt-dlp fallback");
    match super::ytdlp_fallback::resolve(video_id, cookies).await {
        Ok(info) => {
            eprintln!("[yt-player] yt-dlp fallback succeeded");
            Ok(info)
        }
        Err(yt_err) => Err(format!(
            "{primary_err}; yt-dlp fallback also failed: {yt_err}"
        )),
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
    let (fmt, _) = best_webm.or(best_m4a)?;
    let url = fmt.get("url")?.as_str()?.to_string();
    let mime = fmt.get("mimeType")?.as_str()?;
    let format = AudioFormat::from_mime(mime)?;
    // `contentLength` ships as a numeric string in adaptiveFormats.
    let content_length = fmt
        .get("contentLength")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok());

    Some(YtStreamInfo {
        url,
        format,
        user_agent: client.user_agent.to_string(),
        content_length,
    })
}
