use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpotifyError {
    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("access token expired")]
    TokenExpired,

    #[error("token refresh failed: {0}")]
    TokenRefreshFailed(String),

    #[error("http error: {0}")]
    Http(String),

    #[error("spotify api error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("rate limited, retry after {retry_after}s")]
    RateLimited { retry_after: u64 },

    #[error("spotify premium account required")]
    PremiumRequired,

    #[error("forbidden")]
    Forbidden,

    #[error("no active spotify device")]
    NoActiveDevice,

    #[error("selected device is restricted")]
    RestrictedDevice,

    #[error("selected device is unavailable")]
    DeviceUnavailable,

    #[error("web playback sdk unavailable in this environment")]
    WebPlaybackUnavailable,

    #[error("web playback sdk account error")]
    WebPlaybackAccountError,

    #[error("web playback sdk authentication error")]
    WebPlaybackAuthenticationError,

    #[error("web playback sdk initialization error: {0}")]
    WebPlaybackInitializationError(String),

    #[error("web playback sdk playback error: {0}")]
    WebPlaybackPlaybackError(String),

    #[error("malformed response from spotify: {0}")]
    MalformedResponse(String),

    #[error("backend not supported in this build")]
    UnsupportedBackend,

    #[error("not logged in")]
    NotLoggedIn,

    #[error("io error: {0}")]
    Io(String),
}

impl From<reqwest::Error> for SpotifyError {
    fn from(e: reqwest::Error) -> Self {
        SpotifyError::Http(e.to_string())
    }
}

impl From<std::io::Error> for SpotifyError {
    fn from(e: std::io::Error) -> Self {
        SpotifyError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for SpotifyError {
    fn from(e: serde_json::Error) -> Self {
        SpotifyError::MalformedResponse(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, SpotifyError>;

/// Map an HTTP response status and body to a typed SpotifyError.
///
/// The body string is expected to be the (possibly empty) response body. Spotify
/// returns a JSON envelope of the form `{"error":{"status":N,"message":"..."}}`
/// for most errors, plus contextual fields like `reason` for player endpoints.
pub fn classify_http(status: u16, body: &str, retry_after: Option<u64>) -> SpotifyError {
    // Try to extract { "error": { "status", "message", "reason" } }
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or(body)
        .to_string();
    let reason = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("reason"))
        .and_then(|m| m.as_str())
        .map(|s| s.to_string());

    match status {
        401 => SpotifyError::TokenExpired,
        403 => match reason.as_deref() {
            Some("PREMIUM_REQUIRED") => SpotifyError::PremiumRequired,
            Some("DEVICE_NOT_CONTROLLABLE") => SpotifyError::RestrictedDevice,
            _ => {
                if message.to_lowercase().contains("premium") {
                    SpotifyError::PremiumRequired
                } else {
                    SpotifyError::Forbidden
                }
            }
        },
        404 => match reason.as_deref() {
            Some("NO_ACTIVE_DEVICE") => SpotifyError::NoActiveDevice,
            Some("DEVICE_NOT_FOUND") => SpotifyError::DeviceUnavailable,
            _ => SpotifyError::Api { status, message },
        },
        429 => SpotifyError::RateLimited {
            retry_after: retry_after.unwrap_or(1),
        },
        _ => SpotifyError::Api { status, message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_401_is_token_expired() {
        assert!(matches!(
            classify_http(401, "", None),
            SpotifyError::TokenExpired
        ));
    }

    #[test]
    fn classify_403_premium_reason() {
        let body = r#"{"error":{"status":403,"message":"Player command failed","reason":"PREMIUM_REQUIRED"}}"#;
        assert!(matches!(
            classify_http(403, body, None),
            SpotifyError::PremiumRequired
        ));
    }

    #[test]
    fn classify_403_premium_keyword() {
        let body = r#"{"error":{"status":403,"message":"Premium account required"}}"#;
        assert!(matches!(
            classify_http(403, body, None),
            SpotifyError::PremiumRequired
        ));
    }

    #[test]
    fn classify_403_restricted_device() {
        let body = r#"{"error":{"status":403,"message":"foo","reason":"DEVICE_NOT_CONTROLLABLE"}}"#;
        assert!(matches!(
            classify_http(403, body, None),
            SpotifyError::RestrictedDevice
        ));
    }

    #[test]
    fn classify_404_no_active_device() {
        let body = r#"{"error":{"status":404,"message":"x","reason":"NO_ACTIVE_DEVICE"}}"#;
        assert!(matches!(
            classify_http(404, body, None),
            SpotifyError::NoActiveDevice
        ));
    }

    #[test]
    fn classify_429_uses_retry_after() {
        match classify_http(429, "", Some(7)) {
            SpotifyError::RateLimited { retry_after } => assert_eq!(retry_after, 7),
            _ => panic!("wrong variant"),
        }
    }
}
