//! Connection resolution from the active sources, and legacy `"service:id"` track-id parsing, for the remote bridge.

use config::MusicService;

/// Resolved server credentials for a single request batch.
pub struct ServerConn {
    pub service: MusicService,
    pub url: String,
    pub token: String,
    pub user_id: String,
    pub device_id: String,
    pub apple_music_storefront: String,
    pub apple_music_language: String,
    /// The account a multi-account sign-in acts as; `None` is its own.
    pub account: Option<String>,
    /// Library roots for folder-tree backends (Nextcloud). Empty = auto-detect.
    pub folders: Vec<String>,
}

impl ServerConn {
    /// The active server's connection params, or `None` without a token (anonymous YouTube Music runs on none and ignores a leftover one), or without a `user_id` for a service that signs requests with one.
    pub fn resolve(sources: &crate::sources::Sources) -> Option<Self> {
        let server = sources.server.as_ref()?;
        let token = if server.service == MusicService::AudioCd
            || (server.service == MusicService::YtMusic && server.yt_anonymous)
        {
            String::new()
        } else {
            server.access_token.clone()?
        };
        let user_id = match server.service {
            MusicService::YtMusic
            | MusicService::AppleMusic
            | MusicService::Spotify
            | MusicService::AudioCd
            | MusicService::Clippsly => server.user_id.clone().unwrap_or_default(),
            _ => server.user_id.clone()?,
        };
        Some(Self {
            service: server.service,
            url: server.url.clone(),
            token,
            user_id,
            device_id: sources.device_id.clone(),
            apple_music_storefront: server.apple_music_storefront.clone(),
            apple_music_language: server.apple_music_language.clone(),
            account: server.account.clone(),
            folders: server
                .id
                .as_deref()
                .map(|id| sources.folders_for(id))
                .unwrap_or_default(),
        })
    }
}

/// Pull the id segment out of a `"service:id[:…]"` track path. Returns `None`
/// for paths without an id or with an empty one.
pub fn parse_item_id(path: &str) -> Option<&str> {
    path.split(':').nth(1).filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built via serde so the test isn't tied to MusicServer's full field list.
    fn cfg(service: &str, token: Option<&str>, user_id: Option<&str>) -> crate::sources::Sources {
        let mut server = serde_json::json!({
            "name": "test",
            "url": "http://localhost",
            "service": service,
        });
        if let Some(t) = token {
            server["access_token"] = t.into();
        }
        if let Some(u) = user_id {
            server["user_id"] = u.into();
        }
        crate::sources::Sources {
            server: Some(serde_json::from_value(server).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn audio_cd_needs_no_credentials() {
        let connection = ServerConn::resolve(&cfg("AudioCd", None, None)).unwrap();
        assert_eq!(connection.service, MusicService::AudioCd);
        assert!(connection.token.is_empty());
    }

    #[test]
    fn parse_item_id_cases() {
        assert_eq!(parse_item_id("jellyfin:abc"), Some("abc"));
        assert_eq!(parse_item_id("x:abc:def"), Some("abc"));
        assert_eq!(parse_item_id("nocolon"), None);
        assert_eq!(parse_item_id("x:"), None);
        assert_eq!(parse_item_id("x: "), None);
    }

    #[test]
    fn resolve_none_without_server_or_token() {
        // No server configured at all.
        assert!(ServerConn::resolve(&crate::sources::Sources::default()).is_none());
        // Server present but no access token.
        assert!(ServerConn::resolve(&cfg("Jellyfin", None, Some("u"))).is_none());
    }

    #[test]
    fn resolve_requires_user_id_except_ytmusic() {
        assert!(ServerConn::resolve(&cfg("Jellyfin", Some("t"), None)).is_none());
        assert!(ServerConn::resolve(&cfg("Subsonic", Some("t"), None)).is_none());
        assert!(ServerConn::resolve(&cfg("Custom", Some("t"), None)).is_none());

        let c = ServerConn::resolve(&cfg("Jellyfin", Some("t"), Some("u"))).unwrap();
        assert_eq!(c.user_id, "u");
        assert_eq!(c.token, "t");

        let yt = ServerConn::resolve(&cfg("YtMusic", Some("cookie"), None)).unwrap();
        assert_eq!(yt.token, "cookie");
        assert!(yt.user_id.is_empty());
    }
}
