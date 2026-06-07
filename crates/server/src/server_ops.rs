//! Service-agnostic dispatchers for playlist/favorite mutations.
//!
//! Each call site previously inlined the same `match service { Jellyfin … |
//! Subsonic … | YtMusic … }` block inside a `spawn(async move { … })`. Those
//! copies are collapsed here so the logic lives once and carries a span via
//! `#[tracing::instrument]`. Callers pass plain connection params (this crate
//! stays free of Dioxus) and keep their own Signal write-backs.

use crate::jellyfin::JellyfinClient;
use crate::subsonic::SubsonicClient;
use crate::ytmusic::YouTubeMusicClient;
use config::MusicService;

/// Resolved server credentials for a single request batch.
pub struct ServerConn {
    pub service: MusicService,
    pub url: String,
    pub token: String,
    pub user_id: String,
    pub device_id: String,
}

impl ServerConn {
    /// Build connection params from app config for the active server, or
    /// `None` when a field the active service requires is missing. An access
    /// token is always required; Jellyfin/Subsonic/Custom additionally require
    /// a `user_id` (YouTube Music authenticates by cookie only, so a missing
    /// user_id is fine there). Centralizing this stops every UI call site from
    /// coercing an absent user_id into `""` and firing a malformed
    /// authenticated request that silently fails.
    pub fn resolve(config: &config::AppConfig) -> Option<Self> {
        let server = config.server.as_ref()?;
        let token = server.access_token.clone()?;
        let user_id = match server.service {
            MusicService::YtMusic => server.user_id.clone().unwrap_or_default(),
            _ => server.user_id.clone()?,
        };
        Some(Self {
            service: server.service,
            url: server.url.clone(),
            token,
            user_id,
            device_id: config.device_id.clone(),
        })
    }
}

/// Pull the id segment out of a `"service:id[:…]"` track path. Returns `None`
/// for paths without an id or with an empty one.
pub fn parse_item_id(path: &str) -> Option<&str> {
    path.split(':').nth(1).filter(|s| !s.trim().is_empty())
}

/// Add tracks to an existing server playlist. Returns the ids that were added
/// successfully (callers that mirror the playlist into a local Signal use this;
/// fire-and-forget callers ignore it).
#[tracing::instrument(
    name = "playlist.add",
    skip(conn, item_ids),
    fields(service = ?conn.service, playlist_id = %playlist_id, count = item_ids.len())
)]
pub async fn add_tracks_to_playlist(
    conn: &ServerConn,
    playlist_id: &str,
    item_ids: &[String],
) -> Vec<String> {
    let mut added = Vec::new();
    match conn.service {
        MusicService::Jellyfin => {
            let remote = JellyfinClient::new(
                &conn.url,
                Some(&conn.token),
                &conn.device_id,
                Some(&conn.user_id),
            );
            for id in item_ids {
                if remote.add_to_playlist(playlist_id, id).await.is_ok() {
                    added.push(id.clone());
                }
            }
        }
        MusicService::Subsonic | MusicService::Custom => {
            let remote = SubsonicClient::new(&conn.url, &conn.user_id, &conn.token);
            for id in item_ids {
                if remote.add_to_playlist(playlist_id, id).await.is_ok() {
                    added.push(id.clone());
                }
            }
        }
        MusicService::YtMusic => {
            let yt = YouTubeMusicClient::with_cookies(conn.token.clone());
            for id in item_ids {
                if yt.add_to_playlist(playlist_id, id).await.is_ok() {
                    added.push(id.clone());
                }
            }
        }
    }
    added
}

/// Create a playlist on the server seeded with `item_ids`, returning its new id.
#[tracing::instrument(
    name = "playlist.create",
    skip(conn, item_ids),
    fields(service = ?conn.service, count = item_ids.len())
)]
pub async fn create_server_playlist(
    conn: &ServerConn,
    name: &str,
    item_ids: &[String],
) -> Result<String, String> {
    let id_refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
    match conn.service {
        MusicService::Jellyfin => {
            let remote = JellyfinClient::new(
                &conn.url,
                Some(&conn.token),
                &conn.device_id,
                Some(&conn.user_id),
            );
            remote.create_playlist(name, &id_refs).await
        }
        MusicService::Subsonic | MusicService::Custom => {
            let remote = SubsonicClient::new(&conn.url, &conn.user_id, &conn.token);
            remote.create_playlist(name, &id_refs).await
        }
        MusicService::YtMusic => {
            let yt = YouTubeMusicClient::with_cookies(conn.token.clone());
            yt.create_playlist(name, "", &id_refs).await
        }
    }
}

/// Star/unstar (or like/unlike) one or more tracks on the server. Attempts
/// every id and returns the first error encountered, so a single-id caller can
/// revert its optimistic update while a batch caller still touches every track.
#[tracing::instrument(
    name = "favorite.set",
    skip(conn, item_ids),
    fields(service = ?conn.service, favorite, count = item_ids.len())
)]
pub async fn set_tracks_favorite(
    conn: &ServerConn,
    item_ids: &[String],
    favorite: bool,
) -> Result<(), String> {
    let mut first_err: Option<String> = None;
    macro_rules! record {
        ($res:expr) => {
            if let Err(e) = $res {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        };
    }
    match conn.service {
        MusicService::Jellyfin => {
            let remote = JellyfinClient::new(
                &conn.url,
                Some(&conn.token),
                &conn.device_id,
                Some(&conn.user_id),
            );
            for id in item_ids {
                if favorite {
                    record!(remote.mark_favorite(id).await);
                } else {
                    record!(remote.unmark_favorite(id).await);
                }
            }
        }
        MusicService::Subsonic | MusicService::Custom => {
            let remote = SubsonicClient::new(&conn.url, &conn.user_id, &conn.token);
            for id in item_ids {
                if favorite {
                    record!(remote.star(id).await);
                } else {
                    record!(remote.unstar(id).await);
                }
            }
        }
        MusicService::YtMusic => {
            let yt = YouTubeMusicClient::with_cookies(conn.token.clone());
            for id in item_ids {
                if favorite {
                    record!(yt.like_video(id).await);
                } else {
                    record!(yt.unlike_video(id).await);
                }
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
