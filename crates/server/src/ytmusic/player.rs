//! Resolve a video_id to a playable stream URL.
//!
//! Primary path: native sig/n deciphering (`decipher`) against WEB_REMIX.
//! With cookies this unlocks Premium itags (~270 kbps); anonymously it serves
//! the standard ~128 kbps. No PO token is needed either way — an authenticated
//! session is its own proof-of-origin (issue #349).
//!
//! Fallback path: if deciphering is unavailable (no JS engine wired yet) or
//! fails, walk `STREAM_FALLBACK_CLIENTS` (ANDROID_VR) bare — they still hand
//! back plain ~128 kbps URLs without a PO token, so anonymous playback keeps
//! working with zero external dependencies.

use serde_json::Value;

use super::clients::{STREAM_FALLBACK_CLIENTS, WEB_REMIX, YouTubeClient};
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
}

/// Resolve a YT video to a playable stream. Native decipher (WEB_REMIX) is
/// tried first — with cookies it returns Premium itags, anonymously the
/// ~128 kbps ceiling, and no PO token in either case. On failure we fall back
/// to the ANDROID_VR clients, which still return plain ~128 kbps URLs bare —
/// no JS engine required, so anonymous playback always has a path.
pub async fn resolve(video_id: &str, cookies: Option<&str>) -> Result<YtStreamInfo, String> {
    let mut last_err = match try_native_decipher(video_id, cookies).await {
        Ok(info) => return Ok(info),
        Err(e) => {
            eprintln!("[yt-player] native decipher failed ({e}) — falling back to ANDROID_VR");
            e
        }
    };

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
    Err(format!("all stream paths failed; last error: {last_err}"))
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
    eprintln!(
        "[yt-player] resolved itag={} {} kbps {mime} via {} (plain)",
        itag.unwrap_or(0),
        bitrate / 1000,
        client.client_name
    );
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
    eprintln!(
        "[yt-player] resolved itag={} {} kbps {mime} via {} (decipher)",
        itag.unwrap_or(0),
        bitrate.unwrap_or(0) / 1000,
        client.client_name
    );
    Some(YtStreamInfo {
        url,
        format,
        user_agent: client.user_agent.to_string(),
        content_length,
        duration_secs,
        bitrate,
        itag,
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
