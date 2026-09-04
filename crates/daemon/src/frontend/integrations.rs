use super::*;

impl FrontendService {
    pub async fn integration_credentials(&self) -> Vec<api::IntegrationCredentialStatus> {
        let config = self.current().await;
        vec![
            api::IntegrationCredentialStatus {
                kind: api::IntegrationKind::ListenBrainz,
                configured: !config.musicbrainz_token.trim().is_empty(),
            },
            api::IntegrationCredentialStatus {
                kind: api::IntegrationKind::LastFm,
                configured: !config.lastfm_api_key.trim().is_empty()
                    && !config.lastfm_api_secret.trim().is_empty()
                    && !config.lastfm_session_key.trim().is_empty(),
            },
            api::IntegrationCredentialStatus {
                kind: api::IntegrationKind::LibreFm,
                configured: !config.librefm_session_key.trim().is_empty(),
            },
        ]
    }

    pub async fn provision_integration(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        let kind = provision.kind;
        self.config.ensure_unlocked(match kind {
            api::IntegrationKind::ListenBrainz => &["musicbrainz_token"],
            api::IntegrationKind::LastFm => {
                &["lastfm_api_key", "lastfm_api_secret", "lastfm_session_key"]
            }
            api::IntegrationKind::LibreFm => &[
                "librefm_api_key",
                "librefm_api_secret",
                "librefm_session_key",
            ],
            api::IntegrationKind::Unknown => &[],
        })?;
        let complete = match kind {
            api::IntegrationKind::ListenBrainz => provision
                .token
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            api::IntegrationKind::LastFm => [
                provision.api_key.as_deref(),
                provision.api_secret.as_deref(),
                provision.session_key.as_deref(),
            ]
            .into_iter()
            .all(|value| value.is_some_and(|value| !value.trim().is_empty())),
            api::IntegrationKind::LibreFm => provision
                .session_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            api::IntegrationKind::Unknown => false,
        };
        if !complete {
            return Err(ApiError::invalid_input(
                "the required credentials were not provided",
            ));
        }
        let changed = match kind {
            api::IntegrationKind::ListenBrainz => vec!["musicbrainz_token".to_string()],
            api::IntegrationKind::LastFm => vec![
                "lastfm_api_key".to_string(),
                "lastfm_api_secret".to_string(),
                "lastfm_session_key".to_string(),
            ],
            api::IntegrationKind::LibreFm => vec![
                "librefm_api_key".to_string(),
                "librefm_api_secret".to_string(),
                "librefm_session_key".to_string(),
            ],
            api::IntegrationKind::Unknown => {
                return Err(ApiError::invalid_input("unknown integration"));
            }
        };
        let updated = self
            .config
            .mutate_state(move |config| match kind {
                api::IntegrationKind::ListenBrainz => {
                    config.musicbrainz_token = provision.token.unwrap_or_default();
                }
                api::IntegrationKind::LastFm => {
                    config.lastfm_api_key = provision.api_key.unwrap_or_default();
                    config.lastfm_api_secret = provision.api_secret.unwrap_or_default();
                    config.lastfm_session_key = provision.session_key.unwrap_or_default();
                }
                api::IntegrationKind::LibreFm => {
                    config.librefm_api_key = provision
                        .api_key
                        .unwrap_or_else(|| scrobble::librefm::API_KEY.to_string());
                    config.librefm_api_secret = provision
                        .api_secret
                        .unwrap_or_else(|| scrobble::librefm::API_SECRET.to_string());
                    config.librefm_session_key = provision.session_key.unwrap_or_default();
                }
                api::IntegrationKind::Unknown => {}
            })
            .await?;
        self.session.set_config(updated, changed);
        let configured = self
            .integration_credentials()
            .await
            .into_iter()
            .find(|status| status.kind == kind)
            .is_some_and(|status| status.configured);
        Ok(api::IntegrationCredentialStatus { kind, configured })
    }

