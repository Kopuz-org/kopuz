use reader::models::{Track, TrackId};
use serde_json::Value;

pub mod botguard;
pub mod clients;
pub mod cookies;
pub mod decipher;
pub mod discover;
pub mod innertube;
pub mod isolated_profile;
pub mod mix;
pub mod mutations;
pub mod player;
pub mod playlists;
pub mod search;
pub mod verify_session_keepalive;

pub use player::YtStreamInfo;

pub const SOURCE_PREFIX: &str = "ytmusic";

/// Initialize the V8 platform exactly once, before any isolate is created.
///
/// The decipher and BotGuard runtimes are independent `deno_core` isolates on
/// separate threads. V8's platform is process-global and must be set up before
/// the first isolate; if each runtime lazily initializes it from its own thread,
/// the second one races the first and segfaults. Both call this (before spawning
/// their thread) so init happens once, serialized, and only when a YouTube JS
/// operation actually needs a runtime — a non-YouTube session never touches V8.
#[cfg(not(target_os = "android"))]
pub(crate) fn ensure_v8_platform() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| deno_core::JsRuntime::init_platform(None, false));
}

/// Build the typed id for a YouTube Music track (video id).
pub(crate) fn yt_id(video_id: impl Into<String>) -> TrackId {
    TrackId::Server {
        service: config::MusicService::YtMusic,
        item_id: video_id.into(),
    }
}

/// Surfaced by auth-only operations (like/unlike, add-to-playlist,
/// liked-songs sync) when the YT backend is in anonymous mode.
/// Callers should detect this and either skip the action or hint at
/// signing in.
pub const ANON_AUTH_REQUIRED: &str = "YouTube Music not signed in";

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

/// Resolve a YT stream for diagnostics (the `duration_probe` example) without
/// holding a client — a thin wrapper over the same resolve [`YouTubeMusicClient::get_stream`] uses.
pub async fn probe_stream(video_id: &str, cookies: Option<&str>) -> Result<YtStreamInfo, String> {
    player::resolve(video_id, cookies).await
}

pub(crate) struct YouTubeMusicClient {
    cookies: Option<String>,
}

impl YouTubeMusicClient {
    pub fn new() -> Self {
        Self { cookies: None }
    }

    pub fn with_cookies(cookies: String) -> Self {
        // Normalize the empty string (anonymous-mode marker, stored as
        // access_token: Some("")) to None so every `self.cookies` check
        // — auth-only guards, is_anonymous, the public-surface
        // unwrap_or("") — treats anonymous consistently.
        Self {
            cookies: (!cookies.is_empty()).then_some(cookies),
        }
    }

    pub async fn search_tracks(&self, query: &str) -> Result<Vec<Track>, String> {
        search::music_search_tracks(query, self.cookies.as_deref()).await
    }

    pub async fn resolve_artist_channel_id(&self, query: &str) -> Result<Option<String>, String> {
        search::resolve_artist_channel_id(query, self.cookies.as_deref()).await
    }

    /// Top YT Music artist-search avatar for `name` — the Artists grid uses this
    /// so its photos are real YT artist images.
    pub async fn resolve_artist_image(&self, name: &str) -> Result<Option<String>, String> {
        search::resolve_artist_image(name, self.cookies.as_deref()).await
    }

    /// Resolve a saved album (title + artist) back to its YT album browse id
    /// (`MPRE…`). Library YT albums carry no browse id, so the album page calls
    /// this on demand before [`fetch_album_tracks`](Self::fetch_album_tracks)
    /// to show the album's complete track list.
    pub async fn resolve_album_browse_id(
        &self,
        album: &str,
        artist: &str,
    ) -> Result<Option<String>, String> {
        search::resolve_album_browse_id(album, artist, self.cookies.as_deref()).await
    }

