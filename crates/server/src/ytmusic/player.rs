//! Resolve a video_id to a playable stream URL.
//!
//! - **Premium (cookies):** WEB_REMIX + native sig/n decipher, no PO token.
//! - **Anonymous:** VISIONOS, plain URLs with no token; then the same client
//!   with a content-bound PO token (`botguard`) if the plain request was refused.
//! - **Free (cookies):** when both anonymous requests were refused, WEB_REMIX
//!   + decipher with a content-bound PO token on the stream URL.
//!
//! No yt-dlp, no external binary (issue #349).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::OnceCell;
use tracing::Instrument;

use super::botguard;
use super::clients::{VISIONOS, WEB_REMIX, YouTubeClient};
use super::decipher;
use super::innertube::{self, PlayerExtras};
use super::tracking::PlaybackTracking;
use crate::source::{Refusal, SourceError};
use config::StreamQuality;

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
    /// `playerConfig.audioConfig.loudnessDb`.
    pub loudness_db: Option<f32>,
}

/// The visitor id every player call carries, held for the process and kept
/// across launches in the library's metadata cache.
///
/// A visitor id is YouTube's notion of "this device". Minting a new one on
/// every launch, from the same address with the same account, is what a
/// fleet of fresh devices looks like, and a fresh device asking for a stream
/// is what gets challenged. One id per identity, kept, is what a browser
/// presents. The anonymous path and the signed-in one are different
/// identities, so each keeps its own.
static VISITOR_DATA: OnceCell<String> = OnceCell::const_new();
static VISITOR_DATA_SIGNED_IN: OnceCell<String> = OnceCell::const_new();

const VISITOR_META_KIND: &str = "yt_visitor";

async fn visitor_data(cookies: Option<&str>) -> Result<&'static str, String> {
    let (cell, key) = match cookies.and_then(super::derive_user_id) {
        Some(user) => (&VISITOR_DATA_SIGNED_IN, user),
        None => (&VISITOR_DATA, "anon".to_string()),
    };
    cell.get_or_try_init(|| async {
        if let Some(saved) = db::cache::get()
            && let Ok(Some(saved)) = saved.meta_get(&key, VISITOR_META_KIND).await
            && !saved.is_empty()
        {
            return Ok(saved);
        }
        // Any stable id will do for stability's sake, so a signed-in fetch
        // that yields none falls back to an anonymous one filed under the
        // account: the point is that the same id comes back next launch.
        let fresh = match innertube::visitor_id(cookies).await {
            Ok(id) => id,
            Err(error) if cookies.is_some() => {
                tracing::debug!(%error, "signed-in visitor id fetch failed; using an anonymous one");
                innertube::visitor_id(None).await?
            }
            Err(error) => return Err(error),
        };
        if let Some(handle) = db::cache::get()
            && let Err(error) = handle.meta_put(&key, VISITOR_META_KIND, &fresh).await
        {
            tracing::warn!(%error, "storing the visitor id failed; it will be minted again next launch");
        }
        Ok(fresh)
    })
    .await
    .map(|s| s.as_str())
}

/// Resolve a YT video to a playable stream. Premium (cookies) → decipher;
/// anonymous → VISIONOS, plain; then VISIONOS with a headless-minted content
/// pot if the plain request was refused.
///
/// A failure that looks like YouTube's side or the network having a moment
/// is asked once more before it is reported: those come in bursts that a
/// few seconds outlast, and a reported one stops the queue.
#[tracing::instrument(name = "yt.resolve", skip(cookies), fields(video_id = %video_id, anon = cookies.is_none()))]
pub async fn resolve(
    video_id: &str,
    cookies: Option<&str>,
    quality: StreamQuality,
) -> Result<YtStreamInfo, StreamFailure> {
    let mut attempt = resolve_once(video_id, cookies, quality).await;
    for delay in TRANSIENT_RETRY_DELAYS {
        match &attempt {
            Err(failure) if failure.is_transient() => {
                tracing::info!(?delay, refusal = ?failure.refusal, "every stream path failed; asking again");
                tokio::time::sleep(delay).await;
                attempt = resolve_once(video_id, cookies, quality).await;
            }
            _ => break,
        }
    }
    attempt
}

const TRANSIENT_RETRY_DELAYS: [Duration; 1] = [Duration::from_secs(3)];

/// A resolve that failed, with what YouTube's answers say about why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFailure {
    pub refusal: Refusal,
    pub message: String,
}

impl StreamFailure {
    fn is_transient(&self) -> bool {
        matches!(self.refusal, Refusal::Network | Refusal::ServiceRefused)
    }
}

impl std::fmt::Display for StreamFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<StreamFailure> for String {
    fn from(failure: StreamFailure) -> Self {
        failure.message
    }
}

impl From<StreamFailure> for SourceError {
    fn from(failure: StreamFailure) -> Self {
        SourceError::Refused(failure.refusal, failure.message)
    }
}

