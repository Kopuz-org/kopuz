use config::MusicService;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::jellyfin::JellyfinClient;
use crate::subsonic::SubsonicClient;

static SUBSONIC_SESSION_SECRETS: LazyLock<Mutex<HashMap<String, (String, Instant)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const SUBSONIC_SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24);

pub fn resolve_subsonic_secret(token_or_password: &str) -> Option<String> {
    if let Ok(mut store) = SUBSONIC_SESSION_SECRETS.lock() {
        if let Some((secret, created_at)) = store.get(token_or_password).cloned() {
            if created_at.elapsed() <= SUBSONIC_SESSION_TTL {
                return Some(secret);
            }
            store.remove(token_or_password);
            return None;
        }
    }

    // Raw password can be used directly only for non-session-token inputs (e.g. fresh login).
    if uuid::Uuid::parse_str(token_or_password).is_ok() {
        None
    } else {
        Some(token_or_password.to_string())
    }
}

fn store_subsonic_secret(session_token: &str, password: &str) {
    if let Ok(mut store) = SUBSONIC_SESSION_SECRETS.lock() {
        store.insert(
            session_token.to_string(),
            (password.to_string(), Instant::now()),
        );
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
                let session_token = uuid::Uuid::new_v4().to_string();
                store_subsonic_secret(&session_token, password);
                Ok(AuthSession {
                    access_token: session_token,
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
