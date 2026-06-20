//! Connection resolution + legacy track-id parsing for the remote bridge.
//!
//! [`ServerConn`] hydrates the active server's request params from config (used
//! by [`crate::source`] and [`crate::sync`] to build a remote client), and
//! [`parse_item_id`] extracts an item id from a legacy `"service:id"` path. The
//! playlist/favorite mutations that used to live here are now methods on
//! [`crate::source::MediaSource`].

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

#[cfg(test)]
mod tests {
    use super::*;

    // Build an AppConfig with a server via serde so the test isn't tied to
    // MusicServer's full field list (only name/url are required; the rest
    // default).
    fn cfg(service: &str, token: Option<&str>, user_id: Option<&str>) -> config::AppConfig {
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
        config::AppConfig {
            server: Some(serde_json::from_value(server).unwrap()),
            ..Default::default()
        }
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
        assert!(ServerConn::resolve(&config::AppConfig::default()).is_none());
        // Server present but no access token.
        assert!(ServerConn::resolve(&cfg("Jellyfin", None, Some("u"))).is_none());
    }

    #[test]
    fn resolve_requires_user_id_except_ytmusic() {
        // Jellyfin/Subsonic/Custom need a user_id — missing → None.
        assert!(ServerConn::resolve(&cfg("Jellyfin", Some("t"), None)).is_none());
        assert!(ServerConn::resolve(&cfg("Subsonic", Some("t"), None)).is_none());
        assert!(ServerConn::resolve(&cfg("Custom", Some("t"), None)).is_none());

        // …present → resolves with the id carried through.
        let c = ServerConn::resolve(&cfg("Jellyfin", Some("t"), Some("u"))).unwrap();
        assert_eq!(c.user_id, "u");
        assert_eq!(c.token, "t");

        // YtMusic authenticates by cookie — user_id optional, still resolves.
        let yt = ServerConn::resolve(&cfg("YtMusic", Some("cookie"), None)).unwrap();
        assert_eq!(yt.token, "cookie");
        assert!(yt.user_id.is_empty());
    }
}