async fn resolve_once(
    video_id: &str,
    cookies: Option<&str>,
    quality: StreamQuality,
) -> Result<YtStreamInfo, StreamFailure> {
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
    let mut decipher_err: Option<String> = None;
    let mut skipped_signed_in = false;
    if let Some(c) = cookies {
        let uid = super::derive_user_id(c);
        if let Some(u) = &uid {
            seed_tier_from_db(u).await;
        }
        // Skip the Premium decipher attempt for accounts already known to be
        // non-Premium — but only when a pot can actually be minted (the decipher
        // stream is our fallback when it can't). Saves a /player round-trip per
        // track once the account's tier is learned.
        skipped_signed_in =
            uid.as_deref().is_some_and(known_non_premium) && botguard::is_available();
        if !skipped_signed_in {
            match signed_in_with_retry(video_id, cookies, quality).await {
                // Tier comes from what was offered, not picked: a free-tier itag
                // from a Premium session is still exempt from the token.
                Ok((info, true)) => {
                    if let Some(u) = &uid {
                        remember_tier(u, true);
                    }
                    return Ok(info);
                }
                Ok((info, false)) => {
                    if let Some(u) = &uid {
                        remember_tier(u, false);
                    }
                    tracing::debug!(itag = ?info.itag, "signed-in but non-Premium — trying the anonymous client");
                    decipher_fallback = Some(info);
                }
                Err(e) => {
                    // Warn, not debug: for a signed-in account this is the
                    // path that was supposed to work, and every path after it
                    // is an anonymous one YouTube is entitled to refuse.
                    tracing::warn!(error = %e, "signed-in stream path failed — falling back");
                    decipher_err = Some(e);
                }
            }
        }
    }

    // Anonymous: VISIONOS with the kept visitor id. Plain URLs, no token.
    let visitor = match visitor_data(None).await {
        Ok(visitor) => Some(visitor),
        Err(error) => {
            tracing::warn!(%error, "no visitor id for the anonymous player call");
            None
        }
    };
    let extras = PlayerExtras {
        visitor_data: visitor,
        ..Default::default()
    };
    let anonymous_err = match anonymous_attempt(video_id, extras, quality).await {
        Ok(info) => match recheck_gated(
            info,
            || anonymous_attempt(video_id, extras, quality),
            VISIONOS.client_name,
        )
        .await
        {
            Ok(info) => return Ok(info),
            Err(error) => error,
        },
        Err(error) => error,
    };
    tracing::debug!(%anonymous_err, "anonymous path failed");

    // The same client with a content-bound token. yt-dlp marks it neither
    // required nor recommended for this client, so it is asked for only
    // once the plain request was refused: that is the one case the token
    // can change the answer, and minting is a V8 round trip.
    let with_pot_err = if innertube::is_google_block(&anonymous_err) {
        "not attempted behind Google's abuse page".to_string()
    } else {
        match botguard::mint_content_pot(video_id).await {
            Ok(pot) => {
                let extras = PlayerExtras {
                    content_pot: Some(&pot),
                    visitor_data: visitor,
                    signature_timestamp: None,
                };
                match anonymous_attempt(video_id, extras, quality).await {
                    Ok(info) => {
                        match recheck_gated(
                            info,
                            || anonymous_attempt(video_id, extras, quality),
                            VISIONOS.client_name,
                        )
                        .await
                        {
                            Ok(info) => return Ok(info),
                            Err(error) => error,
                        }
                    }
                    Err(error) => error,
                }
            }
            Err(error) => format!("PO mint: {error}"),
        }
    };

    // The skip above bets that the anonymous client will play. When it did
    // not, the account's own stream is still there to try.
    if skipped_signed_in {
        match signed_in_with_retry(video_id, cookies, quality).await {
            Ok((info, true)) => {
                if let Some(u) = cookies.and_then(super::derive_user_id) {
                    remember_tier(&u, true);
                }
                return Ok(info);
            }
            Ok((info, false)) => decipher_fallback = Some(info),
            Err(e) => {
                tracing::warn!(error = %e, "signed-in stream path failed after the anonymous ones");
                decipher_err = Some(e);
            }
        }
    }

    // A free account's deciphered URL is gated like an anonymous one, and a
    // content pot carried on the URL itself is what lifts it.
    if let Some(info) = decipher_fallback.take_if(|_| botguard::is_available()) {
        let again = || async {
            let (info, _) = signed_in_with_retry(video_id, cookies, quality).await?;
            let pot = botguard::remint_content_pot(video_id).await?;
            Ok(with_pot(info, &pot))
        };
        let minted = botguard::mint_content_pot(video_id).await;
        match minted.map(|pot| with_pot(info.clone(), &pot)) {
            Ok(with_pot) => match recheck_gated(with_pot, again, WEB_REMIX.client_name).await {
                Ok(info) => return Ok(info),
                Err(error) => decipher_err = Some(error),
            },
            Err(error) => {
                tracing::warn!(%error, "no content pot for the signed-in stream");
                decipher_fallback = Some(info);
            }
        }
    }

    if let Some(mut info) = decipher_fallback {
        tracing::warn!(
            "anonymous paths refused — using the non-Premium decipher stream sequentially \
             (range requests 403 without a token, so seeking is disabled)"
        );
        info.range_safe = false;
        return Ok(info);
    }
    Err(all_paths_failed(
        decipher_err.as_deref(),
        &anonymous_err,
        &with_pot_err,
    ))
}

/// Name the client that spoke, so the combined report says which path
/// failed.
///
/// Google's abuse page is passed through unlabelled, because that identity
/// lives in the prefix the transport writes and
/// [`innertube::is_google_block`] reads it back with `starts_with`. Labelling
/// it hid the block from the caller, which then paid for a token mint that
/// could not change the answer.
fn labelled(error: String) -> String {
    if innertube::is_google_block(&error) {
        error
    } else {
        format!("{}: {error}", VISIONOS.client_name)
    }
}

/// One anonymous `/player` call, reported as a stream or as the reason it
/// is not one.
async fn anonymous_attempt(
    video_id: &str,
    extras: PlayerExtras<'_>,
    quality: StreamQuality,
) -> Result<YtStreamInfo, String> {
    let json = innertube::player(VISIONOS, video_id, None, extras)
        .await
        .map_err(labelled)?;
    let status = PlayabilityStatus::from_response(&json);
    if !status.is_attemptable() {
        return Err(format!(
            "{} playability {}: {}",
            VISIONOS.client_name,
            status.as_str(),
            playability_reason(&json)
        ));
    }
    pick_plain_format(&json, VISIONOS, quality)
        .ok_or_else(|| format!("{} returned no plain audio format", VISIONOS.client_name))
}

const GATED_RETRIES: usize = 2;

// Well under the range source's timeout: a hung probe passes.
const GATE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Some plain URLs are gated behind a PO token the response never asks for:
/// ranges past the first megabyte get 403, at random, so a repeat request
/// usually returns a clean URL. `again` must reuse the original extras.
async fn recheck_gated<F, Fut>(
    info: YtStreamInfo,
    mut again: F,
    client: &str,
) -> Result<YtStreamInfo, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<YtStreamInfo, String>>,
{
    let mut current = info;
    for retry in 0..=GATED_RETRIES {
        if !is_gated(&current).await {
            return Ok(current);
        }
        if retry == GATED_RETRIES {
            break;
        }
        tracing::info!(
            retry = retry + 1,
            "googlevideo refused the tail of the stream; resolving again"
        );
        current = again().await?;
    }
    Err(format!(
        "{client}: googlevideo refused the stream on every resolve"
    ))
}

