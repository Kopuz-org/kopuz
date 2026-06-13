//! One client trait over Jellyfin / Subsonic / YouTube Music (issue #347,
//! step 8). The per-service quirks (YT cookie auth + paged likes, Subsonic
//! salted calls, Jellyfin item shapes) live inside the impls; the reconciler,
//! auth gate, and `server_ops` dispatch through `Box<dyn MediaServerClient>`
//! instead of scattering `match service` blocks.

use async_trait::async_trait;
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
}

/// Resolve the trait impl for a connection — the ONE place service dispatch
/// happens.
pub fn client_for(conn: &ServerConn) -> Box<dyn MediaServerClient> {
    match conn.service {
        MusicService::Jellyfin => Box::new(JellyfinMsc {
            inner: JellyfinClient::new(
                &conn.url,
                Some(&conn.token),
                &conn.device_id,
                Some(&conn.user_id),
            ),
        }),
        MusicService::Subsonic | MusicService::Custom => Box::new(SubsonicMsc {
            inner: SubsonicClient::new(&conn.url, &conn.user_id, &conn.token),
        }),
        MusicService::YtMusic => Box::new(YtMsc {
            inner: YouTubeMusicClient::with_cookies(conn.token.clone()),
        }),
    }
}

struct JellyfinMsc {
    inner: JellyfinClient,
}

#[async_trait]
impl MediaServerClient for JellyfinMsc {
    async fn validate(&self) -> AuthOutcome {
        match self.inner.ping().await {
            Ok(()) => AuthOutcome::Valid,
            // ping surfaces the HTTP status in its error string; only a real
            // auth rejection means the token is dead.
            Err(e) if e.contains("401") || e.contains("403") => AuthOutcome::Expired,
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, String> {
        Ok(self
            .inner
            .get_favorite_items()
            .await?
            .into_iter()
            .map(|i| i.id)
            .collect())
    }

    async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String> {
        if on {
            self.inner.mark_favorite(item_id).await
        } else {
            self.inner.unmark_favorite(item_id).await
        }
    }

    async fn add_to_playlist(&self, playlist_id: &str, item_ids: &[String]) -> Vec<String> {
        let mut added = Vec::new();
        for id in item_ids {
            if self.inner.add_to_playlist(playlist_id, id).await.is_ok() {
                added.push(id.clone());
            }
        }
        added
    }

    async fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, String> {
        let refs: Vec<&str> = item_ids.iter().map(String::as_str).collect();
        self.inner.create_playlist(name, &refs).await
    }
}

struct SubsonicMsc {
    inner: SubsonicClient,
}

#[async_trait]
impl MediaServerClient for SubsonicMsc {
    async fn validate(&self) -> AuthOutcome {
        match self.inner.ping().await {
            Ok(()) => AuthOutcome::Valid,
            // Subsonic reports bad creds as error code 40 ("Wrong username or
            // password"); anything else is treated as a transient failure.
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
        self.inner.get_starred_song_ids().await
    }

    async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String> {
        if on {
            self.inner.star(item_id).await
        } else {
            self.inner.unstar(item_id).await
        }
    }

    async fn add_to_playlist(&self, playlist_id: &str, item_ids: &[String]) -> Vec<String> {
        let mut added = Vec::new();
        for id in item_ids {
            if self.inner.add_to_playlist(playlist_id, id).await.is_ok() {
                added.push(id.clone());
            }
        }
        added
    }

    async fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, String> {
        let refs: Vec<&str> = item_ids.iter().map(String::as_str).collect();
        self.inner.create_playlist(name, &refs).await
    }
}

struct YtMsc {
    inner: YouTubeMusicClient,
}

#[async_trait]
impl MediaServerClient for YtMsc {
    async fn validate(&self) -> AuthOutcome {
        match self.inner.validate_cookies().await {
            Ok(()) => AuthOutcome::Valid,
            Err(e) if e.contains("cookies expired") || e.contains("signed out") => {
                AuthOutcome::Expired
            }
            Err(_) => AuthOutcome::Unreachable,
        }
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, String> {
        let mut ids = Vec::new();
        self.inner
            .stream_liked_songs(|page| {
                ids.extend(page.into_iter().map(|t| t.id.key().into_owned()));
            })
            .await?;
        Ok(ids)
    }

    async fn set_favorite(&self, item_id: &str, on: bool) -> Result<(), String> {
        if on {
            self.inner.like_video(item_id).await
        } else {
            self.inner.unlike_video(item_id).await
        }
    }

    async fn add_to_playlist(&self, playlist_id: &str, item_ids: &[String]) -> Vec<String> {
        let mut added = Vec::new();
        for id in item_ids {
            if self.inner.add_to_playlist(playlist_id, id).await.is_ok() {
                added.push(id.clone());
            }
        }
        added
    }

    async fn create_playlist(&self, name: &str, item_ids: &[String]) -> Result<String, String> {
        let refs: Vec<&str> = item_ids.iter().map(String::as_str).collect();
        self.inner.create_playlist(name, "", &refs).await
    }
}
