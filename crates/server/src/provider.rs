use config::MusicService;

use crate::source::SourceError;
use crate::ytmusic::YouTubeMusicClient;

pub struct AuthSession {
    pub access_token: String,
    pub user_id: String,
}

pub struct ProviderClient {
    service: MusicService,
    /// The plugin's own slug, for a `MusicService::Plugin` server.
    plugin_service: Option<String>,
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
            plugin_service: None,
            server_url: server_url.into(),
            device_id: device_id.into(),
        }
    }

    /// Sign in to a plugin's own service rather than the built-in one.
    pub fn with_plugin_service(mut self, slug: Option<String>) -> Self {
        self.plugin_service = slug;
        self
    }

    fn service_id(&self) -> &str {
        match (self.service, self.plugin_service.as_deref()) {
            (MusicService::Plugin, Some(slug)) => slug,
            (service, _) => service.id(),
        }
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<AuthSession, SourceError> {
        if let Some(plugin) = crate::plugins::registry().for_service(self.service_id()) {
            let connection = plugin_host::wit::Connection {
                service: self.service_id().to_string(),
                url: self.server_url.clone(),
                token: String::new(),
                user_id: String::new(),
                device_id: self.device_id.clone(),
                account: None,
                folders: Vec::new(),
            };
            let session = plugin
                .login(connection, username, password)
                .await
                .map_err(crate::source::plugin_error)?;
            return Ok(AuthSession {
                access_token: session.access_token,
                user_id: session.user_id,
            });
        }
        match self.service {
            MusicService::Jellyfin
            | MusicService::Subsonic
            | MusicService::Custom
            | MusicService::Clippsly
            | MusicService::Plugin => {
                Err(SourceError::Backend("no plugin serves this service".into()))
            }
            MusicService::Nextcloud => {
                // The password is expected to be an app password (Settings,
                // Security), which is revocable and survives 2FA.
                let client =
                    crate::nextcloud::NextcloudClient::new(&self.server_url, username, password)
                        .map_err(SourceError::InvalidInput)?;
                client
                    .ping()
                    .await
                    .map_err(crate::nextcloud::source_error)?;
                Ok(AuthSession {
                    access_token: password.to_string(),
                    user_id: username.to_string(),
                })
            }
            MusicService::Smb => {
                crate::smb::login(&self.server_url, username, password).await?;
                Ok(AuthSession {
                    access_token: password.to_string(),
                    user_id: username.to_string(),
                })
            }
            MusicService::YtMusic
            | MusicService::SoundCloud
            | MusicService::AppleMusic
            | MusicService::Spotify => {
                Err(SourceError::unsupported("username and password sign-in"))
            }
        }
    }

    pub fn service(&self) -> MusicService {
        self.service
    }
}

/// What YouTube says about a YT Music cookie string. Used by the
/// session-resume flow, which has no resolved source yet (the server may not
/// be the active one), so it can't go through [`MediaSource`](crate::source).
pub async fn check_ytmusic_session(cookies: &str) -> crate::ytmusic::SessionCheck {
    YouTubeMusicClient::with_cookies(cookies.to_string())
        .check_session()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn answering(status: &'static str) -> String {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = server.local_addr().expect("address");
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok((mut stream, _)) = server.accept().await {
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request).await;
                let reply =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{address}")
    }

    fn jellyfin(url: String) -> ProviderClient {
        ProviderClient::new(MusicService::Jellyfin, url, "test-device")
    }

    #[tokio::test]
    async fn a_refused_login_is_an_auth_failure_and_a_missing_server_is_not() {
        let refused = jellyfin(answering("401 Unauthorized").await)
            .login("someone", "hunter2")
            .await;
        assert_eq!(refused.err(), Some(SourceError::Auth));

        let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = closed.local_addr().expect("address");
        drop(closed);
        let gone = jellyfin(format!("http://{address}"))
            .login("someone", "hunter2")
            .await;
        assert_eq!(gone.err(), Some(SourceError::Connectivity));

        let broken = jellyfin(answering("500 Internal Server Error").await)
            .login("someone", "hunter2")
            .await;
        assert!(matches!(broken.err(), Some(SourceError::Backend(_))));
    }
}