    /// List the user's saved playlists (everything under Library →
    /// Playlists, minus the Liked Music auto-playlist). Returns summary
    /// rows; call [`get_playlist_entries`] for the tracks of any given
    /// playlist.
    /// Library playlists view (FEmusic_liked_playlists). Auth-only —
    /// returns Ok(vec![]) in anonymous mode so the playlists tab
    /// just shows empty rather than erroring.
    pub async fn list_playlists(&self) -> Result<Vec<playlists::YtPlaylistSummary>, String> {
        let Some(cookies) = self.cookies.as_deref() else {
            return Ok(Vec::new());
        };
        playlists::list_playlists(cookies).await
    }

    /// Playlist contents. Public playlists work anonymously; the
    /// user's personal/private ones obviously won't.
    pub async fn get_playlist_entries(&self, playlist_id: &str) -> Result<Vec<Track>, String> {
        playlists::get_playlist_entries(playlist_id, self.cookies.as_deref().unwrap_or("")).await
    }

    pub async fn playlist_page(
        &self,
        playlist_id: &str,
        continuation: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>), String> {
        playlists::playlist_page(
            playlist_id,
            self.cookies.as_deref().unwrap_or(""),
            continuation,
        )
        .await
    }

    // Mutations are inherently auth-only — keep the explicit "not
    // signed in" error so callers (favorite toggle, add-to-playlist
    // modal) can surface a clear "sign in to enable" message.
    pub async fn like_video(&self, video_id: &str) -> Result<(), String> {
        let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
        mutations::like_video(video_id, cookies).await
    }

