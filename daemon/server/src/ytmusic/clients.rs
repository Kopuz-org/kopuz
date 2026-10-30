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
//! from yt-dlp's defaults within a day. WEB_REMIX's version is read off the live site
//! at runtime ([`refresh_live_version`]); the baked one is only the fallback.

use std::sync::Mutex;

#[derive(Clone, Copy, Debug)]
pub struct YouTubeClient {
    pub client_name: &'static str,
    /// The version this build shipped with. Send [`YouTubeClient::version`] instead.
    pub baked_version: &'static str,
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
    baked_version: "1.20260707.12.00",
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

/// The YouTube Music Android app. Only used to read a song's lyrics tab,
/// which answers it with timed lines where the web client gets plain text.
pub const ANDROID_MUSIC: YouTubeClient = YouTubeClient {
    client_name: "ANDROID_MUSIC",
    baked_version: "7.27.52",
    client_id: "21",
    user_agent: "com.google.android.apps.youtube.music/7.27.52 (Linux; U; Android 14) gzip",
    os_name: "Android",
    os_version: "14",
    device_make: "",
    device_model: "",
    android_sdk_version: Some(34),
    login_supported: false,
    use_signature_timestamp: false,
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
    baked_version: "1.02",
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

/// The WEB_REMIX version music.youtube.com last served, once read.
static LIVE_WEB_REMIX_VERSION: Mutex<Option<&'static str>> = Mutex::new(None);

impl YouTubeClient {
    /// The version to send. For WEB_REMIX that is the one music.youtube.com
    /// serves today when it has been read: YouTube moves the web client every
    /// week or two and starts answering a stale one with degraded responses
    /// (a library page with no shelves) long before it refuses it outright.
    pub fn version(&self) -> &'static str {
        if self.client_name == WEB_REMIX.client_name
            && let Ok(live) = LIVE_WEB_REMIX_VERSION.lock()
            && let Some(live) = *live
        {
            return live;
        }
        self.baked_version
    }
}

/// Read the WEB_REMIX version off music.youtube.com and send it from now on.
/// A failure keeps whatever was sent before.
pub async fn refresh_live_version() -> Result<&'static str, String> {
    let page = super::innertube::http_client()
        .get(format!("{ORIGIN_YOUTUBE_MUSIC}/"))
        .header("User-Agent", USER_AGENT_WEB)
        // Outside the EU this is ignored; inside it, it skips the consent
        // interstitial, which carries no ytcfg.
        .header("Cookie", "SOCS=CAI")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| format!("music.youtube.com: {error}"))?
        .text()
        .await
        .map_err(|error| format!("music.youtube.com body: {error}"))?;
    let found =
        web_remix_version_in(&page).ok_or("music.youtube.com carried no WEB_REMIX version")?;
    let mut live = LIVE_WEB_REMIX_VERSION
        .lock()
        .map_err(|_| "live version lock poisoned".to_string())?;
    if let Some(current) = *live
        && current == found
    {
        return Ok(current);
    }
    let leaked: &'static str = Box::leak(found.to_owned().into_boxed_str());
    if leaked != WEB_REMIX.baked_version {
        tracing::info!(
            baked = WEB_REMIX.baked_version,
            live = leaked,
            "using the WEB_REMIX version music.youtube.com serves"
        );
    }
    *live = Some(leaked);
    Ok(leaked)
}

/// Read the live version unless it already has been. For the paths that run
/// before any upkeep tick could have, such as a first sign-in.
pub async fn ensure_live_version() {
    if LIVE_WEB_REMIX_VERSION
        .lock()
        .is_ok_and(|live| live.is_some())
    {
        return;
    }
    if let Err(error) = refresh_live_version().await {
        tracing::warn!(%error, "reading the live WEB_REMIX version failed; sending the baked one");
    }
}

/// `"INNERTUBE_CLIENT_VERSION":"1.20261013.15.00"` from the page's ytcfg,
/// accepted only in the `1.YYYYMMDD.NN.NN` shape a real one has.
fn web_remix_version_in(page: &str) -> Option<&str> {
    const KEY: &str = "\"INNERTUBE_CLIENT_VERSION\":\"";
    let start = page.find(KEY)? + KEY.len();
    let version = &page[start..start + page[start..].find('"')?];
    let parts: Vec<&str> = version.split('.').collect();
    let well_formed = matches!(parts.as_slice(), ["1", date, a, b]
        if date.len() == 8
            && [*date, *a, *b].iter().all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit())));
    well_formed.then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_read_from_ytcfg() {
        let page = r#"ytcfg.set({"INNERTUBE_CLIENT_NAME":"WEB_REMIX","INNERTUBE_CLIENT_VERSION":"1.20261013.15.00","GL":"US"});"#;
        assert_eq!(web_remix_version_in(page), Some("1.20261013.15.00"));
    }

    #[test]
    fn a_page_without_a_real_version_yields_none() {
        assert_eq!(web_remix_version_in("<html>consent</html>"), None);
        assert_eq!(
            web_remix_version_in(r#""INNERTUBE_CLIENT_VERSION":"2.0""#),
            None
        );
        assert_eq!(
            web_remix_version_in(r#""INNERTUBE_CLIENT_VERSION":"1.2026101x.15.00""#),
            None
        );
    }

    #[test]
    fn only_web_remix_takes_the_live_version() {
        assert_eq!(VISIONOS.version(), VISIONOS.baked_version);
        assert_eq!(ANDROID_MUSIC.version(), ANDROID_MUSIC.baked_version);
    }
}