/// Any probe failure other than 403/410 passes; the range source rechecks the tail on open.
async fn is_gated(info: &YtStreamInfo) -> bool {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let Some(last) = info.content_length.and_then(|len| len.checked_sub(1)) else {
        return false;
    };
    let client = CLIENT.get_or_init(|| {
        utils::http::builder()
            .timeout(GATE_PROBE_TIMEOUT)
            .build()
            .unwrap_or_default()
    });
    let started = Instant::now();
    let response = client
        .get(&info.url)
        .header(reqwest::header::USER_AGENT, &info.user_agent)
        .header(reqwest::header::RANGE, format!("bytes={last}-{last}"))
        .send()
        .await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match response {
        Ok(response) => {
            let status = response.status();
            let gated = matches!(
                status,
                reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::GONE
            );
            tracing::debug!(%status, gated, elapsed_ms, "googlevideo tail probe");
            gated
        }
        Err(error) => {
            tracing::debug!(%error, elapsed_ms, "googlevideo tail probe failed");
            false
        }
    }
}

/// Why every path failed, not only the last one.
///
/// The anonymous client answers LOGIN_REQUIRED for anything gated, so that is
/// the expected ending for such a track -- reporting it alone said "sign in"
/// to someone who already was, and hid the signed-in path's actual error
/// behind a debug line nobody runs with.
fn all_paths_failed(decipher: Option<&str>, anonymous: &str, with_pot: &str) -> StreamFailure {
    let mut message = String::from("all stream paths failed");
    match decipher {
        Some(error) => message.push_str(&format!("; signed-in: {error}")),
        None => message.push_str("; signed-in: not attempted"),
    }
    message.push_str(&format!(
        "; anonymous: {anonymous}; anonymous+pot: {with_pot}"
    ));
    StreamFailure {
        refusal: classify(decipher, anonymous, with_pot),
        message,
    }
}

/// What the three answers say together. A bot check anywhere is the answer,
/// since it is about the network and no other path can get past it. An item
/// YouTube calls unavailable is that whichever client said so. Only then is
/// a signed-in LOGIN_REQUIRED the session's fault: the anonymous client says
/// LOGIN_REQUIRED for anything gated, so it proves nothing about the account.
fn classify(decipher: Option<&str>, anonymous: &str, with_pot: &str) -> Refusal {
    let answers = [decipher.unwrap_or_default(), anonymous, with_pot];
    if answers
        .iter()
        .any(|answer| innertube::is_google_block(answer) || answer.contains("not a bot"))
    {
        return Refusal::Challenged;
    }
    if answers.iter().any(|answer| says_unavailable(answer)) {
        return Refusal::Unavailable;
    }
    if decipher.is_some_and(|answer| answer.contains("playability LOGIN_REQUIRED")) {
        return Refusal::SessionExpired;
    }
    if is_transport_error(anonymous) && decipher.is_none_or(is_transport_error) {
        return Refusal::Network;
    }
    Refusal::ServiceRefused
}

fn says_unavailable(answer: &str) -> bool {
    const MARKERS: [&str; 4] = [
        "playability error",
        "unavailable",
        "not available",
        "private",
    ];
    let answer = answer.to_ascii_lowercase();
    answer.contains("playability") && MARKERS.iter().any(|marker| answer.contains(marker))
}

/// No answer at all: the request never got a status line back.
fn is_transport_error(answer: &str) -> bool {
    answer.contains("HTTP: ") || answer.contains("fetch: ")
}

fn with_pot(info: YtStreamInfo, pot: &str) -> YtStreamInfo {
    YtStreamInfo {
        url: format!("{}&pot={pot}", info.url),
        ..info
    }
}

/// A Premium *subscription* yields 774-class Opus and is PO-token-exempt. Any
/// lesser itag (251, etc.) — even from a signed-in account — needs a content
/// pot for deep ranges, exactly like anonymous.
fn is_premium_itag(itag: Option<u32>) -> bool {
    // Formats only a paid subscription unlocks: 774 (Opus ~256k), 141 (AAC
    // 256k), 256/258 (AAC 192/384k). A free/anon account never sees these — it
    // caps at 251/140 (~128k) — so any of them proves the account is Premium
    // and the deciphered stream is served directly, no content pot. Only the
    // free-tier itags fall through to the anonymous path. (Crucially:
    // without 141 here, a Premium user playing a video that has no Opus format
    // gets mis-tagged as free, poisoning the per-account tier cache — and with
    // a flaky minter that breaks playback for the whole 5-min TTL window.)
    matches!(itag, Some(774 | 141 | 256 | 258))
}

