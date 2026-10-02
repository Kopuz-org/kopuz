//! The librespot session: one live connection to a Spotify access point,
//! shared by everything in the process that speaks Spotify.
//!
//! A session is expensive to stand up (access-point resolve, key exchange,
//! login) and cannot be reused once its connection drops, so the process
//! keeps exactly one per stored credential and rebuilds it on demand. The
//! `MediaSource` runs its catalogue reads on it; the decode worker asks it
//! for audio keys and file streams.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::error::ErrorKind;
use librespot_core::{Error as LibrespotError, Session, SessionConfig};

/// How much decrypted-on-demand audio librespot may keep on disk between
/// runs, so a replay does not fetch the file again.
const AUDIO_CACHE_LIMIT: u64 = 1024 * 1024 * 1024;

/// Why the session could not do something. The distinction matters to
/// `validate`: a refused login is "sign in again", everything else is "try
/// later".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    Unauthenticated(String),
    Unavailable(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthenticated(message) | Self::Unavailable(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<LibrespotError> for SessionError {
    fn from(error: LibrespotError) -> Self {
        match error.kind {
            ErrorKind::Unauthenticated | ErrorKind::PermissionDenied => {
                Self::Unauthenticated(error.to_string())
            }
            _ => Self::Unavailable(error.to_string()),
        }
    }
}

/// Credentials as the config stores them: the reusable blob a login hands
/// back, which never expires on its own. This is what the `access_token`
/// column holds for a Spotify server.
pub fn pack_credentials(credentials: &Credentials) -> Result<String, SessionError> {
    serde_json::to_string(credentials)
        .map_err(|error| SessionError::Unavailable(format!("credential encode: {error}")))
}

pub fn unpack_credentials(stored: &str) -> Result<Credentials, SessionError> {
    serde_json::from_str::<Credentials>(stored).map_err(|_| {
        SessionError::Unauthenticated(
            "the stored Spotify credentials are from an older kopuz; sign in again".to_string(),
        )
    })
}

/// The reusable credentials a connected session was given.
pub fn reusable_credentials(session: &Session) -> Credentials {
    use librespot_protocol::authentication::AuthenticationType;
    Credentials {
        username: Some(session.username()),
        auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
        auth_data: session.auth_data(),
    }
}

/// One account's session, connected lazily and reconnected after a drop.
pub struct SpotifySession {
    stored: String,
    device_id: String,
    live: tokio::sync::Mutex<Option<Session>>,
}

impl SpotifySession {
    fn new(stored: String, device_id: String) -> Self {
        Self {
            stored,
            device_id,
            live: tokio::sync::Mutex::new(None),
        }
    }

    /// The connected session, connecting first when there is none or the
    /// last one lost its socket.
    pub async fn session(&self) -> Result<Session, SessionError> {
        let mut live = self.live.lock().await;
        if let Some(session) = live.as_ref().filter(|session| !session.is_invalid()) {
            return Ok(session.clone());
        }
        let credentials = unpack_credentials(&self.stored)?;
        let session = connect(credentials, &self.device_id).await?;
        *live = Some(session.clone());
        Ok(session)
    }

    /// The reusable credentials, once connected. What sign-in stores.
    pub async fn credentials(&self) -> Result<Credentials, SessionError> {
        Ok(reusable_credentials(&self.session().await?))
    }

    /// Take over a session that is already logged in, so the handshake a
    /// sign-in just did is not repeated for the first track.
    pub async fn adopt(&self, session: Session) {
        *self.live.lock().await = Some(session);
    }
}

/// Log in with `credentials` on a fresh session.
pub async fn connect(credentials: Credentials, device_id: &str) -> Result<Session, SessionError> {
    let session = Session::new(session_config(device_id), audio_cache());
    session.connect(credentials, false).await?;
    Ok(session)
}

fn session_config(device_id: &str) -> SessionConfig {
    let mut config = SessionConfig::default();
    if !device_id.trim().is_empty() {
        config.device_id = device_id.to_string();
    }
    if let Some(dir) = cache_root() {
        let tmp = dir.join("spotify-tmp");
        if std::fs::create_dir_all(&tmp).is_ok() {
            config.tmp_dir = tmp;
        }
    }
    config
}

fn cache_root() -> Option<PathBuf> {
    directories::ProjectDirs::from("moe", "kopuz", "kopuz").map(|dirs| dirs.cache_dir().to_owned())
}

/// Audio-only cache: credentials live in the config, not in librespot's
/// `credentials.json`, so removing the server forgets the account.
fn audio_cache() -> Option<Cache> {
    let audio = cache_root()?.join("spotify-audio");
    match Cache::new::<PathBuf>(None, None, Some(audio), Some(AUDIO_CACHE_LIMIT)) {
        Ok(cache) => Some(cache),
        Err(error) => {
            tracing::debug!(%error, "spotify audio cache unavailable; streaming without one");
            None
        }
    }
}

static SHARED: OnceLock<Mutex<Option<Arc<SpotifySession>>>> = OnceLock::new();

fn registry() -> &'static Mutex<Option<Arc<SpotifySession>>> {
    SHARED.get_or_init(|| Mutex::new(None))
}

/// The process-wide session for `stored`. A different credential replaces
/// the one held (the account changed), the same one is shared, so a source
/// rebuilt on a config write keeps its live connection. The replaced
/// session's tasks only hold weak references, so dropping it ends them.
pub fn shared(stored: &str, device_id: &str) -> Arc<SpotifySession> {
    let mut slot = match registry().lock() {
        Ok(slot) => slot,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(current) = slot.as_ref().filter(|current| current.stored == stored) {
        return current.clone();
    }
    let session = Arc::new(SpotifySession::new(
        stored.to_string(),
        device_id.to_string(),
    ));
    *slot = Some(session.clone());
    session
}

/// The session most recently asked for, which is the signed-in account's.
/// The decode worker reaches the session this way: a stream ref names only
/// the track, never the credential.
pub fn current() -> Option<Arc<SpotifySession>> {
    match registry().lock() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Forget the shared session, e.g. when the Spotify server is removed.
pub fn clear() {
    match registry().lock() {
        Ok(mut slot) => slot.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_roundtrip_through_the_config_column() {
        use librespot_protocol::authentication::AuthenticationType;
        let credentials = Credentials {
            username: Some("someone".into()),
            auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
            auth_data: vec![1, 2, 3, 250],
        };
        let packed = pack_credentials(&credentials).expect("pack");
        assert_eq!(unpack_credentials(&packed).expect("unpack"), credentials);
    }

    #[test]
    fn a_legacy_oauth_pack_is_a_sign_in_again() {
        assert!(matches!(
            unpack_credentials("access\nrefresh"),
            Err(SessionError::Unauthenticated(_))
        ));
    }
}
