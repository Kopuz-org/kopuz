use reader::models::Track;
use serde_json::Value;

pub mod botguard;
pub mod clients;
pub mod cookies;
pub mod discover;
pub mod innertube;
pub mod isolated_profile;
pub mod mix;
pub mod mutations;
pub mod player;
pub mod playlists;
pub mod search;
pub mod verify_session_keepalive;
pub mod ytdlp_fallback;

pub use player::YtStreamInfo;

pub const SOURCE_PREFIX: &str = "ytmusic";

/// Stable per-Google-account id derived from the SAPISID cookie. Used
/// as the server-identity cache-bust key so switching YT accounts
/// invalidates synced library/favorites state. Never logged.
pub fn derive_user_id(cookies: &str) -> Option<String> {
    use sha1::{Digest, Sha1};
    let sapisid = cookies.split(';').find_map(|p| {
        let (k, v) = p.trim().split_once('=')?;
        (k == "SAPISID" || k == "__Secure-3PAPISID").then(|| v.to_string())
    })?;
    let mut h = Sha1::new();
    h.update(sapisid.as_bytes());
    Some(format!("yt-{}", hex::encode(&h.finalize()[..6])))
}

pub struct YouTubeMusicClient {
    cookies: Option<String>,
}

impl YouTubeMusicClient {
    pub fn new() -> Self {
        Self { cookies: None }
    }

    pub fn with_cookies(cookies: String) -> Self {
        Self {
            cookies: Some(cookies),
        }
    }

    pub async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, String> {
        search::music_search_tracks(query, self.cookies.as_deref()).await
    }

    /// List the user's saved playlists (everything under Library →
    /// Playlists, minus the Liked Music auto-playlist). Returns summary
    /// rows; call [`get_playlist_entries`] for the tracks of any given
    /// playlist.
    pub async fn list_playlists(
        &self,
    ) -> Result<Vec<playlists::YtPlaylistSummary>, String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        playlists::list_playlists(cookies).await
    }

    pub async fn get_playlist_entries(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<Track>, String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        playlists::get_playlist_entries(playlist_id, cookies).await
    }

    pub async fn like_video(&self, video_id: &str) -> Result<(), String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        mutations::like_video(video_id, cookies).await
    }

    pub async fn unlike_video(&self, video_id: &str) -> Result<(), String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        mutations::unlike_video(video_id, cookies).await
    }

    pub async fn add_to_playlist(
        &self,
        playlist_id: &str,
        video_id: &str,
    ) -> Result<(), String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        mutations::add_to_playlist(playlist_id, video_id, cookies).await
    }

    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        video_id: &str,
    ) -> Result<(), String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        mutations::remove_from_playlist(playlist_id, video_id, cookies).await
    }

    pub async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[&str],
    ) -> Result<String, String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        mutations::create_playlist(title, description, video_ids, cookies).await
    }

    /// Stream the user's full Liked Music playlist page by page. The
    /// callback fires once per ~100-track batch as soon as it arrives,
    /// so the UI can populate incrementally instead of waiting for the
    /// whole library to download. Walks `continuationItemRenderer`
    /// tokens until exhausted.
    pub async fn stream_liked_songs<F>(&self, mut on_page: F) -> Result<(), String>
    where
        F: FnMut(Vec<Track>),
    {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        let resp: Value = innertube::browse("VLLM", cookies).await?;
        if !has_playlist_shelf(&resp) {
            return Err("Sign-in prompt returned — cookies expired".to_string());
        }
        // YT's continuation pagination commonly repeats one or more tracks at page
        // boundaries; dedup against a video-id set across the entire stream so the
        // callback always sees unique tracks.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let dedup = |page: Vec<Track>, seen: &mut std::collections::HashSet<String>| -> Vec<Track> {
            page.into_iter()
                .filter(|t| {
                    let id = t
                        .path
                        .to_string_lossy()
                        .split(':')
                        .nth(1)
                        .unwrap_or("")
                        .to_string();
                    !id.is_empty() && seen.insert(id)
                })
                .collect()
        };

        let (page1, mut next) = search::walk_playlist_shelf(&resp);
        let page1 = dedup(page1, &mut seen);
        if !page1.is_empty() {
            on_page(page1);
        }
        while let Some(token) = next.take() {
            let page = innertube::browse_continuation(&token, cookies).await?;
            let (more, next_token) = search::walk_playlist_continuation(&page);
            let more = dedup(more, &mut seen);
            // An empty page after dedup means YT either gave us only
            // duplicates of already-seen tracks or no new content. Even
            // if it returned a continuation token, looping again would
            // hammer the same endpoint without progress, so stop.
            if more.is_empty() {
                break;
            }
            on_page(more);
            next = next_token;
        }
        Ok(())
    }

    /// Buffered convenience around [`stream_liked_songs`] — collects all
    /// pages before returning. Use only when the caller doesn't care
    /// about incremental updates.
    pub async fn get_liked_songs(&self) -> Result<Vec<Track>, String> {
        let mut all = Vec::new();
        self.stream_liked_songs(|page| all.extend(page)).await?;
        Ok(all)
    }

    /// Resolves a playable stream URL: mints a content-bound PO token via
    /// `rustypipe-botguard`, then asks `/player` (ANDROID_VR client) for a
    /// plain audio URL. ~400 ms warm, ~1 s cold (snapshot regeneration).
    pub async fn get_stream(&self, video_id: &str) -> Result<YtStreamInfo, String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        player::resolve(video_id, cookies).await
    }

    pub async fn start_mix(&self, seed_video_id: &str) -> Result<Vec<Track>, String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        mix::start_mix(seed_video_id, cookies).await
    }

    pub async fn discover_home(&self) -> Result<discover::DiscoverHome, String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        discover::fetch_home(cookies).await
    }

    pub async fn discover_continuation(
        &self,
        token: &str,
    ) -> Result<discover::DiscoverHome, String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        discover::fetch_continuation(token, cookies).await
    }

    pub async fn fetch_album_tracks(&self, browse_id: &str) -> Result<Vec<Track>, String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        discover::fetch_album_tracks(browse_id, cookies).await
    }

    /// Confirms the `rustypipe-botguard` binary is reachable. Call this
    /// at server-selection time so users get the install hint up front
    /// instead of seeing a silent 403 partway into their first track.
    pub async fn check_botguard_available(&self) -> Result<(), String> {
        botguard::check_available().await
    }

    /// Confirms the cookie session is actually signed in — InnerTube
    /// `/browse?browseId=VLLM` (Liked Music) is the canonical probe: it
    /// returns a `signInEndpoint`-bearing message renderer for anonymous
    /// callers and real playlist content for signed-in ones.
    pub async fn validate_cookies(&self) -> Result<(), String> {
        let cookies = self
            .cookies
            .as_deref()
            .ok_or("YouTube Music not signed in")?;
        let json: Value = innertube::browse("VLLM", cookies).await?;
        if has_playlist_shelf(&json) {
            Ok(())
        } else {
            Err("YouTube returned no playlist shelf — cookies expired or browser signed out".into())
        }
    }
}

fn has_playlist_shelf(json: &Value) -> bool {
    json.pointer(
        "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents",
    )
    .or_else(|| {
        json.pointer(
            "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
        )
    })
    .and_then(|v| v.as_array())
    .map(|arr| arr.iter().any(|shelf| shelf.get("musicPlaylistShelfRenderer").is_some()))
    .unwrap_or(false)
}

impl Default for YouTubeMusicClient {
    fn default() -> Self {
        Self::new()
    }
}