/// Premium-tier memo, keyed by Google user id (so switching accounts re-learns)
/// and PERSISTED through the metadata cache so a restart doesn't re-probe.
/// Lets us skip the redundant Premium decipher attempt for accounts already
/// known to be non-Premium.
///
/// Trust is asymmetric, because the free signal is weak: ONE non-premium itag
/// can mean a free account — but also a track with no premium encodes, or a
/// /player response served unauthenticated by a transient cookie hiccup. So a
/// premium verdict survives contradictions (a real downgrade just costs one
/// extra /player attempt per track until tiers re-learn at sign-in), and the
/// free pin is short — a mis-pinned Premium account recovers in minutes,
/// while a truly free account merely re-pays one probe per window.
static ACCOUNT_PREMIUM: OnceLock<Mutex<HashMap<String, (Instant, bool)>>> = OnceLock::new();
static TIER_DB: OnceLock<db::Db> = OnceLock::new();
const FREE_TIER_TTL: Duration = Duration::from_secs(30 * 60);
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
        let Ok(m) = account_premium().lock() else {
            return;
        };
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
    if !premium {
        // Asymmetric trust (see ACCOUNT_PREMIUM): a known-premium account is
        // never downgraded by a single non-premium itag — the track may just
        // lack premium encodes. The pot path still serves THIS stream fine.
        let was_premium = account_premium()
            .lock()
            .ok()
            .and_then(|m| m.get(user_id).map(|(_, p)| *p))
            .unwrap_or(false);
        if was_premium {
            tracing::info!(
                "yt: non-premium itag from a known-Premium account — keeping the premium verdict (track without premium encodes, or a transient auth hiccup)"
            );
            return;
        }
    }
    if let Ok(mut m) = account_premium().lock() {
        m.insert(user_id.to_string(), (Instant::now(), premium));
    }
    if let Some(handle) = TIER_DB.get() {
        let handle = handle.clone();
        let uid = user_id.to_string();
        tokio::spawn(
            async move {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let payload = format!("{}:{now}", if premium { "premium" } else { "free" });
                let _ = handle.meta_put(&uid, TIER_META_KIND, &payload).await;
            }
            .instrument(tracing::info_span!("yt.tier_persist")),
        );
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

    /// Whether the response is worth reading formats out of: `Ok`, and the
    /// `Unknown` inferred when YouTube omits the field entirely.
    fn is_attemptable(self) -> bool {
        matches!(self, PlayabilityStatus::Ok | PlayabilityStatus::Unknown)
    }
}

fn playability_reason(json: &Value) -> &str {
    json.pointer("/playabilityStatus/reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// Only formats with a plain `url` count. `None` when every one uses
/// `signatureCipher`, so the caller falls through to the next client.
fn pick_plain_format(
    json: &Value,
    client: YouTubeClient,
    quality: StreamQuality,
) -> Option<YtStreamInfo> {
    let formats = json
        .pointer("/streamingData/adaptiveFormats")
        .and_then(|v| v.as_array())?
        .iter()
        .filter(|f| f.get("url").and_then(|v| v.as_str()).is_some());
    let fmt = choose_format(formats, quality, true)?;
    let bitrate = fmt.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0);
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
        loudness_db: loudness_db(json),
    })
}

/// WEB_REMIX's `loudnessDb` is relative to this target. VISIONOS states absolute
/// LKFS against -14, so it is rebased here to level a queue mixing both clients.
const YTM_LOUDNESS_TARGET_LKFS: f64 = -7.0;

fn loudness_db(json: &Value) -> Option<f32> {
    let audio = json.pointer("/playerConfig/audioConfig")?;
    audio
        .get("trackAbsoluteLoudnessLkfs")
        .or_else(|| audio.get("perceptualLoudnessDb"))
        .and_then(Value::as_f64)
        .map(|lkfs| lkfs - YTM_LOUDNESS_TARGET_LKFS)
        .or_else(|| audio.get("loudnessDb").and_then(Value::as_f64))
        .map(|db| db as f32)
        .filter(|db| db.is_finite())
}

/// Above this average bitrate a format is one a subscription unlocks.
const NORMAL_CAP_BPS: u64 = 160_000;
/// Itags 249 (Opus) and 139 (AAC) average about 50 kbps.
const LOW_CAP_BPS: u64 = 64_000;

fn itag_of(format: &Value) -> Option<u32> {
    format
        .get("itag")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
}

/// `averageBitrate` where YouTube sends it: `bitrate` is the peak, which for
/// 251 can sit above the Normal cap while the stream averages ~130 kbps.
fn average_bitrate(format: &Value) -> u64 {
    format
        .get("averageBitrate")
        .or_else(|| format.get("bitrate"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// `prefer_webm` puts webm first because the fMP4 probe walks the whole file.
/// Normal and Low never take a Premium itag and fall back to the smallest format.
fn choose_format<'a>(
    formats: impl IntoIterator<Item = &'a Value>,
    quality: StreamQuality,
    prefer_webm: bool,
) -> Option<&'a Value> {
    let audio: Vec<&Value> = formats
        .into_iter()
        .filter(|f| {
            f.get("mimeType")
                .and_then(|v| v.as_str())
                .is_some_and(|m| m.starts_with("audio/"))
        })
        .collect();
    let is_webm = |f: &Value| {
        f.get("mimeType")
            .and_then(|v| v.as_str())
            .is_some_and(|m| m.contains("webm"))
    };
    let peak = |f: &Value| f.get("bitrate").and_then(|v| v.as_u64()).unwrap_or(0);
    let cap = match quality {
        StreamQuality::High if prefer_webm => {
            return audio
                .iter()
                .copied()
                .filter(|f| is_webm(f))
                .max_by_key(|f| peak(f))
                .or_else(|| audio.iter().copied().max_by_key(|f| peak(f)));
        }
        StreamQuality::High => return audio.iter().copied().max_by_key(|f| peak(f)),
        StreamQuality::Normal => NORMAL_CAP_BPS,
        StreamQuality::Low => LOW_CAP_BPS,
    };
    let standard = || {
        audio
            .iter()
            .copied()
            .filter(|f| !is_premium_itag(itag_of(f)))
    };
    standard()
        .filter(|f| average_bitrate(f) <= cap)
        .max_by_key(|f| (is_webm(f), average_bitrate(f)))
        .or_else(|| standard().min_by_key(|f| average_bitrate(f)))
        .or_else(|| audio.iter().copied().min_by_key(|f| average_bitrate(f)))
}

fn offers_premium(json: &Value) -> bool {
    json.pointer("/streamingData/adaptiveFormats")
        .and_then(|v| v.as_array())
        .is_some_and(|formats| formats.iter().any(|f| is_premium_itag(itag_of(f))))
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
    let bitrate = fmt
        .get("bitrate")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
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
        loudness_db: loudness_db(json),
    })
}

/// WEB_REMIX + native sig/n decipher. Authenticated cookies (when present)
/// unlock Premium itags; **no PO token is sent** — an authenticated session is
/// its own proof-of-origin (issue #349). Anonymous callers still resolve here,
/// at the standard ~128 kbps ceiling.
/// How long to wait before each further attempt when Google's abuse page
/// answers instead of the API. It is sampled per request and clears within
/// seconds; a retry by hand was enough, so this is that retry, done for the
/// user. Anything longer would make a stuck track worse than a skipped one.
const BLOCK_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