    pub async fn clear_integration(&self, kind: api::IntegrationKind) -> Result<(), ApiError> {
        self.config.ensure_unlocked(match kind {
            api::IntegrationKind::ListenBrainz => &["musicbrainz_token"],
            api::IntegrationKind::LastFm => {
                &["lastfm_api_key", "lastfm_api_secret", "lastfm_session_key"]
            }
            api::IntegrationKind::LibreFm => &[
                "librefm_api_key",
                "librefm_api_secret",
                "librefm_session_key",
            ],
            api::IntegrationKind::Unknown => &[],
        })?;
        let provision = api::IntegrationCredentialProvision {
            kind,
            ..Default::default()
        };
        let changed = match kind {
            api::IntegrationKind::ListenBrainz => vec!["musicbrainz_token".to_string()],
            api::IntegrationKind::LastFm => vec![
                "lastfm_api_key".to_string(),
                "lastfm_api_secret".to_string(),
                "lastfm_session_key".to_string(),
            ],
            api::IntegrationKind::LibreFm => vec![
                "librefm_api_key".to_string(),
                "librefm_api_secret".to_string(),
                "librefm_session_key".to_string(),
            ],
            api::IntegrationKind::Unknown => {
                return Err(ApiError::invalid_input("unknown integration"));
            }
        };
        let updated = self
            .config
            .mutate_state(move |config| match provision.kind {
                api::IntegrationKind::ListenBrainz => config.musicbrainz_token.clear(),
                api::IntegrationKind::LastFm => {
                    config.lastfm_api_key.clear();
                    config.lastfm_api_secret.clear();
                    config.lastfm_session_key.clear();
                }
                api::IntegrationKind::LibreFm => {
                    config.librefm_api_key.clear();
                    config.librefm_api_secret.clear();
                    config.librefm_session_key.clear();
                }
                api::IntegrationKind::Unknown => {}
            })
            .await?;
        self.session.set_config(updated, changed);
        Ok(())
    }

    #[cfg(target_os = "android")]
    pub async fn authenticate_integration(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        let _ = provision;
        Err(ApiError::unsupported(
            "daemon-owned browser authentication is unavailable on Android",
        ))
    }

    #[cfg(not(target_os = "android"))]
    pub async fn authenticate_integration(
        &self,
        mut provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        self.config.ensure_unlocked(match provision.kind {
            api::IntegrationKind::ListenBrainz => &["musicbrainz_token"],
            api::IntegrationKind::LastFm => {
                &["lastfm_api_key", "lastfm_api_secret", "lastfm_session_key"]
            }
            api::IntegrationKind::LibreFm => &[
                "librefm_api_key",
                "librefm_api_secret",
                "librefm_session_key",
            ],
            api::IntegrationKind::Unknown => &[],
        })?;
        let (api_key, api_secret, token, url) = match provision.kind {
            api::IntegrationKind::LastFm => {
                let key = provision
                    .api_key
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| ApiError::invalid_input("Last.fm API key is required"))?;
                let secret = provision
                    .api_secret
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| ApiError::invalid_input("Last.fm API secret is required"))?;
                let token = scrobble::lastfm::get_auth_token(&key)
                    .await
                    .map_err(|error| ApiError::internal(format!("Last.fm auth failed: {error}")))?;
                let url = scrobble::lastfm::auth_url(&key, &token);
                (key, secret, token, url)
            }
            api::IntegrationKind::LibreFm => {
                let key = scrobble::librefm::API_KEY.to_string();
                let secret = scrobble::librefm::API_SECRET.to_string();
                let token = scrobble::librefm::get_auth_token(&key)
                    .await
                    .map_err(|error| {
                        ApiError::internal(format!("Libre.fm auth failed: {error}"))
                    })?;
                let url = scrobble::librefm::auth_url(&key, &token);
                (key, secret, token, url)
            }
            api::IntegrationKind::ListenBrainz => {
                return self.provision_integration(provision).await;
            }
            api::IntegrationKind::Unknown => {
                return Err(ApiError::invalid_input("unknown integration"));
            }
        };
        webbrowser::open(&url)
            .map_err(|error| ApiError::internal(format!("could not open browser: {error}")))?;
        let mut session_key = None;
        for _ in 0..150 {
            let result = match provision.kind {
                api::IntegrationKind::LastFm => {
                    scrobble::lastfm::get_session_key(&api_key, &api_secret, &token).await
                }
                api::IntegrationKind::LibreFm => {
                    scrobble::librefm::get_session_key(&api_key, &api_secret, &token).await
                }
                _ => return Err(ApiError::invalid_input("unknown integration")),
            };
            if let Ok(key) = result {
                session_key = Some(key);
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        provision.api_key = Some(api_key);
        provision.api_secret = Some(api_secret);
        provision.session_key = Some(session_key.ok_or_else(|| {
            ApiError::new(ErrorCode::SourceAuthExpired, "authorization timed out")
        })?);
        self.provision_integration(provision).await
    }
}
