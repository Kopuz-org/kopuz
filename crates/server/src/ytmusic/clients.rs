//! InnerTube client identities. Each entry is one of YouTube's published
//! client tuples (clientName, clientVersion, etc.); the player fallback
//! chain iterates these looking for one whose response carries plain
//! (non-signature-cipher) stream URLs.
//!
//! The constants are sourced from common public reverse-engineering
//! references (NewPipe, yt-dlp, etc.) — they're factual identifiers
//! YouTube's own apps send.

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
    /// True when the client expects `playbackContext.contentPlaybackContext.signatureTimestamp`.
    /// We can't actually compute this (needs JS deobf) so any client with
    /// `use_signature_timestamp = true` will return signed URLs we can't decode.
    pub use_signature_timestamp: bool,
    /// `--app=URL` embedded variants need this.
    pub is_embedded: bool,
}

pub const ORIGIN_YOUTUBE_MUSIC: &str = "https://music.youtube.com";

const USER_AGENT_WEB: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";

pub const WEB_REMIX: YouTubeClient = YouTubeClient {
    client_name: "WEB_REMIX",
    client_version: "1.20260213.01.00",
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

pub const TVHTML5_SIMPLY_EMBEDDED_PLAYER: YouTubeClient = YouTubeClient {
    client_name: "TVHTML5_SIMPLY_EMBEDDED_PLAYER",
    client_version: "2.0",
    client_id: "85",
    user_agent: "Mozilla/5.0 (PlayStation; PlayStation 4/12.02) AppleWebKit/605.1.15 \
                 (KHTML, like Gecko) Version/15.4 Safari/605.1.15",
    os_name: "",
    os_version: "",
    device_make: "",
    device_model: "",
    android_sdk_version: None,
    login_supported: true,
    use_signature_timestamp: true,
    is_embedded: true,
};

pub const ANDROID_VR_1_43_32: YouTubeClient = YouTubeClient {
    client_name: "ANDROID_VR",
    client_version: "1.43.32",
    client_id: "28",
    user_agent: "com.google.android.apps.youtube.vr.oculus/1.43.32 \
                 (Linux; U; Android 12; en_US; Quest 3; Build/SQ3A.220605.009.A1; \
                 Cronet/107.0.5284.2)",
    os_name: "Android",
    os_version: "12",
    device_make: "Oculus",
    device_model: "Quest 3",
    android_sdk_version: Some(32),
    login_supported: false,
    use_signature_timestamp: false,
    is_embedded: false,
};

pub const ANDROID_VR_1_61_48: YouTubeClient = YouTubeClient {
    client_name: "ANDROID_VR",
    client_version: "1.61.48",
    client_id: "28",
    user_agent: "com.google.android.apps.youtube.vr.oculus/1.61.48 \
                 (Linux; U; Android 12; en_US; Quest 3; Build/SQ3A.220605.009.A1; \
                 Cronet/132.0.6808.3)",
    os_name: "Android",
    os_version: "12",
    device_make: "Oculus",
    device_model: "Quest 3",
    android_sdk_version: Some(32),
    login_supported: false,
    use_signature_timestamp: false,
    is_embedded: false,
};

/// Default client for browse/search and the main-path `/player` attempt
/// inside the fallback chain — WEB_REMIX with the user's auth cookies.
pub const MAIN_CLIENT: YouTubeClient = WEB_REMIX;

/// Player fallback chain. Tried in order if the primary ANDROID_VR + pot
/// path fails. Kept narrow: just two ANDROID_VR versions. IOS / IPADOS
/// were dropped — their stream URLs are rate-limited to the first ~1 MiB
/// from byte 0, so even when /player returns OK, chunked Range fetches
/// for the rest of the track 403 mid-playback. Worse than failing fast.
pub const STREAM_FALLBACK_CLIENTS: &[YouTubeClient] = &[
    ANDROID_VR_1_43_32,
    ANDROID_VR_1_61_48,
];