/// The signed-in path, retried across Google's block page. Any other failure
/// is an answer about the track and is not retried.
async fn signed_in_with_retry(
    video_id: &str,
    cookies: Option<&str>,
    quality: StreamQuality,
) -> Result<(YtStreamInfo, bool), String> {
    let mut attempt = try_native_decipher(video_id, cookies, quality).await;
    for delay in BLOCK_RETRY_DELAYS {
        match &attempt {
            Err(error) if innertube::is_google_block(error) => {
                tracing::info!(
                    ?delay,
                    "blocked by Google's abuse page; retrying the signed-in path"
                );
                tokio::time::sleep(delay).await;
                attempt = try_native_decipher(video_id, cookies, quality).await;
            }
            _ => break,
        }
    }
    attempt
}

/// The deciphered stream, and whether the account was offered Premium formats.
async fn try_native_decipher(
    video_id: &str,
    cookies: Option<&str>,
    quality: StreamQuality,
) -> Result<(YtStreamInfo, bool), String> {
    let player = decipher::player_js(video_id).await?;
    // The same device identity a browser would present with these cookies;
    // a signed-in request with none is the odd one out.
    let visitor = match visitor_data(cookies).await {
        Ok(visitor) => Some(visitor),
        // Proceeding without one is the state that draws challenges, so it
        // is not something to do quietly.
        Err(error) => {
            tracing::warn!(%error, "no visitor id for the signed-in player call");
            None
        }
    };
    let extras = PlayerExtras {
        signature_timestamp: Some(player.1),
        visitor_data: visitor,
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
    if let Some(cookies) = cookies {
        remember_tracking(cookies, video_id, &json);
    }
    let formats = json
        .pointer("/streamingData/adaptiveFormats")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten();
    let fmt = choose_format(formats, quality, false).ok_or("WEB_REMIX returned no audio format")?;
    let url = decipher::deciphered_url(&player.0, fmt).await?;
    let info = stream_info_from(&json, fmt, url, WEB_REMIX)
        .ok_or_else(|| "deciphered format missing fields".to_string())?;
    Ok((info, offers_premium(&json)))
}

/// Tracking URLs from signed-in `player` responses, keyed by account and video.
/// Anonymous responses never land here: their URLs carry the anonymous identity.
static TRACKING: OnceLock<Mutex<HashMap<String, PlaybackTracking>>> = OnceLock::new();
const TRACKING_CACHE_LIMIT: usize = 64;

fn tracking_key(cookies: &str, video_id: &str) -> Option<String> {
    let user = super::derive_user_id(cookies)?;
    let page = innertube::page_id().unwrap_or_default();
    Some(format!("{user}/{page}/{video_id}"))
}

fn remember_tracking(cookies: &str, video_id: &str, json: &Value) {
    let (Some(key), Some(tracking)) = (
        tracking_key(cookies, video_id),
        PlaybackTracking::from_player(json),
    ) else {
        return;
    };
    if let Ok(mut cache) = TRACKING.get_or_init(Default::default).lock() {
        if cache.len() >= TRACKING_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, tracking);
    }
}

/// Cached when the stream path already made a signed-in `player` call, else
/// fetched with one of its own.
pub async fn playback_tracking(video_id: &str, cookies: &str) -> Result<PlaybackTracking, String> {
    let key = tracking_key(cookies, video_id).ok_or("SAPISID missing")?;
    let cached = TRACKING
        .get_or_init(Default::default)
        .lock()
        .ok()
        .and_then(|mut cache| cache.remove(&key));
    if let Some(tracking) = cached {
        return Ok(tracking);
    }
    let extras = PlayerExtras {
        signature_timestamp: decipher::player_js(video_id).await.ok().map(|js| js.1),
        visitor_data: signed_in_visitor(cookies).await,
        ..Default::default()
    };
    let json = innertube::player(WEB_REMIX, video_id, Some(cookies), extras).await?;
    let status = PlayabilityStatus::from_response(&json);
    if status != PlayabilityStatus::Ok {
        return Err(format!("WEB_REMIX playability {}", status.as_str()));
    }
    PlaybackTracking::from_player(&json).ok_or_else(|| "no playbackTracking".to_string())
}

/// The signed-in path alone, once, for a connection test.
pub(crate) async fn probe_signed_in(video_id: &str, cookies: &str) -> Result<YtStreamInfo, String> {
    try_native_decipher(video_id, Some(cookies), StreamQuality::High)
        .await
        .map(|(info, _)| info)
}

/// The anonymous path alone, once, including whether googlevideo actually
/// serves the URL it handed out.
pub(crate) async fn probe_anonymous(video_id: &str) -> Result<YtStreamInfo, String> {
    let extras = PlayerExtras {
        visitor_data: visitor_data(None).await.ok(),
        ..Default::default()
    };
    let info = anonymous_attempt(video_id, extras, StreamQuality::High).await?;
    if is_gated(&info).await {
        return Err(format!(
            "{} handed out a stream URL that googlevideo refuses",
            VISIONOS.client_name
        ));
    }
    Ok(info)
}

pub async fn signed_in_visitor(cookies: &str) -> Option<&'static str> {
    super::derive_user_id(cookies)?;
    visitor_data(Some(cookies)).await.ok()
}

/// A picture-only stream; the sound plays through the engine from the audio stream.
#[derive(Clone, Debug)]
pub struct YtVideoStream {
    pub url: String,
    pub mime: String,
    pub user_agent: String,
    pub content_length: Option<u64>,
}

/// Taller is wasted on a now-playing view and costs bandwidth the audio needs.
const MAX_VIDEO_HEIGHT: u64 = 1080;

