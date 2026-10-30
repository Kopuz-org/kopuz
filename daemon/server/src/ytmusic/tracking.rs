//! Playback pings that put a play into the account's History. Parameters follow
//! yt-dlp's `_mark_watched`; the cadence follows the player response's
//! `videostatsScheduledFlushWalltimeSeconds` (10, 20, 30 s) and then
//! `videostatsDefaultFlushIntervalSeconds` (40 s).
//!
//! Only sent as the signed-in identity: the anonymous visitor id would tie that
//! identity to the account.

use std::time::Instant;

use serde_json::Value;

use super::clients::{ORIGIN_YOUTUBE_MUSIC, WEB_REMIX};
use super::innertube::{http_client, page_id, sapisid_hash};

const SCHEDULED_FLUSH_MS: [u64; 3] = [10_000, 20_000, 30_000];
const FLUSH_INTERVAL_MS: u64 = 40_000;
/// Position steps larger than this are seeks, not playback.
const MAX_STEP_MS: u64 = 2_000;

/// The `playbackTracking` URLs of one signed-in `player` response.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackTracking {
    pub playback_url: String,
    pub watchtime_url: Option<String>,
}

impl PlaybackTracking {
    pub fn from_player(json: &Value) -> Option<Self> {
        let url = |key: &str| {
            json.pointer(&format!("/playbackTracking/{key}/baseUrl"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        Some(Self {
            playback_url: url("videostatsPlaybackUrl")?,
            watchtime_url: url("videostatsWatchtimeUrl"),
        })
    }
}

/// The tracking URLs arrive after playback starts, so positions are recorded
/// from the first report and a flush that falls due before then is held back.
#[derive(Debug)]
pub struct Watch {
    tracking: Option<PlaybackTracking>,
    cpn: String,
    started: Instant,
    last_ms: u64,
    segment_start_ms: u64,
    segments: Vec<(u64, u64)>,
    watched_ms: u64,
    flushes: usize,
}

impl Watch {
    pub fn new(position_ms: u64) -> Self {
        Self {
            tracking: None,
            cpn: client_playback_nonce(),
            started: Instant::now(),
            last_ms: position_ms,
            segment_start_ms: position_ms,
            segments: Vec::new(),
            watched_ms: 0,
            flushes: 0,
        }
    }

    pub fn cpn(&self) -> &str {
        &self.cpn
    }

    pub fn attach(&mut self, tracking: PlaybackTracking) -> String {
        let url = format!(
            "{}&ver=2&{}&cpn={}&cmt={}&rt={}",
            tracking.playback_url,
            client_params(),
            self.cpn,
            secs(self.last_ms),
            secs(self.started.elapsed().as_millis() as u64),
        );
        self.tracking = Some(tracking);
        url
    }

    pub fn on_position(&mut self, position_ms: u64) -> Option<String> {
        let step = position_ms.wrapping_sub(self.last_ms);
        if position_ms >= self.last_ms && step <= MAX_STEP_MS {
            self.watched_ms += step;
        } else {
            self.close_segment();
            self.segment_start_ms = position_ms;
        }
        self.last_ms = position_ms;
        if self.watched_ms >= self.next_flush_ms() {
            self.flushes += 1;
            return self.flush("playing");
        }
        None
    }

    pub fn finish(mut self) -> Option<String> {
        self.flush("paused")
    }

    fn next_flush_ms(&self) -> u64 {
        match SCHEDULED_FLUSH_MS.get(self.flushes) {
            Some(ms) => *ms,
            None => {
                SCHEDULED_FLUSH_MS[SCHEDULED_FLUSH_MS.len() - 1]
                    + FLUSH_INTERVAL_MS * (self.flushes + 1 - SCHEDULED_FLUSH_MS.len()) as u64
            }
        }
    }

    fn close_segment(&mut self) {
        if self.last_ms > self.segment_start_ms {
            self.segments.push((self.segment_start_ms, self.last_ms));
        }
    }

    fn flush(&mut self, state: &str) -> Option<String> {
        let base = self.tracking.as_ref()?.watchtime_url.clone()?;
        self.close_segment();
        if self.segments.is_empty() {
            return None;
        }
        let list = |pick: fn(&(u64, u64)) -> u64| {
            self.segments
                .iter()
                .map(|s| secs(pick(s)))
                .collect::<Vec<_>>()
                .join(",")
        };
        let url = format!(
            "{base}&ver=2&{}&cpn={}&cmt={}&st={}&et={}&state={state}&rt={}",
            client_params(),
            self.cpn,
            secs(self.last_ms),
            list(|s| s.0),
            list(|s| s.1),
            secs(self.started.elapsed().as_millis() as u64),
        );
        self.segments.clear();
        self.segment_start_ms = self.last_ms;
        Some(url)
    }
}

/// Which client played it. Left out, the play is taken for youtube.com's
/// player and stays out of YouTube Music's History.
fn client_params() -> String {
    format!("c={}&cver={}", WEB_REMIX.client_name, WEB_REMIX.version())
}

fn secs(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Every tracking ping of one play carries the same nonce.
fn client_playback_nonce() -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let bytes: [u8; 16] = rand::random();
    bytes
        .iter()
        .map(|b| ALPHABET[(b & 63) as usize] as char)
        .collect()
}

pub async fn ping(url: &str, cookies: &str, visitor: Option<&str>) -> Result<u16, String> {
    let auth = sapisid_hash(cookies, ORIGIN_YOUTUBE_MUSIC).ok_or("SAPISID missing")?;
    let mut req = http_client()
        .get(url)
        .header("User-Agent", WEB_REMIX.user_agent)
        .header("X-Goog-Api-Format-Version", "1")
        .header("X-YouTube-Client-Name", WEB_REMIX.client_id)
        .header("X-YouTube-Client-Version", WEB_REMIX.version())
        .header("Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("X-Origin", ORIGIN_YOUTUBE_MUSIC)
        .header("Referer", format!("{ORIGIN_YOUTUBE_MUSIC}/"))
        .header("X-Goog-AuthUser", "0")
        .header("Cookie", cookies)
        .header("Authorization", auth);
    if let Some(visitor) = visitor {
        req = req.header("X-Goog-Visitor-Id", visitor);
    }
    if let Some(page_id) = page_id() {
        req = req.header("X-Goog-PageId", page_id);
    }
    let status = req
        .send()
        .await
        .map_err(|e| format!("tracking HTTP: {e}"))?
        .status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(format!("tracking HTTP {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tracking() -> PlaybackTracking {
        PlaybackTracking {
            playback_url: "https://s.youtube.com/api/stats/playback?docid=v&len=200".into(),
            watchtime_url: Some("https://s.youtube.com/api/stats/watchtime?docid=v&len=200".into()),
        }
    }

    fn watch() -> Watch {
        let mut w = Watch::new(0);
        w.attach(tracking());
        w
    }

    fn param<'a>(url: &'a str, key: &str) -> &'a str {
        url.split(['?', '&'])
            .find_map(|p| p.strip_prefix(&format!("{key}=")))
            .unwrap()
    }

    #[test]
    fn flushes_at_10_20_30_then_every_40_seconds() {
        let mut w = watch();
        let mut flushed_at = Vec::new();
        for ms in (250..=120_000).step_by(250) {
            if let Some(url) = w.on_position(ms) {
                flushed_at.push(ms);
                assert_eq!(param(&url, "state"), "playing");
            }
        }
        assert_eq!(flushed_at, [10_000, 20_000, 30_000, 70_000, 110_000]);
    }

    #[test]
    fn a_pause_does_not_count_as_watched() {
        let mut w = watch();
        for ms in (250..=5_000).step_by(250) {
            w.on_position(ms);
        }
        for _ in 0..100 {
            assert!(w.on_position(5_000).is_none());
        }
        assert_eq!(w.watched_ms, 5_000);
    }

    #[test]
    fn seeks_split_the_watched_ranges() {
        let mut w = watch();
        for ms in (250..=4_000).step_by(250) {
            assert!(w.on_position(ms).is_none());
        }
        w.on_position(60_000);
        for ms in (60_250..=63_000).step_by(250) {
            w.on_position(ms);
        }
        let url = w.finish().unwrap();
        assert_eq!(param(&url, "st"), "0.000,60.000");
        assert_eq!(param(&url, "et"), "4.000,63.000");
        assert_eq!(param(&url, "cmt"), "63.000");
        assert_eq!(param(&url, "ver"), "2");
        assert_eq!(param(&url, "cpn").len(), 16);
        assert_eq!(param(&url, "state"), "paused");
        assert_eq!(param(&url, "docid"), "v");
        assert_eq!(param(&url, "c"), "WEB_REMIX");
        assert_eq!(param(&url, "cver"), WEB_REMIX.version());
    }

    #[test]
    fn playback_ping_shares_the_nonce() {
        let mut w = Watch::new(0);
        let start = w.attach(tracking());
        assert!(start.starts_with("https://s.youtube.com/api/stats/playback?docid=v&len=200&"));
        assert_eq!(param(&start, "ver"), "2");
        assert_eq!(param(&start, "cmt"), "0.000");
        assert_eq!(param(&start, "c"), "WEB_REMIX");
        for ms in (250..=10_000).step_by(250) {
            if let Some(url) = w.on_position(ms) {
                assert_eq!(param(&url, "cpn"), param(&start, "cpn"));
            }
        }
    }

    #[test]
    fn ranges_played_before_the_urls_arrive_are_kept() {
        let mut w = Watch::new(0);
        for ms in (250..=12_000).step_by(250) {
            assert!(w.on_position(ms).is_none());
        }
        w.attach(tracking());
        let url = w.finish().unwrap();
        assert_eq!(param(&url, "st"), "0.000");
        assert_eq!(param(&url, "et"), "12.000");
    }

    #[test]
    fn nonce_shape() {
        let cpn = client_playback_nonce();
        assert_eq!(cpn.len(), 16);
        assert!(
            cpn.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
    }

    #[test]
    fn reads_the_tracking_urls_from_a_player_response() {
        let json = json!({ "playbackTracking": {
            "videostatsPlaybackUrl": { "baseUrl": "https://s.youtube.com/api/stats/playback?docid=v" },
            "videostatsWatchtimeUrl": { "baseUrl": "https://s.youtube.com/api/stats/watchtime?docid=v" },
        }});
        let tracking = PlaybackTracking::from_player(&json).unwrap();
        assert_eq!(
            tracking.watchtime_url.as_deref(),
            Some("https://s.youtube.com/api/stats/watchtime?docid=v")
        );
        assert_eq!(PlaybackTracking::from_player(&json!({})), None);
    }
}
