use reader::models::{Track, TrackId};
use serde_json::Value;

pub mod accounts;
pub mod actions;
pub mod botguard;
pub mod browse;
#[cfg(not(target_os = "android"))]
pub mod browser_sessions;
pub mod clients;
pub mod cookies;
pub mod decipher;
pub mod diagnose;
pub mod discover;
pub mod innertube;
pub mod isolated_profile;
pub mod lyrics;
pub mod mix;
pub mod mutations;
pub mod player;
pub mod playlists;
pub mod search;
pub mod tracking;
pub mod verify_session_keepalive;

#[cfg(test)]
mod fixture_tests;

pub use player::YtStreamInfo;

pub const SOURCE_PREFIX: &str = "ytmusic";

/// Initialize the process-global V8 platform once, before any isolate.
///
/// Call it from the thread that will build the async runtime, before building
/// it. V8's default platform protects JIT memory with memory protection keys
/// on CPUs that have them, and a key's permission is inherited only by threads
/// created *after* it, by descendants of the thread that called initialize.
/// The decipher and BotGuard runtimes each spawn their own thread from
/// whichever runtime worker first needs them; when the first V8 user
/// initialised lazily from one worker, the other user's thread descended from
/// a worker without the key and died on first touch (SEGV_PKUERR). The lazy
/// calls stay as a backstop; they are no-ops after the eager one.
#[cfg(not(target_os = "android"))]
pub fn ensure_v8_platform() {
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

/// Rows carry the explicit badge in `badges`, cards in `subtitleBadges`.
pub(crate) fn has_explicit_badge(r: &Value) -> bool {
    ["badges", "subtitleBadges"]
        .iter()
        .filter_map(|key| r[*key].as_array())
        .flatten()
        .any(|badge| {
            badge["musicInlineBadgeRenderer"]["icon"]["iconType"] == "MUSIC_EXPLICIT_BADGE"
        })
}

/// "900M views", "1.2M subscribers"; a show called "Full Interviews" is a name.
pub(crate) fn is_count(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_digit())
        && [
            "views",
            "plays",
            "view",
            "play",
            "listeners",
            "subscribers",
            "monthly audience",
        ]
        .iter()
        .any(|suffix| text.ends_with(suffix))
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
pub async fn probe_stream(
    video_id: &str,
    cookies: Option<&str>,
    quality: config::StreamQuality,
) -> Result<YtStreamInfo, String> {
    Ok(player::resolve(video_id, cookies, quality).await?)
}

pub(crate) struct YouTubeMusicClient {
    cookies: Option<String>,
    /// The brand account every call acts as; `None` is the Google account.
    page_id: Option<String>,
}

impl YouTubeMusicClient {
    pub fn new() -> Self {
        Self {
            cookies: None,
            page_id: None,
        }
    }

    pub fn with_cookies(cookies: String) -> Self {
        Self::signed_in_as(cookies, None)
    }

    /// Anonymous cookies drop `page_id`: there is no account to act as.
    pub fn signed_in_as(cookies: String, page_id: Option<String>) -> Self {
        // Normalize the empty string (anonymous-mode marker, stored as
        // access_token: Some("")) to None so every `self.cookies` check
        // — auth-only guards, is_anonymous, the public-surface
        // unwrap_or("") — treats anonymous consistently.
        let cookies = (!cookies.is_empty()).then_some(cookies);
        Self {
            page_id: page_id.filter(|id| cookies.is_some() && !id.is_empty()),
            cookies,
        }
    }

    async fn account<F: std::future::Future>(&self, call: F) -> F::Output {
        if let (Some(cookies), Some(page_id)) = (self.cookies.as_deref(), self.page_id.as_deref())
            && innertube::manager(cookies, page_id).is_none()
        {
            match accounts::list(cookies).await {
                Ok(_) if innertube::manager(cookies, page_id).is_none() => {
                    innertube::remember_manager(cookies, page_id, None);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "no Google account found behind the brand account");
                }
            }
        }
        innertube::as_account(self.page_id.clone(), call).await
    }

    /// The channels this session can act as. Anonymous has none.
    pub async fn accounts(&self) -> Result<Vec<accounts::YtAccount>, String> {
        match self.cookies.as_deref() {
            Some(cookies) => accounts::list(cookies).await,
            None => Ok(Vec::new()),
        }
    }

    pub fn cookies(&self) -> Option<&str> {
        self.cookies.as_deref()
    }

    pub fn page_id(&self) -> Option<&str> {
        self.page_id.as_deref()
    }

    pub async fn search_tracks(
        &self,
        query: &str,
    ) -> Result<Vec<Track>, crate::source::SourceError> {
        self.account(async { search::music_search_tracks(query, self.cookies.as_deref()).await })
            .await
    }

    /// The channel's own name and square avatar.
    pub async fn artist_header(&self, channel_id: &str) -> Result<discover::ChannelHeader, String> {
        self.account(async {
            discover::artist_header(channel_id, self.cookies.as_deref().unwrap_or("")).await
        })
        .await
    }

    /// List the user's saved playlists (everything under Library →
    /// Playlists, minus the Liked Music auto-playlist). Returns summary
    /// rows; call [`get_playlist_entries`] for the tracks of any given
    /// playlist.
    /// Library playlists view (FEmusic_liked_playlists). Auth-only —
    /// returns Ok(vec![]) in anonymous mode so the playlists tab
    /// just shows empty rather than erroring.
    pub async fn list_playlists(&self) -> Result<Vec<playlists::YtPlaylistSummary>, String> {
        self.account(async {
            let Some(cookies) = self.cookies.as_deref() else {
                return Ok(Vec::new());
            };
            playlists::list_playlists(cookies).await
        })
        .await
    }

    /// Playlist contents. Public playlists work anonymously; the
    /// user's personal/private ones obviously won't.
    pub async fn get_playlist_entries(&self, playlist_id: &str) -> Result<Vec<Track>, String> {
        self.account(async {
            playlists::get_playlist_entries(playlist_id, self.cookies.as_deref().unwrap_or(""))
                .await
        })
        .await
    }

    pub async fn playlist_page(
        &self,
        playlist_id: &str,
        continuation: Option<&str>,
    ) -> Result<playlists::PlaylistPage, String> {
        self.account(async {
            playlists::playlist_page(
                playlist_id,
                self.cookies.as_deref().unwrap_or(""),
                continuation,
            )
            .await
        })
        .await
    }

    // Mutations are inherently auth-only — keep the explicit "not
    // signed in" error so callers (favorite toggle, add-to-playlist
    // modal) can surface a clear "sign in to enable" message.
    pub async fn like_video(&self, video_id: &str) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::like_video(video_id, cookies).await
        })
        .await
    }

    pub async fn unlike_video(&self, video_id: &str) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::unlike_video(video_id, cookies).await
        })
        .await
    }

    pub async fn dislike_video(&self, video_id: &str) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::dislike_video(video_id, cookies).await
        })
        .await
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, video_id: &str) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::add_to_playlist(playlist_id, video_id, cookies).await
        })
        .await
    }

    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        video_id: &str,
    ) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::remove_from_playlist(playlist_id, video_id, cookies).await
        })
        .await
    }

    pub async fn create_playlist(&self, title: &str, video_ids: &[&str]) -> Result<String, String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::create_playlist(title, video_ids, cookies).await
        })
        .await
    }

    /// `item_ref` is one [`actions`] handed out, or a video id.
    pub async fn rate(&self, item_ref: &str, rating: discover::Rating) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            let target = actions::read_ref(item_ref).ok_or("nothing to rate")?;
            mutations::rate(target, rating, cookies).await
        })
        .await
    }

    pub async fn save(&self, item_ref: &str, saved: bool) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            let target = actions::read_ref(item_ref).ok_or("nothing to save")?;
            mutations::save(target, saved, cookies).await
        })
        .await
    }

    pub async fn subscribe(&self, channel_id: &str, subscribed: bool) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::subscribe(channel_id, subscribed, cookies).await
        })
        .await
    }

    pub async fn remove_from_history(&self, token: &str) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::remove_from_history(token, cookies).await
        })
        .await
    }

    pub async fn edit_playlist(
        &self,
        playlist_id: &str,
        name: Option<&str>,
        description: Option<&str>,
        privacy: Option<discover::Privacy>,
    ) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::edit_playlist(playlist_id, name, description, privacy, cookies).await
        })
        .await
    }

    pub async fn delete_playlist(&self, playlist_id: &str) -> Result<(), String> {
        self.account(async {
            let cookies = self.cookies.as_deref().ok_or(ANON_AUTH_REQUIRED)?;
            mutations::delete_playlist(playlist_id, cookies).await
        })
        .await
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
        self.account(async {
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
        })
        .await
    }

    /// One page of Liked Music, cursor-driven: pass `None` for the first page,
    /// then the returned token for each next page (until it's `None`). Lets a
    /// caller paginate without an internal callback (cross-page dedup is the
    /// caller's job — YT repeats tracks at page boundaries). Anonymous → empty.
    pub async fn liked_songs_page(
        &self,
        continuation: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>), String> {
        self.account(async {
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
        })
        .await
    }

    /// Resolves a playable stream URL via native sig/n deciphering against
    /// WEB_REMIX (see `player::resolve`). With cookies this returns Premium
    /// itags; anonymously the ~128 kbps ceiling. No PO token, no yt-dlp.
    pub async fn get_stream(
        &self,
        video_id: &str,
        quality: config::StreamQuality,
    ) -> Result<YtStreamInfo, player::StreamFailure> {
        self.account(async { player::resolve(video_id, self.cookies.as_deref(), quality).await })
            .await
    }

    pub async fn get_video_stream(&self, video_id: &str) -> Result<player::YtVideoStream, String> {
        self.account(async { player::resolve_video(video_id, self.cookies.as_deref()).await })
            .await
    }

    pub async fn counterpart(&self, video_id: &str) -> Result<Option<reader::Counterpart>, String> {
        self.account(async {
            mix::counterpart(video_id, self.cookies.as_deref().unwrap_or("")).await
        })
        .await
    }

    // Public surfaces work anonymously. `cookies.as_deref().unwrap_or("")`
    // hands an empty header to the parser, which the lower-level
    // discover/mix `post` and innertube::browse interpret as unauthenticated.

    #[cfg(test)]
    pub(crate) async fn acting_as(&self) -> Option<String> {
        innertube::as_account(self.page_id.clone(), async { innertube::page_id() }).await
    }

    /// Whether this client has cookies — i.e. the user is signed in rather than
    /// browsing YT anonymously.
    pub fn is_authenticated(&self) -> bool {
        self.cookies.is_some()
    }

    pub async fn fetch_track(&self, video_id: &str) -> Result<Option<Track>, String> {
        mix::watch_track(video_id, self.cookies.as_deref()).await
    }

    pub async fn account_avatar(&self) -> Result<Option<String>, String> {
        let Some(cookies) = self.cookies.as_deref() else {
            return Ok(None);
        };
        let menu = innertube::post(
            clients::WEB_REMIX,
            "account/account_menu",
            serde_json::json!({}),
            Some(cookies),
        )
        .await?;
        Ok(account_photo(&menu))
    }

    pub async fn start_mix(&self, seed_video_id: &str) -> Result<mix::MixPage, String> {
        self.account(async {
            mix::fetch(
                mix::MixSeed::Video(seed_video_id),
                self.cookies.as_deref().unwrap_or(""),
            )
            .await
        })
        .await
    }

    pub async fn start_playlist_mix(&self, playlist_id: &str) -> Result<mix::MixPage, String> {
        self.account(async {
            mix::fetch(
                mix::MixSeed::from_playlist_ref(playlist_id),
                self.cookies.as_deref().unwrap_or(""),
            )
            .await
        })
        .await
    }

    pub async fn mix_continuation(
        &self,
        playlist_id: &str,
        token: &str,
    ) -> Result<mix::MixPage, String> {
        self.account(async {
            mix::fetch_continuation(playlist_id, token, self.cookies.as_deref().unwrap_or("")).await
        })
        .await
    }

    pub async fn discover_home(&self) -> Result<discover::DiscoverHome, String> {
        self.account(async { discover::fetch_home(self.cookies.as_deref().unwrap_or("")).await })
            .await
    }

    pub async fn discover_continuation(
        &self,
        token: &str,
    ) -> Result<discover::DiscoverHome, String> {
        self.account(async {
            discover::fetch_continuation(token, self.cookies.as_deref().unwrap_or("")).await
        })
        .await
    }

    /// The library tabs and history come back without shelves when signed out.
    pub async fn browse_page(&self, id: &str) -> Result<discover::BrowsePage, String> {
        browse::fetch_page(id, self.cookies.as_deref()).await
    }

    pub async fn browse_continuation(&self, token: &str) -> Result<discover::BrowsePage, String> {
        browse::fetch_continuation(token, self.cookies.as_deref()).await
    }

    pub async fn related(&self, video_id: &str) -> Result<discover::BrowsePage, String> {
        browse::fetch_related(video_id, self.cookies.as_deref()).await
    }

    pub async fn search_page(
        &self,
        query: &str,
        filter: Option<&'static browse::search::Filter>,
    ) -> Result<browse::search::SearchPage, String> {
        browse::fetch_search(query, filter, self.cookies.as_deref()).await
    }

    pub async fn search_suggestions(
        &self,
        query: &str,
    ) -> Result<Vec<browse::search::Suggestion>, String> {
        browse::fetch_suggestions(query, self.cookies.as_deref()).await
    }

    pub async fn fetch_album_tracks(&self, browse_id: &str) -> Result<Vec<Track>, String> {
        self.account(async {
            discover::fetch_album_tracks(browse_id, self.cookies.as_deref().unwrap_or("")).await
        })
        .await
    }

    /// The full album (header metadata — title, artist, year, cover — plus every
    /// track). The album page uses this for its YT-Music-style header; the
    /// thinner [`fetch_album_tracks`](Self::fetch_album_tracks) is for callers
    /// that only queue the tracks.
    pub async fn fetch_album(&self, browse_id: &str) -> Result<discover::YtAlbum, String> {
        self.account(async {
            discover::fetch_album(browse_id, self.cookies.as_deref().unwrap_or("")).await
        })
        .await
    }

    pub async fn fetch_artist(&self, channel_id: &str) -> Result<discover::YtArtist, String> {
        self.account(async {
            discover::fetch_artist(channel_id, self.cookies.as_deref().unwrap_or("")).await
        })
        .await
    }

    /// What YouTube says about the cookie session. InnerTube
    /// `/browse?browseId=VLLM` (Liked Music) is the probe: real playlist
    /// content for a signed-in caller, a `signInEndpoint` prompt for one
    /// that is not.
    #[tracing::instrument(name = "yt.validate", skip_all)]
    pub async fn check_session(&self) -> SessionCheck {
        self.account(async {
            // Anonymous mode has no session to lose.
            let Some(cookies) = self.cookies.as_deref() else {
                return SessionCheck::Valid;
            };
            match innertube::browse("VLLM", cookies).await {
                Ok(json) => session_in(&json),
                Err(error) if error.contains("SAPISID missing") || error.contains("HTTP 401") => {
                    SessionCheck::SignedOut
                }
                Err(error) => SessionCheck::Unknown(error),
            }
        })
        .await
    }
}

