use config::MusicService;

use crate::jellyfin::JellyfinClient;
use crate::source::SourceError;
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

    pub async fn login(&self, username: &str, password: &str) -> Result<AuthSession, SourceError> {
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
            MusicService::Clippsly => {
                let client = crate::clippsly::ClippslyClient::new(&self.server_url, password)?;
                let account = client.account().await?;
                Ok(AuthSession {
                    access_token: password.to_string(),
                    user_id: account.id.to_string(),
                })
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
