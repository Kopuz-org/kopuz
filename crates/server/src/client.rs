//! One client over Jellyfin / Subsonic / YouTube Music (issue #347, step 8).
//! The per-service quirks (YT cookie auth + paged likes, Subsonic salted calls,
//! Jellyfin item shapes) live inside the variants; the reconciler, auth gate,
//! and `server_ops` dispatch through `MediaServerClient` instead of scattering
//! `match service` blocks. An enum (not a `dyn` trait) so the async methods
//! stay native — `async fn` in traits isn't `dyn`-compatible — and dispatch is
//! static, no vtable or heap box.

use config::MusicService;

use crate::jellyfin::JellyfinClient;
use crate::server_ops::ServerConn;
use crate::subsonic::SubsonicClient;
use crate::ytmusic::YouTubeMusicClient;

/// What credential validation concluded. `Unreachable` ≠ `Expired`: a network
/// blip must not reprompt sign-in — only a real auth rejection does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    Valid,
    Expired,
    Unreachable,
}

/// A resolved server client. The per-service impls back each variant.
pub enum MediaServerClient {
    Jellyfin(JellyfinClient),
    Subsonic(SubsonicClient),
    YtMusic(YouTubeMusicClient),
}

/// Resolve the client for a connection — the ONE place service dispatch happens.
pub fn client_for(conn: &ServerConn) -> MediaServerClient {
    match conn.service {
        MusicService::Jellyfin => MediaServerClient::Jellyfin(JellyfinClient::new(
            &conn.url,
            Some(&conn.token),
            &conn.device_id,
            Some(&conn.user_id),
        )),
        MusicService::Subsonic | MusicService::Custom => {
            MediaServerClient::Subsonic(SubsonicClient::new(&conn.url, &conn.user_id, &conn.token))
        }
        MusicService::YtMusic => {
            MediaServerClient::YtMusic(YouTubeMusicClient::with_cookies(conn.token.clone()))
        }
    }
}

impl MediaServerClient {
    /// Check the stored creds against the server.
    pub async fn validate(&self) -> AuthOutcome {
        match self {
            // ping surfaces the HTTP status in its error string; only a real
            // auth rejection means the token is dead.
            Self::Jellyfin(c) => match c.ping().await {
                Ok(()) => AuthOutcome::Valid,
                Err(e) if e.contains("401") || e.contains("403") => AuthOutcome::Expired,
                Err(_) => AuthOutcome::Unreachable,
            },
            // Subsonic reports bad creds as error code 40 ("Wrong username or
            // password"); anything else is treated as a transient failure.
            Self::Subsonic(c) => match c.ping().await {
                Ok(()) => AuthOutcome::Valid,
                Err(e)
                    if e.contains("Wrong username")
                        || e.contains("not authorized")
                        || e.contains("code 40") =>
                {
                    AuthOutcome::Expired
                }
                Err(_) => AuthOutcome::Unreachable,
            },
            Self::YtMusic(c) => match c.validate_cookies().await {
                Ok(()) => AuthOutcome::Valid,
                Err(e) if e.contains("cookies expired") || e.contains("signed out") => {
                    AuthOutcome::Expired
                }
                Err(_) => AuthOutcome::Unreachable,
            },
        }
    }

    /// All favorited item ids on the remote (YT pages internally).
    pub async fn fetch_favorites(&self) -> Result<Vec<String>, String> {
        match self {
            Self::Jellyfin(c) => Ok(c
                .get_favorite_items()
                .await?
                .into_iter()
                .map(|i| i.id)
                .collect()),
            Self::Subsonic(c) => c.get_starred_song_ids().await,
            Self::YtMusic(c) => {
                let mut ids = Vec::new();
                c.stream_liked_songs(|page| {
                    ids.extend(page.into_iter().map(|t| t.id.key().into_owned()));
                })
                .await?;
                Ok(ids)
            }
        }
    }

    /// Favorite/unfavorite one item.
    pub async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String> {
        match self {
            Self::Jellyfin(c) => {
                if on {
                    c.mark_favorite(item_id).await
                } else {
                    c.unmark_favorite(item_id).await
                }
            }
            Self::Subsonic(c) => {
                if on {
                    c.star(item_id).await
                } else {
                    c.unstar(item_id).await
                }
            }
            Self::YtMusic(c) => {
                if on {
                    c.like_video(item_id).await
                } else {
                    c.unlike_video(item_id).await
                }
            }
        }
    }

    /// Add items to a playlist; returns the ids that were added successfully.
    pub async fn add_to_playlist(&self, playlist_id: &str, item_ids: &[String]) -> Vec<String> {
        let mut added = Vec::new();
        for id in item_ids {
            let ok = match self {
                Self::Jellyfin(c) => c.add_to_playlist(playlist_id, id).await.is_ok(),
                Self::Subsonic(c) => c.add_to_playlist(playlist_id, id).await.is_ok(),
                Self::YtMusic(c) => c.add_to_playlist(playlist_id, id).await.is_ok(),
            };
            if ok {
                added.push(id.clone());
            }
        }
        added
    }

    /// Create a playlist seeded with items, returning its new id.
    pub async fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, String> {
        let refs: Vec<&str> = item_ids.iter().map(String::as_str).collect();
        match self {
            Self::Jellyfin(c) => c.create_playlist(name, &refs).await,
            Self::Subsonic(c) => c.create_playlist(name, &refs).await,
            Self::YtMusic(c) => c.create_playlist(name, "", &refs).await,
        }
    }
}
