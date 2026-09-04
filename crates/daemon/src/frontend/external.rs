use super::*;

impl FrontendService {
    pub async fn external_access(&self, kind: &str) -> Result<api::ExternalAccess, ApiError> {
        if kind != "spotify" {
            return Err(ApiError::unsupported("unsupported external playback kind"));
        }
        let config = self.current().await;
        let server = config
            .server
            .filter(|server| server.service == config::MusicService::Spotify)
            .ok_or_else(|| ApiError::not_found("Spotify is not the active source"))?;
        let packed = server.access_token.ok_or_else(|| {
            ApiError::new(ErrorCode::SourceAuthExpired, "Spotify is not signed in")
        })?;
        let client_id = server.url.clone();
        let (access_token, refresh_token) = server::spotify::auth::unpack_token(&packed);
        if refresh_token.is_empty() {
            return Ok(api::ExternalAccess {
                kind: kind.to_string(),
                access_token,
                client_id: Some(client_id),
            });
        }
        let refreshed = server::spotify::auth::refresh_packed(&packed, client_id.clone())
            .await
            .map_err(|error| {
                ApiError::new(
                    ErrorCode::SourceAuthExpired,
                    format!("Spotify credential refresh failed: {error}"),
                )
            })?;
        let server_id = server
            .id
            .ok_or_else(|| ApiError::internal("active Spotify server has no id"))?;
        self.db
            .set_server_credentials(&server_id, Some(&refreshed), server.user_id.as_deref())
            .await
            .map_err(Self::db_error)?;
        let expected = packed;
        let refreshed_for_config = refreshed.clone();
        let updated = self
            .config
            .mutate_state(move |config| {
                if config.active_source.server_id() == Some(server_id.as_str())
                    && let Some(server) = config.server.as_mut()
                    && server.access_token.as_deref() == Some(expected.as_str())
                {
                    server.access_token = Some(refreshed_for_config);
                }
            })
            .await?;
        self.session
            .set_config(updated, vec!["servers".to_string()]);
        let (access_token, _) = server::spotify::auth::unpack_token(&refreshed);
        Ok(api::ExternalAccess {
            kind: kind.to_string(),
            access_token,
            client_id: Some(client_id),
        })
    }

    pub fn set_external(&self, external: Option<api::ExternalPlayback>) {
        self.session.set_external(external);
    }

    pub async fn claim_external(
        &self,
        external: api::ExternalPlayback,
    ) -> Result<api::ExternalPlaybackLease, ApiError> {
        if external.kind.trim().is_empty() {
            return Err(ApiError::invalid_input(
                "external playback kind is required",
            ));
        }
        if external.kind != "spotify" {
            return Err(ApiError::unsupported("unsupported external playback kind"));
        }
        if !self.current().await.server.is_some_and(|server| {
            server.service == config::MusicService::Spotify
                && server
                    .access_token
                    .as_deref()
                    .is_some_and(|token| !token.is_empty())
        }) {
            return Err(ApiError::new(
                ErrorCode::SourceAuthExpired,
                "Spotify must be the authenticated active source",
            ));
        }
        let mut lease = self.external_lease.lock().await;
        if lease
            .as_ref()
            .is_some_and(|current| current.expires_at > Instant::now())
        {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback is owned by another frontend",
            ));
        }
        if lease.take().is_some() {
            self.session.set_external(None);
        }
        let lease_id = uuid::Uuid::new_v4().to_string();
        *lease = Some(ExternalLease {
            id: lease_id.clone(),
            expires_at: Instant::now() + EXTERNAL_LEASE_TTL,
        });
        drop(lease);
        self.session.set_external(Some(external));
        self.spawn_external_expiry(lease_id.clone());
        Ok(api::ExternalPlaybackLease {
            lease_id,
            expires_in_ms: EXTERNAL_LEASE_TTL.as_millis() as u64,
        })
    }

    fn spawn_external_expiry(&self, lease_id: String) {
        let external_lease = self.external_lease.clone();
        let session = self.session.clone();
        tokio::spawn(async move {
            loop {
                let deadline = {
                    let lease = external_lease.lock().await;
                    lease
                        .as_ref()
                        .filter(|current| current.id == lease_id)
                        .map(|current| current.expires_at)
                };
                let Some(deadline) = deadline else {
                    return;
                };
                tokio::time::sleep_until(deadline.into()).await;
                let mut lease = external_lease.lock().await;
                let expired = lease.as_ref().is_some_and(|current| {
                    current.id == lease_id && current.expires_at <= Instant::now()
                });
                if expired {
                    *lease = None;
                    drop(lease);
                    session.set_external(None);
                    tracing::info!("external playback lease expired");
                    return;
                }
            }
        });
    }

    pub async fn report_external(
        &self,
        report: api::ExternalPlaybackReport,
    ) -> Result<(), ApiError> {
        let mut lease = self.external_lease.lock().await;
        let Some(current) = lease.as_mut() else {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback is not claimed",
            ));
        };
        if current.expires_at <= Instant::now() {
            *lease = None;
            drop(lease);
            self.session.set_external(None);
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback lease expired",
            ));
        }
        if current.id != report.lease_id {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback lease does not match",
            ));
        }
        current.expires_at = Instant::now() + EXTERNAL_LEASE_TTL;
        drop(lease);
        let track = report
            .track
            .as_ref()
            .map(|value| -> Result<reader::Track, ApiError> {
                let decoded = LibraryService::track_from_info(value)?;
                Ok(self
                    .library
                    .transient_track_for_info(value)
                    .unwrap_or(decoded))
            })
            .transpose()?;
        if track
            .as_ref()
            .is_some_and(|track| track.id.service() != Some(config::MusicService::Spotify))
        {
            return Err(ApiError::invalid_input(
                "external Spotify playback requires a Spotify track",
            ));
        }
        if let Some(track) = track.as_ref() {
            self.library.register_transient(std::slice::from_ref(track));
        }
        self.session
            .report_external(
                track,
                report.position_ms,
                report.playing,
                report.completed,
                report.device,
            )
            .await
    }

    pub async fn release_external(&self, lease_id: &str) -> Result<(), ApiError> {
        let mut lease = self.external_lease.lock().await;
        let Some(current) = lease.as_ref() else {
            return Ok(());
        };
        if current.id != lease_id {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "external playback lease does not match",
            ));
        }
        *lease = None;
        drop(lease);
        self.session.set_external(None);
        Ok(())
    }
}
