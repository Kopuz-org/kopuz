use serde::{Deserialize, Serialize};

/// Persistent Spotify configuration block, mirrored from the user-facing
/// `[spotify]` section of the application config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpotifyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub client_id: String,
    #[serde(default = "default_redirect_uri")]
    pub redirect_uri: String,
    #[serde(default)]
    pub backend: SpotifyBackendKind,
    #[serde(default = "default_device_name")]
    pub device_name: String,
    #[serde(default)]
    pub default_device_id: String,
    #[serde(default)]
    pub market: String,
}

impl Default for SpotifyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_id: String::new(),
            redirect_uri: default_redirect_uri(),
            backend: SpotifyBackendKind::default(),
            device_name: default_device_name(),
            default_device_id: String::new(),
            market: String::new(),
        }
    }
}

fn default_redirect_uri() -> String {
    // Loopback IP literal per Spotify's PKCE guidance. Do not use `localhost`.
    // Spotify requires the redirect_uri sent in the auth request to byte-match
    // the URI registered in the Dashboard, so we ship a fixed port and ask
    // the user to register exactly this string. 8898 is in the IANA-unassigned
    // user range and unlikely to conflict.
    "http://127.0.0.1:8898/callback".to_string()
}

fn default_device_name() -> String {
    "Rust Music Player".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SpotifyBackendKind {
    #[default]
    #[serde(rename = "connect")]
    Connect,
    #[serde(rename = "web-playback", alias = "web_playback")]
    WebPlayback,
}

/// Persisted OAuth credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds when the access token expires.
    pub expires_at: u64,
    pub scope: String,
    pub token_type: String,
}

impl Tokens {
    /// True if the token has expired or will expire within `skew_secs`.
    pub fn is_expired_within(&self, now_unix: u64, skew_secs: u64) -> bool {
        now_unix.saturating_add(skew_secs) >= self.expires_at
    }
}

/// Internal metadata representation that the provider maps Spotify objects into.
#[derive(Debug, Clone, Default)]
pub struct TrackMetadata {
    pub id: String,
    pub uri: String,
    pub title: String,
    pub artist_names: Vec<String>,
    pub album_title: String,
    pub duration_ms: u64,
    pub artwork_urls: Vec<String>,
    pub explicit: bool,
    pub external_url: Option<String>,
    pub is_playable: Option<bool>,
    pub linked_from_uri: Option<String>,
    pub restrictions_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_redirect_uses_loopback_ip_literal() {
        let c = SpotifyConfig::default();
        assert!(c.redirect_uri.starts_with("http://127.0.0.1:"));
        assert!(!c.redirect_uri.contains("localhost"));
        assert!(c.redirect_uri.ends_with("/callback"));
    }

    #[test]
    fn backend_parses_both_forms() {
        let v: SpotifyBackendKind = serde_json::from_str("\"connect\"").unwrap();
        assert_eq!(v, SpotifyBackendKind::Connect);
        let v: SpotifyBackendKind = serde_json::from_str("\"web-playback\"").unwrap();
        assert_eq!(v, SpotifyBackendKind::WebPlayback);
        let v: SpotifyBackendKind = serde_json::from_str("\"web_playback\"").unwrap();
        assert_eq!(v, SpotifyBackendKind::WebPlayback);
    }

    #[test]
    fn token_expiry_logic() {
        let t = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 1_000,
            scope: String::new(),
            token_type: "Bearer".into(),
        };
        assert!(t.is_expired_within(1_000, 0));
        assert!(t.is_expired_within(950, 60));
        assert!(!t.is_expired_within(900, 60));
    }
}
