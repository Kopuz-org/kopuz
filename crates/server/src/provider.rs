use config::MusicService;

use crate::jellyfin::JellyfinClient;
use crate::subsonic::SubsonicClient;
use crate::ytmusic::YouTubeMusicClient;

pub fn resolve_subsonic_secret(password: &str) -> Option<String> {
    if password.is_empty() {
        None
    } else {
        Some(password.to_string())
    }
}

pub struct AuthSession {
    pub access_token: String,
    pub user_id: String,
}

pub struct ProviderClient {
    service: MusicService,
    server_url: String,
    device_id: String,
}

impl ProviderClient {
    pub fn new(
        service: MusicService,
        server_url: impl Into<String>,
        device_id: impl Into<String>,
    ) -> Self {
        Self {
            service,
            server_url: server_url.into(),
            device_id: device_id.into(),
        }
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<AuthSession, String> {
        match self.service {
            MusicService::Jellyfin => {
                let mut client = JellyfinClient::new(&self.server_url, None, &self.device_id, None);
                let (access_token, user_id) = client.login(username, password).await?;
                Ok(AuthSession {
                    access_token,
                    user_id,
                })
            }
            MusicService::Subsonic | MusicService::Custom => {
                let client = SubsonicClient::new(&self.server_url, username, password);
                client.ping().await?;
                Ok(AuthSession {
                    access_token: password.to_string(),
                    user_id: username.to_string(),
                })
            }
            MusicService::YtMusic => Err(
                "YouTube Music uses OAuth device flow; call login_ytmusic_device() instead"
                    .to_string(),
            ),
            MusicService::SoundCloud => Err(
                "SoundCloud uses browser sign-in; extract its OAuth token via the sign-in window \
                 instead of username/password login"
                    .to_string(),
            ),
            MusicService::Spotify => Err(
                "Spotify uses OAuth browser sign-in; call spotify::auth::launch_signin_and_extract() \
                 instead of username/password login"
                    .to_string(),
            ),
        }
    }

    pub fn service(&self) -> MusicService {
        self.service
    }
}

/// Whether a YT Music cookie string is still a valid signed-in session. Used by
/// the session-resume flow, which has no resolved source yet (the server may not
/// be the active one), so it can't go through [`MediaSource`](crate::source).
pub async fn validate_ytmusic_cookies(cookies: &str) -> bool {
    YouTubeMusicClient::with_cookies(cookies.to_string())
        .validate_cookies()
        .await
        .is_ok()
}