/// Resolve the picture of a music video. The anonymous client hands out plain
/// URLs for video formats, so it goes first even for a signed-in account; the
/// deciphered signed-in path is the fallback for what it refuses.
#[tracing::instrument(name = "yt.resolve_video", skip(cookies), fields(video_id = %video_id))]
pub async fn resolve_video(video_id: &str, cookies: Option<&str>) -> Result<YtVideoStream, String> {
    let visitor = visitor_data(None).await.ok();
    let anonymous = async |extras: PlayerExtras<'_>| -> Result<YtVideoStream, String> {
        let json = innertube::player(VISIONOS, video_id, None, extras)
            .await
            .map_err(labelled)?;
        let status = PlayabilityStatus::from_response(&json);
        if !status.is_attemptable() {
            return Err(format!(
                "{} playability {}: {}",
                VISIONOS.client_name,
                status.as_str(),
                playability_reason(&json)
            ));
        }
        let format = pick_video_format(&json, true)
            .ok_or_else(|| format!("{} returned no plain video format", VISIONOS.client_name))?;
        video_stream_from(
            format,
            format["url"].as_str().unwrap_or_default().to_string(),
            VISIONOS,
        )
        .ok_or_else(|| "video format missing fields".to_string())
    };
    let plain = PlayerExtras {
        visitor_data: visitor,
        ..Default::default()
    };
    let mut error = match anonymous(plain).await {
        Ok(stream) => return Ok(stream),
        Err(error) => error,
    };
    if !innertube::is_google_block(&error)
        && let Ok(pot) = botguard::mint_content_pot(video_id).await
    {
        let extras = PlayerExtras {
            content_pot: Some(&pot),
            visitor_data: visitor,
            signature_timestamp: None,
        };
        match anonymous(extras).await {
            Ok(stream) => return Ok(stream),
            Err(with_pot) => error = format!("{error}; with pot: {with_pot}"),
        }
    }
    if cookies.is_some() {
        let player = decipher::player_js(video_id).await?;
        let extras = PlayerExtras {
            signature_timestamp: Some(player.1),
            visitor_data: visitor_data(cookies).await.ok(),
            ..Default::default()
        };
        let json = innertube::player(WEB_REMIX, video_id, cookies, extras).await?;
        if let Some(format) = pick_video_format(&json, false) {
            let url = decipher::deciphered_url(&player.0, format).await?;
            if let Some(stream) = video_stream_from(format, url, WEB_REMIX) {
                return Ok(stream);
            }
        }
        error = format!("{error}; signed-in: WEB_REMIX returned no video format");
    }
    Err(error)
}

/// The tallest H.264 MP4 picture under the cap. H.264 is the one codec every
/// desktop webview decodes, which VP9 and AV1 are not.
fn pick_video_format(json: &Value, plain_only: bool) -> Option<&Value> {
    json.pointer("/streamingData/adaptiveFormats")?
        .as_array()?
        .iter()
        .filter(|f| {
            let mime = f["mimeType"].as_str().unwrap_or_default();
            mime.starts_with("video/mp4") && mime.contains("avc1")
        })
        .filter(|f| f["height"].as_u64().is_some_and(|h| h <= MAX_VIDEO_HEIGHT))
        .filter(|f| !plain_only || f["url"].is_string())
        .max_by_key(|f| (f["height"].as_u64(), f["bitrate"].as_u64()))
}