    pub async fn unlike_video(&self, video_id: &str) -> Result<(), String> {
        let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
        mutations::unlike_video(video_id, cookies).await
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, video_id: &str) -> Result<(), String> {
        let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
        mutations::add_to_playlist(playlist_id, video_id, cookies).await
    }

    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        video_id: &str,
    ) -> Result<(), String> {
        let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
        mutations::remove_from_playlist(playlist_id, video_id, cookies).await
    }

    pub async fn create_playlist(&self, title: &str, video_ids: &[&str]) -> Result<String, String> {
        let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
        mutations::create_playlist(title, video_ids, cookies).await
    }

    /// Stream the user's full Liked Music playlist page by page. The
    /// callback fires once per ~100-track batch as soon as it arrives,
    /// so the UI can populate incrementally instead of waiting for the
    /// whole library to download. Walks `continuationItemRenderer`
    /// tokens until exhausted.
    #[tracing::instrument(name = "yt.liked_stream", skip_all)]
    pub async fn stream_liked_songs<F>(&self, mut on_page: F) -> Result<(), String>
    where
        F: FnMut(Vec<Track>),
    {
        // Liked Music is auth-only — anonymous callers get an empty
        // list rather than an error so favorites views render the
        // standard empty state without surfacing a stack trace.
        let Some(cookies) = self.cookies.as_deref() else {
            let _ = &mut on_page;
            return Ok(());
        };
        let resp: Value = innertube::browse("VLLM", cookies).await?;
        if !has_playlist_shelf(&resp) {
            return Err("Sign-in prompt returned — cookies expired".to_string());
        }
        // YT's continuation pagination commonly repeats one or more tracks at page
        // boundaries; dedup against a video-id set across the entire stream so the
        // callback always sees unique tracks.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let dedup =
            |page: Vec<Track>, seen: &mut std::collections::HashSet<String>| -> Vec<Track> {
                page.into_iter()
                    .filter(|t| {
                        let id = t.id.key().to_string();
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

    /// One page of Liked Music, cursor-driven: pass `None` for the first page,
    /// then the returned token for each next page (until it's `None`). Lets a
    /// caller paginate without an internal callback (cross-page dedup is the
    /// caller's job — YT repeats tracks at page boundaries). Anonymous → empty.
    pub async fn liked_songs_page(
        &self,
        continuation: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>), String> {
        let Some(cookies) = self.cookies.as_deref() else {
            return Ok((Vec::new(), None));
        };
        match continuation {
            None => {
                let resp: Value = innertube::browse("VLLM", cookies).await?;
                if !has_playlist_shelf(&resp) {
                    return Err("Sign-in prompt returned — cookies expired".to_string());
                }
                Ok(search::walk_playlist_shelf(&resp))
            }
            Some(token) => {
                let resp = innertube::browse_continuation(token, cookies).await?;
                Ok(search::walk_playlist_continuation(&resp))
            }
        }
    }

    /// Resolves a playable stream URL via native sig/n deciphering against
    /// WEB_REMIX (see `player::resolve`). With cookies this returns Premium
    /// itags; anonymously the ~128 kbps ceiling. No PO token, no yt-dlp.
    pub async fn get_stream(&self, video_id: &str) -> Result<YtStreamInfo, String> {
        player::resolve(video_id, self.cookies.as_deref()).await
    }

    // Public surfaces — work anonymously. `cookies.as_deref().unwrap_or("")`
    // hands an empty header to the parser, which the lower-level
    // discover/mix `post` and innertube::browse now interpret as "skip
    // SAPISID auth headers" (see browse_maybe_auth / discover::post).

    pub async fn start_mix(&self, seed_video_id: &str) -> Result<Vec<Track>, String> {
        mix::start_mix(seed_video_id, self.cookies.as_deref().unwrap_or("")).await
    }

    pub async fn discover_home(&self) -> Result<discover::DiscoverHome, String> {
        discover::fetch_home(self.cookies.as_deref().unwrap_or("")).await
    }

    pub async fn discover_continuation(
        &self,
        token: &str,
    ) -> Result<discover::DiscoverHome, String> {
        discover::fetch_continuation(token, self.cookies.as_deref().unwrap_or("")).await
    }

    pub async fn fetch_album_tracks(&self, browse_id: &str) -> Result<Vec<Track>, String> {
        discover::fetch_album_tracks(browse_id, self.cookies.as_deref().unwrap_or("")).await
    }

    /// The full album (header metadata — title, artist, year, cover — plus every
    /// track). The album page uses this for its YT-Music-style header; the
    /// thinner [`fetch_album_tracks`](Self::fetch_album_tracks) is for callers
    /// that only queue the tracks.
    pub async fn fetch_album(&self, browse_id: &str) -> Result<discover::YtAlbum, String> {
        discover::fetch_album(browse_id, self.cookies.as_deref().unwrap_or("")).await
    }

    pub async fn fetch_artist(&self, channel_id: &str) -> Result<discover::YtArtist, String> {
        discover::fetch_artist(channel_id, self.cookies.as_deref().unwrap_or("")).await
    }

    /// Confirms the cookie session is actually signed in — InnerTube
    /// `/browse?browseId=VLLM` (Liked Music) is the canonical probe: it
    /// returns a `signInEndpoint`-bearing message renderer for anonymous
    /// callers and real playlist content for signed-in ones.
    #[tracing::instrument(name = "yt.validate", skip_all)]
    pub async fn validate_cookies(&self) -> Result<(), String> {
        // Anonymous mode has no cookies to validate — succeed silently
        // so callers (settings probe, keepalive) treat it as healthy.
        let Some(cookies) = self.cookies.as_deref() else {
            return Ok(());
        };
        let json: Value = innertube::browse("VLLM", cookies).await?;
        if has_playlist_shelf(&json) {
            Ok(())
        } else {
            Err("YouTube returned no playlist shelf — cookies expired or browser signed out".into())
        }
    }
}

fn has_playlist_shelf(json: &Value) -> bool {
    json.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents")
        .or_else(|| {
            json.pointer(
                "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
            )
        })
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .any(|shelf| shelf.get("musicPlaylistShelfRenderer").is_some())
        })
        .unwrap_or(false)
}

impl Default for YouTubeMusicClient {
    fn default() -> Self {
        Self::new()
    }
}
