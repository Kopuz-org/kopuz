//! InnerTube client identities. Each entry is one of YouTube's published
//! client tuples (clientName, clientVersion, etc.); the player fallback
//! chain iterates these looking for one whose response carries plain
//! (non-signature-cipher) stream URLs.
//!
//! The constants are copied from yt-dlp's `INNERTUBE_CLIENTS` table in
//! `yt_dlp/extractor/youtube/_base.py`, which is the best-maintained record
//! of which clients YouTube still serves and what each one needs. Last
//! matched against yt-dlp 2026.08.19. When YouTube playback breaks, diff this
//! file against that table first: a client YouTube retires is usually gone
//! from yt-dlp's defaults within a day.

#[derive(Clone, Copy, Debug)]
pub struct YouTubeClient {
    pub client_name: &'static str,
    pub client_version: &'static str,
    /// Numeric client id sent as `X-YouTube-Client-Name`.
    pub client_id: &'static str,
    pub user_agent: &'static str,
    pub os_name: &'static str,
    pub os_version: &'static str,
    pub device_make: &'static str,
    pub device_model: &'static str,
    pub android_sdk_version: Option<u32>,
    /// Sends `Cookie:` + `SAPISIDHASH` when true.
    pub login_supported: bool,
    /// True when the client expects `playbackContext.contentPlaybackContext.signatureTimestamp`
    /// and answers with signature-ciphered URLs the decipher engine has to solve.
    pub use_signature_timestamp: bool,
    /// `--app=URL` embedded variants need this.
    pub is_embedded: bool,
}

pub const ORIGIN_YOUTUBE_MUSIC: &str = "https://music.youtube.com";

const USER_AGENT_WEB: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";

/// The signed-in client. Its player response carries signature-ciphered
/// URLs, which the decipher engine solves; a Premium account gets its 774
/// Opus here with no proof-of-origin token at all.
pub const WEB_REMIX: YouTubeClient = YouTubeClient {
    client_name: "WEB_REMIX",
    client_version: "1.20260707.12.00",
    client_id: "67",
    user_agent: USER_AGENT_WEB,
    os_name: "",
    os_version: "",
    device_make: "",
    device_model: "",
    android_sdk_version: None,
    login_supported: true,
    use_signature_timestamp: true,
    is_embedded: false,
};

/// The anonymous client: plain URLs, no JS player, and -- as yt-dlp
/// classifies it -- no proof-of-origin token required or recommended.
///
/// It replaced ANDROID_VR, which YouTube shut on 2026-08-17: every format,
/// with any version and even a fresh content token, answers 403 or
/// LOGIN_REQUIRED since. yt-dlp dropped it from its defaults the next day and
/// shipped this one, which it describes as the same client in all but name.
/// Made-for-kids videos are not served to it.
pub const VISIONOS: YouTubeClient = YouTubeClient {
    client_name: "VISIONOS",
    client_version: "1.02",
    client_id: "101",
    user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
    os_name: "visionOS",
    os_version: "26.5.23O471",
    device_make: "Apple",
    device_model: "RealityDevice17,1",
    android_sdk_version: None,
    login_supported: false,
    use_signature_timestamp: false,
    is_embedded: false,
};