fn video_stream_from(format: &Value, url: String, client: YouTubeClient) -> Option<YtVideoStream> {
    let mime = format["mimeType"].as_str()?;
    if url.is_empty() {
        return None;
    }
    tracing::info!(
        itag = format["itag"].as_u64().unwrap_or(0),
        height = format["height"].as_u64().unwrap_or(0),
        client = client.client_name,
        "video stream resolved"
    );
    Some(YtVideoStream {
        url,
        mime: mime.split(';').next().unwrap_or(mime).trim().to_string(),
        user_agent: client.user_agent.to_string(),
        content_length: format["contentLength"]
            .as_str()
            .and_then(|s| s.parse().ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn a_brand_account_never_takes_the_google_accounts_tracking() {
        let cookies = "SAPISID=abc";
        let own = tracking_key(cookies, "v").unwrap();
        let brand = innertube::as_account(Some("42".into()), async {
            tracking_key(cookies, "v").unwrap()
        })
        .await;
        assert_ne!(own, brand);
    }

    /// The anonymous client ends on LOGIN_REQUIRED for anything YouTube gates,
    /// so that line alone told a signed-in user to sign in. The reason their
    /// own path failed has to travel with it.
    #[test]
    fn the_failure_names_the_signed_in_reason_not_just_the_last_client() {
        let message = all_paths_failed(
            Some("WEB_REMIX playability UNPLAYABLE: try again later"),
            "VISIONOS playability LOGIN_REQUIRED: Sign in to confirm you're not a bot",
            "PO mint: minter unavailable",
        )
        .message;

        assert!(
            message.contains("signed-in: WEB_REMIX playability UNPLAYABLE: try again later"),
            "{message}"
        );
        assert!(message.contains("anonymous: VISIONOS"), "{message}");
        assert!(message.contains("anonymous+pot: PO mint"), "{message}");
    }

    #[test]
    fn a_skipped_signed_in_path_says_so_rather_than_looking_like_a_success() {
        let message =
            all_paths_failed(None, "VISIONOS: nope", "PO mint: minter unavailable").message;
        assert!(message.contains("signed-in: not attempted"), "{message}");
    }

    fn refusal(decipher: Option<&str>, anonymous: &str) -> Refusal {
        all_paths_failed(decipher, anonymous, "PO mint: minter unavailable").refusal
    }

    /// A bot check is about the network, so it wins over everything a
    /// client said next to it, including the signed-in LOGIN_REQUIRED it
    /// arrives as.
    #[test]
    fn a_bot_check_anywhere_is_reported_as_one() {
        assert_eq!(
            refusal(
                Some("WEB_REMIX playability LOGIN_REQUIRED: Sign in to confirm you're not a bot"),
                "VISIONOS playability LOGIN_REQUIRED: Sign in to confirm you're not a bot",
            ),
            Refusal::Challenged
        );
        assert_eq!(
            refusal(None, &format!("{}: HTTP 403", innertube::GOOGLE_BLOCK)),
            Refusal::Challenged
        );
    }

    /// The anonymous client says LOGIN_REQUIRED for anything gated, so only
    /// the signed-in client saying it means the session is gone.
    #[test]
    fn only_a_signed_in_login_required_blames_the_session() {
        assert_eq!(
            refusal(
                Some("WEB_REMIX playability LOGIN_REQUIRED: Please sign in"),
                "VISIONOS playability LOGIN_REQUIRED: Please sign in",
            ),
            Refusal::SessionExpired
        );
        assert_eq!(
            refusal(None, "VISIONOS playability LOGIN_REQUIRED: Please sign in"),
            Refusal::ServiceRefused
        );
    }

    #[test]
    fn an_item_youtube_calls_unavailable_is_reported_as_such() {
        assert_eq!(
            refusal(None, "VISIONOS playability ERROR: Video unavailable"),
            Refusal::Unavailable
        );
        assert_eq!(
            refusal(
                Some("WEB_REMIX playability LOGIN_REQUIRED: This video is private"),
                "VISIONOS playability LOGIN_REQUIRED: This video is private",
            ),
            Refusal::Unavailable
        );
    }

    #[test]
    fn no_answer_from_any_client_is_a_network_failure() {
        assert_eq!(
            refusal(
                Some("watch page fetch: error sending request"),
                "VISIONOS: player HTTP: error sending request",
            ),
            Refusal::Network
        );
        assert_eq!(
            refusal(
                Some("WEB_REMIX playability UNPLAYABLE: try again later"),
                "VISIONOS: player HTTP: error sending request",
            ),
            Refusal::ServiceRefused
        );
    }

    /// The anonymous path decides whether to mint a content token by asking
    /// `is_google_block` about the error it just got, and that answer is a
    /// `starts_with` on the transport's own prefix. A client label in front of
    /// it made the block unrecognisable and bought a pointless mint.
    #[test]
    fn the_abuse_page_stays_recognisable_through_the_client_label() {
        let block = labelled(format!("{}: HTTP 403", innertube::GOOGLE_BLOCK));
        assert!(innertube::is_google_block(&block), "{block}");

        let other = labelled("returned no plain audio format".to_string());
        assert!(!innertube::is_google_block(&other), "{other}");
        assert_eq!(other, "VISIONOS: returned no plain audio format");
    }

    async fn googlevideo() -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..n]);
                let status = if request.starts_with("GET /gated") {
                    "403 Forbidden"
                } else {
                    "206 Partial Content"
                };
                let _ = socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await;
            }
        });
        addr
    }

    fn stream_at(addr: std::net::SocketAddr, path: &str) -> YtStreamInfo {
        YtStreamInfo {
            url: format!("http://{addr}/{path}"),
            format: AudioFormat::Webm,
            user_agent: VISIONOS.user_agent.to_string(),
            content_length: Some(4096),
            duration_secs: Some(212),
            bitrate: Some(136544),
            itag: Some(251),
            range_safe: true,
            loudness_db: None,
        }
    }

    #[tokio::test]
    async fn a_gated_url_is_replaced_by_a_working_one() {
        let addr = googlevideo().await;
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let info = recheck_gated(
            stream_at(addr, "gated"),
            || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { Ok(stream_at(addr, "good")) }
            },
            "VISIONOS",
        )
        .await
        .unwrap();
        assert!(info.url.ends_with("/good"), "{}", info.url);
        assert!(info.range_safe);
        assert_eq!(calls.into_inner(), 1);
    }

    #[tokio::test]
    async fn a_url_gated_on_every_resolve_is_refused() {
        let addr = googlevideo().await;
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let result = recheck_gated(
            stream_at(addr, "gated"),
            || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { Ok(stream_at(addr, "gated")) }
            },
            "VISIONOS",
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.into_inner(), GATED_RETRIES);
    }

    #[tokio::test]
    async fn a_failed_resolve_of_a_gated_url_reports_its_error() {
        let addr = googlevideo().await;
        let result = recheck_gated(
            stream_at(addr, "gated"),
            || async { Err("VISIONOS playability ERROR: nope".to_string()) },
            "VISIONOS",
        )
        .await;
        assert_eq!(result.unwrap_err(), "VISIONOS playability ERROR: nope");
    }

    #[tokio::test]
    async fn a_working_url_is_not_resolved_again() {
        let addr = googlevideo().await;
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let info = recheck_gated(
            stream_at(addr, "good"),
            || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { Ok(stream_at(addr, "gated")) }
            },
            "VISIONOS",
        )
        .await
        .unwrap();
        assert!(info.url.ends_with("/good"), "{}", info.url);
        assert!(info.range_safe);
        assert_eq!(calls.into_inner(), 0);
    }

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
        let info = pick_plain_format(&json, WEB_REMIX, StreamQuality::High)
            .expect("should pick a plain format");
        assert_eq!(info.itag, Some(251));
        assert_eq!(info.bitrate, Some(136544));
        assert_eq!(info.duration_secs, Some(212));
    }

    #[test]
    fn the_video_is_the_tallest_h264_under_the_cap() {
        let json = json!({ "streamingData": { "adaptiveFormats": [
            { "itag": 137, "mimeType": "video/mp4; codecs=\"avc1.640028\"", "height": 1080,
              "bitrate": 4000000, "url": "https://x/137", "contentLength": "80911999" },
            { "itag": 401, "mimeType": "video/mp4; codecs=\"av01.0.12M.08\"", "height": 2160,
              "bitrate": 9000000, "url": "https://x/401" },
            { "itag": 248, "mimeType": "video/webm; codecs=\"vp9\"", "height": 1080,
              "bitrate": 2000000, "url": "https://x/248" },
            { "itag": 299, "mimeType": "video/mp4; codecs=\"avc1.64002a\"", "height": 1440,
              "bitrate": 6000000, "url": "https://x/299" },
            { "itag": 136, "mimeType": "video/mp4; codecs=\"avc1.4D401F\"", "height": 720,
              "bitrate": 1500000, "signatureCipher": "s=abc" },
            { "itag": 251, "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 136544,
              "url": "https://x/251" }
        ]}});
        let format = pick_video_format(&json, true).expect("a plain H.264 format");
        assert_eq!(format["itag"], 137);
        let stream = video_stream_from(format, "https://x/137".into(), VISIONOS).unwrap();
        assert_eq!(stream.mime, "video/mp4");
        assert_eq!(stream.content_length, Some(80_911_999));

        let mut ciphered = json.clone();
        ciphered["streamingData"]["adaptiveFormats"][0]["url"] = Value::Null;
        assert!(pick_video_format(&ciphered, true).is_none());
        assert_eq!(pick_video_format(&ciphered, false).unwrap()["itag"], 137);
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

    #[test]
    fn a_recorded_player_response_carries_its_loudness() {
        let json: Value =
            serde_json::from_str(include_str!("testdata/player_web_remix.json")).unwrap();
        let formats = json
            .pointer("/streamingData/adaptiveFormats")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten();
        let fmt = choose_format(formats, StreamQuality::High, false).expect("an audio format");
        let info = stream_info_from(&json, fmt, "https://x/y".into(), WEB_REMIX)
            .expect("should build stream info");
        let loudness = info.loudness_db.expect("loudnessDb");
        assert!((loudness - -6.11).abs() < 1e-3, "{loudness}");
    }

    #[test]
    fn visionos_loudness_is_rebased_onto_the_web_target() {
        let json = json!({ "playerConfig": { "audioConfig": {
            "perceptualLoudnessDb": -5.24,
            "trackAbsoluteLoudnessLkfs": -5.24,
            "loudnessTargetLkfs": -14
        }}});
        let loudness = loudness_db(&json).expect("loudness");
        assert!((loudness - 1.76).abs() < 1e-3, "{loudness}");

        let only_relative = json!({ "playerConfig": { "audioConfig": { "loudnessDb": 2.5 }}});
        assert_eq!(loudness_db(&only_relative), Some(2.5));
    }

    #[test]
    fn a_response_without_loudness_carries_none() {
        let json = json!({ "videoDetails": { "lengthSeconds": "212" } });
        let fmt = json!({ "itag": 251, "mimeType": "audio/webm; codecs=\"opus\"" });
        let info = stream_info_from(&json, &fmt, "https://x/y".into(), WEB_REMIX).unwrap();
        assert_eq!(info.loudness_db, None);
    }

    /// Skrillex, "Bangarang", about -5 LKFS.
    #[tokio::test]
    #[ignore = "hits live YouTube"]
    async fn resolve_reports_loudness() {
        let info = resolve("YJVmu6yttiw", None, StreamQuality::High)
            .await
            .expect("resolve should succeed");
        let loudness = info.loudness_db.expect("loudnessDb");
        let gain = config::loudness_gain(loudness);
        tracing::info!(
            loudness,
            gain,
            gain_db = 20.0 * gain.log10(),
            "resolved loudness"
        );
        assert!(gain < 1.0, "a track this loud is turned down");
    }

    fn fixture() -> Value {
        serde_json::from_str(include_str!("testdata/player_formats.json")).expect("fixture parses")
    }

    fn chosen(json: &Value, quality: StreamQuality, plain: bool) -> Option<u32> {
        let formats = json
            .pointer("/streamingData/adaptiveFormats")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter(|f| !plain || f.get("url").is_some());
        choose_format(formats, quality, plain).and_then(itag_of)
    }

    /// Normal stays on 251: its peak bitrate is over the cap, but the cap is on
    /// the average.
    #[test]
    fn the_signed_in_path_picks_by_quality() {
        let json = fixture();
        assert!(offers_premium(&json));
        assert_eq!(chosen(&json, StreamQuality::High, false), Some(774));
        assert_eq!(chosen(&json, StreamQuality::Normal, false), Some(251));
        assert_eq!(chosen(&json, StreamQuality::Low, false), Some(249));
    }

    /// The anonymous path only sees plain URLs, so Premium never reaches it.
    #[test]
    fn the_anonymous_path_picks_by_quality() {
        let json = fixture();
        for (quality, itag) in [
            (StreamQuality::High, 251),
            (StreamQuality::Normal, 251),
            (StreamQuality::Low, 249),
        ] {
            assert_eq!(chosen(&json, quality, true), Some(itag), "{quality:?}");
            let info = pick_plain_format(&json, VISIONOS, quality).expect("a plain format");
            assert_eq!(info.itag, Some(itag), "{quality:?}");
        }
    }

    #[test]
    fn quality_falls_back_to_aac_and_then_to_the_smallest_format() {
        let mut json = fixture();
        let formats = json
            .pointer_mut("/streamingData/adaptiveFormats")
            .and_then(|v| v.as_array_mut())
            .expect("formats");
        formats.retain(|f| !f["mimeType"].as_str().unwrap_or("").contains("webm"));
        assert!(offers_premium(&json), "141 is still offered");
        assert_eq!(chosen(&json, StreamQuality::High, false), Some(141));
        assert_eq!(chosen(&json, StreamQuality::Normal, false), Some(140));
        assert_eq!(chosen(&json, StreamQuality::Low, false), Some(139));

        let only_large = serde_json::json!([
            { "itag": 140, "mimeType": "audio/mp4", "averageBitrate": 129478 },
            { "itag": 251, "mimeType": "audio/webm", "averageBitrate": 131468 },
        ]);
        let pick = choose_format(only_large.as_array().unwrap(), StreamQuality::Low, true);
        assert_eq!(pick.and_then(itag_of), Some(140));
    }

    /// End-to-end: resolve a public track (decipher via the SubprocessEngine)
    /// and assert the resolved stream carries a real bitrate + itag — the same
    /// `YtStreamInfo` the player controller stamps onto the bottom bar.
    #[tokio::test]
    #[ignore = "hits live YouTube + needs a system JS runtime"]
    async fn resolve_populates_bitrate_itag_duration() {
        let info = resolve("dQw4w9WgXcQ", None, StreamQuality::High)
            .await
            .expect("resolve should succeed");
        tracing::debug!(
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