fn account_photo(menu: &Value) -> Option<String> {
    let header = &menu["actions"][0]["openPopupAction"]["popup"]["multiPageMenuRenderer"]["header"]
        ["activeAccountHeaderRenderer"];
    header["accountPhoto"]["thumbnails"]
        .as_array()?
        .iter()
        .max_by_key(|thumb| thumb["width"].as_u64().unwrap_or(0))?["url"]
        .as_str()
        .filter(|url| !url.is_empty())
        .map(str::to_string)
}

/// What YouTube's answer to a signed-in request says about the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCheck {
    Valid,
    /// YouTube answered the way it answers someone signed out.
    SignedOut,
    /// No answer, or one in a shape this build cannot read. That says
    /// nothing about the session, so callers keep it: dropping a working
    /// session over a bad hour on YouTube's side is what sent people to sign
    /// in again, and that sign-in failed the same check.
    Unknown(String),
}

impl SessionCheck {
    /// Everything but a definite signed-out answer.
    pub fn is_usable(&self) -> bool {
        !matches!(self, SessionCheck::SignedOut)
    }
}

fn session_in(json: &Value) -> SessionCheck {
    if has_playlist_shelf(json) {
        SessionCheck::Valid
    } else if json.to_string().contains("\"signInEndpoint\"") {
        SessionCheck::SignedOut
    } else {
        SessionCheck::Unknown(
            "YouTube answered Liked Music with neither a playlist nor a sign-in prompt".into(),
        )
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

#[cfg(test)]
mod account_tests {
    use serde_json::json;

    /// The shape `account/account_menu` answers with, trimmed to the photo.
    #[test]
    fn the_account_menu_names_the_largest_photo() {
        let menu = json!({"actions": [{"openPopupAction": {"popup": {"multiPageMenuRenderer": {
            "header": {"activeAccountHeaderRenderer": {
                "accountName": {"runs": [{"text": "Someone"}]},
                "accountPhoto": {"thumbnails": [
                    {"url": "https://yt3.ggpht.com/a=s48", "width": 48, "height": 48},
                    {"url": "https://yt3.ggpht.com/a=s88", "width": 88, "height": 88}
                ]}
            }}
        }}}}]});
        assert_eq!(
            super::account_photo(&menu).as_deref(),
            Some("https://yt3.ggpht.com/a=s88")
        );
        assert_eq!(super::account_photo(&json!({})), None);
    }
}

#[cfg(test)]
mod tests {
    use super::YouTubeMusicClient;

    #[test]
    fn only_a_signed_in_client_acts_as_a_brand_account() {
        let signed_in = YouTubeMusicClient::signed_in_as("SAPISID=a".into(), Some("42".into()));
        assert_eq!(signed_in.page_id.as_deref(), Some("42"));
        let anonymous = YouTubeMusicClient::signed_in_as(String::new(), Some("42".into()));
        assert_eq!(anonymous.page_id, None);
        let own = YouTubeMusicClient::signed_in_as("SAPISID=a".into(), Some(String::new()));
        assert_eq!(own.page_id, None);
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod v8_thread_tests {
    /// One isolate on a fresh thread, running enough of a loop to hit the JIT.
    /// The value is the proof the isolate actually executed code.
    fn run_isolate_on_new_thread() -> std::thread::JoinHandle<i64> {
        std::thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            rt.block_on(async {
                let mut js = deno_core::JsRuntime::new(deno_core::RuntimeOptions::default());
                let value = js
                    .execute_script(
                        "test:jit",
                        "let s = 0; for (let i = 0; i < 200000; i++) { s += i % 7; } s",
                    )
                    .expect("script");
                let scope = &mut js.handle_scope();
                let local = deno_core::v8::Local::new(scope, value);
                local.integer_value(scope).expect("integer")
            })
        })
    }

    /// V8's default platform guards JIT memory with protection keys on CPUs
    /// that have them, and only descendants of the initialising thread carry
    /// the key. Two isolates on threads spawned from two different runtime
    /// workers is exactly the shape of the decipher and BotGuard runtimes; it
    /// segfaulted (SEGV_PKUERR) whenever V8 was initialised lazily from one
    /// worker. Initialising on the thread that builds the runtime -- what
    /// `daemon::boot::prepare_thread` does -- makes every worker a descendant.
    #[test]
    fn isolates_on_threads_from_different_workers_survive_when_v8_inits_on_their_ancestor() {
        let core = std::thread::spawn(|| {
            super::ensure_v8_platform();
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("runtime");
            rt.block_on(async {
                let first = tokio::task::spawn_blocking(run_isolate_on_new_thread)
                    .await
                    .expect("spawn");
                let second = tokio::task::spawn_blocking(run_isolate_on_new_thread)
                    .await
                    .expect("spawn");
                (first.join().expect("first"), second.join().expect("second"))
            })
        });
        let (first, second) = core.join().expect("core thread");
        assert_eq!(first, 599_994);
        assert_eq!(second, 599_994);
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_playlist_shelf_is_a_valid_session() {
        let page = json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [{"musicPlaylistShelfRenderer": {}}]}}}}]}}});
        assert_eq!(session_in(&page), SessionCheck::Valid);
    }

    #[test]
    fn a_sign_in_prompt_is_a_signed_out_session() {
        let page = json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [{"messageRenderer": {"button": {"buttonRenderer": {"navigationEndpoint": {"signInEndpoint": {}}}}}}]}}}}]}}});
        assert_eq!(session_in(&page), SessionCheck::SignedOut);
    }

    /// The answer YouTube gives a stale client, or gives anyone during an
    /// outage, has neither. Calling that signed out sent people to sign in
    /// again, and the new session failed the same check.
    #[test]
    fn an_answer_with_neither_keeps_the_session() {
        let page = json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": []}}});
        let check = session_in(&page);
        assert!(matches!(check, SessionCheck::Unknown(_)), "{check:?}");
        assert!(check.is_usable());
    }
}
