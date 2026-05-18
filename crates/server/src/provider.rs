use config::MusicService;

use crate::jellyfin::JellyfinClient;
use crate::subsonic::SubsonicClient;

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
        }
    }

    pub fn make_jellyfin_client(&self, access_token: &str, user_id: &str) -> JellyfinClient {
        JellyfinClient::new(
            &self.server_url,
            Some(access_token),
            &self.device_id,
            Some(user_id),
        )
    }

    pub fn make_subsonic_client(&self, username: &str, password: &str) -> SubsonicClient {
        SubsonicClient::new(&self.server_url, username, password)
    }

    pub fn service(&self) -> MusicService {
        self.service
    }
}
