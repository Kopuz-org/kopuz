//! The remote-server bridge over Jellyfin / Subsonic / YouTube Music (issue
//! #347, step 8). Per-service quirks (YT cookie auth + paged likes, Subsonic
//! salted calls, Jellyfin item shapes) live inside the per-remote impls; the
//! reconciler, auth gate, and `server_ops` dispatch through
//! `Box<dyn MediaServerClient>` instead of scattering `match service` blocks.
//!
//! A trait (not an enum) because the source set is growing — adding one of the
//! several planned remotes is a single new impl here, not an arm in every
//! method. `async_trait` because the backend is chosen at runtime (so dispatch
//! is `dyn`, which native `async fn` in traits can't yet do).

use async_trait::async_trait;
use config::MusicService;

use crate::jellyfin::JellyfinClient;
use crate::server_ops::ServerConn;
use crate::subsonic::SubsonicClient;
use crate::ytmusic::YouTubeMusicClient;
use crate::ytmusic::player::AudioFormat;

/// What credential validation concluded. `Unreachable` ≠ `Expired`: a network
/// blip must not reprompt sign-in — only a real auth rejection does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    Valid,
    Expired,
    Unreachable,
}

/// A resolved playable stream for one server item. `format`/`user_agent`/
/// `duration_secs`/`bitrate` are populated only where the source provides them
/// (YT's deciphered stream carries format, user-agent and probed duration/bitrate);
/// a plain progressive URL (Jellyfin/Subsonic) leaves them `None`.
pub struct StreamInfo {
    pub url: String,
    pub format: Option<(AudioFormat, bool)>,
    pub user_agent: Option<String>,
    pub duration_secs: Option<u64>,
    pub bitrate: Option<u32>,
}

/// The remote-server bridge: the operations that must talk to the actual
/// service. One impl per remote; adding a service = implement this once.
#[async_trait]
pub trait MediaServerClient: Send + Sync {
    /// Check the stored creds against the server.
    async fn validate(&self) -> AuthOutcome;

    /// All favorited item ids on the remote (YT pages internally).
    async fn fetch_favorites(&self) -> Result<Vec<String>, String>;

    /// Favorite/unfavorite one item.
    async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String>;

    /// Add items to a playlist; returns the ids that were added successfully.
    async fn add_to_playlist(&self, playlist_id: &str, item_ids: &[String]) -> Vec<String>;

    /// Create a playlist seeded with items, returning its new id.
    async fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, String>;

    /// Resolve a playable stream for one item id (the one genuinely per-source
    /// op — a URL for Jellyfin/Subsonic, a deciphered stream for YT).
    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, String>;
}

/// Resolve the client for a connection — the ONE place service dispatch happens.
pub fn client_for(conn: &ServerConn) -> Box<dyn MediaServerClient> {
    match conn.service {
        MusicService::Jellyfin => Box::new(JellyfinClient::new(
            &conn.url,
            Some(&conn.token),
            &conn.device_id,
            Some(&conn.user_id),
        )),
        MusicService::Subsonic | MusicService::Custom => {
            Box::new(SubsonicClient::new(&conn.url, &conn.user_id, &conn.token))
        }
        MusicService::YtMusic => Box::new(YouTubeMusicClient::with_cookies(conn.token.clone())),
    }
}

#[async_trait]
impl MediaServerClient for JellyfinClient {
    async fn validate(&self) -> AuthOutcome {
        // ping surfaces the HTTP status in its error string; only a real auth
        // rejection means the token is dead.
        match self.ping().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.contains("401") || e.contains("403") => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, String> {
        Ok(self
            .get_favorite_items()
            .await?
            .into_iter()
            .map(|i| i.id)
            .collect())
    }

    async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String> {
        if on {
            self.mark_favorite(item_id).await
        } else {
            self.unmark_favorite(item_id).await
        }
    }

    async fn add_to_playlist(&self, playlist_id: &str, item_ids: &[String]) -> Vec<String> {
        let mut added = Vec::new();
        for id in item_ids {
            if JellyfinClient::add_to_playlist(self, playlist_id, id)
                .await
                .is_ok()
            {
                added.push(id.clone());
            }
        }
        added
    }

    async fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, String> {
        let refs: Vec<&str> = item_ids.iter().map(String::as_str).collect();
        JellyfinClient::create_playlist(self, name, &refs).await
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, String> {
        Ok(StreamInfo {
            url: self.stream_url(item_id),
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
        })
    }
}

#[async_trait]
impl MediaServerClient for SubsonicClient {
    async fn validate(&self) -> AuthOutcome {
        // Subsonic reports bad creds as error code 40 ("Wrong username or
        // password"); anything else is treated as a transient failure.
        match self.ping().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e)
                if e.contains("Wrong username")
                    || e.contains("not authorized")
                    || e.contains("code 40") =>
            {
                AuthOutcome::Expired
            }
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, String> {
        self.get_starred_song_ids().await
    }

    async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String> {
        if on {
            self.star(item_id).await
        } else {
            self.unstar(item_id).await
        }
    }

    async fn add_to_playlist(&self, playlist_id: &str, item_ids: &[String]) -> Vec<String> {
        let mut added = Vec::new();
        for id in item_ids {
            if SubsonicClient::add_to_playlist(self, playlist_id, id)
                .await
                .is_ok()
            {
                added.push(id.clone());
            }
        }
        added
    }

    async fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, String> {
        let refs: Vec<&str> = item_ids.iter().map(String::as_str).collect();
        SubsonicClient::create_playlist(self, name, &refs).await
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, String> {
        Ok(StreamInfo {
            url: self.stream_url(item_id)?,
            format: None,
            user_agent: None,
            duration_secs: None,
            bitrate: None,
        })
    }
}

#[async_trait]
impl MediaServerClient for YouTubeMusicClient {
    async fn validate(&self) -> AuthOutcome {
        match self.validate_cookies().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.contains("cookies expired") || e.contains("signed out") => {
                AuthOutcome::Expired
            }
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, String> {
        let mut ids = Vec::new();
        self.stream_liked_songs(|page| {
            ids.extend(page.into_iter().map(|t| t.id.key().into_owned()));
        })
        .await?;
        Ok(ids)
    }

    async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String> {
        if on {
            self.like_video(item_id).await
        } else {
            self.unlike_video(item_id).await
        }
    }

    async fn add_to_playlist(&self, playlist_id: &str, item_ids: &[String]) -> Vec<String> {
        let mut added = Vec::new();
        for id in item_ids {
            if YouTubeMusicClient::add_to_playlist(self, playlist_id, id)
                .await
                .is_ok()
            {
                added.push(id.clone());
            }
        }
        added
    }

    async fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, String> {
        let refs: Vec<&str> = item_ids.iter().map(String::as_str).collect();
        YouTubeMusicClient::create_playlist(self, name, &refs).await
    }

    async fn resolve_stream(&self, item_id: &str) -> Result<StreamInfo, String> {
        let info = self.get_stream(item_id).await?;
        Ok(StreamInfo {
            url: info.url,
            format: Some((info.format, info.range_safe)),
            user_agent: Some(info.user_agent),
            duration_secs: info.duration_secs,
            bitrate: info.bitrate,
        })
    }
}
