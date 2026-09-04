use super::*;
use server::source::AuthOutcome;

impl FrontendService {
    pub async fn sources(&self) -> Result<Vec<api::SourceInfo>, ApiError> {
        let config = self.current().await;
        let mut ids = Vec::with_capacity(config.local_sources.len() + config.servers.len() + 1);
        ids.push("local".to_string());
        ids.extend(config.local_sources.iter().map(|source| source.id.clone()));
        ids.extend(config.servers.iter().map(|server| server.id.clone()));
        let mut sources = Vec::with_capacity(ids.len());
        for id in ids {
            sources.push(self.source_info_for(&id).await?);
        }
        Ok(sources)
    }

    pub async fn switch_source(&self, id: &str) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["active_source", "server"])?;
        let previous_source = self.current().await.active_source;
        let (target, _) = self.source_for(id).await?;
        let source = target.active_source.clone();
        let source_changed = previous_source != source;
        let updated = self
            .config
            .mutate_state(move |config| match source {
                config::Source::Local | config::Source::LocalLibrary(_) => {
                    config.set_active_local_source(source)
                }
                config::Source::Server(_) => {
                    if let Some(server) = target.server {
                        config.set_active_server_snapshot(server);
                    }
                }
            })
            .await?;
        if source_changed {
            self.finish_source_change(updated, vec!["active_source".to_string()])
                .await?;
        } else {
            self.publish_config(updated, vec!["active_source".to_string()])
                .await;
        }
        self.source_info_for(id).await
    }

    pub async fn upsert_local_source(
        &self,
        draft: api::LocalSourceDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["local_sources"])?;
        let name = draft.name.trim();
        if name.is_empty() {
            return Err(ApiError::invalid_input("local source name is required"));
        }
        if draft.directories.is_empty()
            || draft.directories.iter().any(|path| path.trim().is_empty())
        {
            return Err(ApiError::invalid_input(
                "at least one local source directory is required",
            ));
        }
        let id = draft
            .id
            .unwrap_or_else(|| format!("local:{}", uuid::Uuid::new_v4()));
        if !id.starts_with("local:") {
            return Err(ApiError::invalid_input("invalid local source id"));
        }
        let saved = config::SavedLocalSource {
            id: id.clone(),
            name: name.to_string(),
            directories: draft.directories.into_iter().map(PathBuf::from).collect(),
        };
        let updated = self
            .config
            .mutate_state({
                let saved = saved.clone();
                move |config| {
                    if let Some(existing) = config
                        .local_sources
                        .iter_mut()
                        .find(|source| source.id == saved.id)
                    {
                        *existing = saved.clone();
                    } else {
                        config.local_sources.push(saved.clone());
                    }
                }
            })
            .await?;
        self.publish_config(updated, vec!["local_sources".to_string()])
            .await;
        self.library.invalidate(api::Table::Servers);
        self.source_info_for(&id).await
    }

    pub async fn delete_local_source(&self, id: &str) -> Result<(), ApiError> {
        self.config
            .ensure_unlocked(&["active_source", "local_sources"])?;
        if id == "local" {
            return Err(ApiError::invalid_input(
                "the default local source cannot be deleted",
            ));
        }
        let current = self.current().await;
        if !current.local_sources.iter().any(|source| source.id == id) {
            return Err(ApiError::not_found("local source not found"));
        }
        let was_active = current.active_source.local_library_id() == Some(id);
        let id = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| config.remove_local_source(&id))
            .await?;
        if was_active {
            self.finish_source_change(
                updated,
                vec!["local_sources".to_string(), "active_source".to_string()],
            )
            .await?;
        } else {
            self.publish_config(updated, vec!["local_sources".to_string()])
                .await;
            self.library.invalidate(api::Table::Servers);
        }
        Ok(())
    }

    pub async fn set_source_directories(
        &self,
        id: &str,
        directories: Vec<String>,
    ) -> Result<api::SourceInfo, ApiError> {
        if directories.iter().any(|path| path.trim().is_empty()) {
            return Err(ApiError::invalid_input("source directory is empty"));
        }
        let source = config::Source::from_column(id);
        let changed = match &source {
            config::Source::Local => "music_directory",
            config::Source::LocalLibrary(_) => "local_sources",
            config::Source::Server(_) => "server_folders",
        };
        self.config.ensure_unlocked(&[changed])?;
        let current = self.current().await;
        if let config::Source::LocalLibrary(local_id) = &source
            && !current
                .local_sources
                .iter()
                .any(|saved| saved.id == *local_id)
        {
            return Err(ApiError::not_found("local source not found"));
        }
        if let config::Source::Server(server_id) = &source
            && !current.servers.iter().any(|saved| saved.id == *server_id)
        {
            return Err(ApiError::not_found("server not found"));
        }
        let id_owned = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| match source {
                config::Source::Local => {
                    config.music_directory = directories.into_iter().map(PathBuf::from).collect();
                }
                config::Source::LocalLibrary(local_id) => {
                    if let Some(saved) = config
                        .local_sources
                        .iter_mut()
                        .find(|saved| saved.id == local_id)
                    {
                        saved.directories = directories.into_iter().map(PathBuf::from).collect();
                    }
                }
                config::Source::Server(server_id) => {
                    config.set_folders_for(&server_id, directories);
                }
            })
            .await?;
        self.publish_config(updated, vec![changed.to_string()])
            .await;
        self.library.invalidate(api::Table::Servers);
        self.source_info_for(&id_owned).await
    }

    pub async fn upsert_server(
        &self,
        draft: api::ServerDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        let service = crate::wire::music_service_from_api(draft.service)
            .ok_or_else(|| ApiError::invalid_input("unknown music service"))?;
        if draft.name.trim().is_empty() {
            return Err(ApiError::invalid_input("server name is required"));
        }
        if !service.uses_browser_signin() && !draft.url.starts_with("http") {
            return Err(ApiError::invalid_input("server URL must use HTTP or HTTPS"));
        }
        let id = draft.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Self::validate_server_id(&id)?;
        let browser = match draft.browser.as_deref() {
            Some(browser) => Some(
                config::Browser::from_id(browser)
                    .ok_or_else(|| ApiError::invalid_input("unknown browser"))?,
            ),
            None => None,
        };
        let saved = config::SavedServer {
            id: id.clone(),
            name: draft.name.trim().to_string(),
            url: draft.url.trim_end_matches('/').to_string(),
            service,
            yt_browser: browser,
            yt_anonymous: draft.anonymous,
            apple_music_storefront: draft.storefront.unwrap_or_else(|| "us".to_string()),
            apple_music_language: draft.language.unwrap_or_else(|| "en".to_string()),
        };
        let current = self.current().await;
        let backend_changed = current.active_source.server_id() == Some(id.as_str())
            && current
                .server
                .as_ref()
                .is_some_and(|server| server.service != saved.service || server.url != saved.url);
        let updated = self
            .config
            .mutate_state({
                let saved = saved.clone();
                move |config| {
                    if let Some(existing) = config
                        .servers
                        .iter_mut()
                        .find(|server| server.id == saved.id)
                    {
                        *existing = saved.clone();
                    } else {
                        config.servers.push(saved.clone());
                    }
                    if config.active_source.server_id() == Some(saved.id.as_str())
                        && let Some(server) = config.server.as_mut()
                    {
                        server.name.clone_from(&saved.name);
                        server.url.clone_from(&saved.url);
                        server.service = saved.service;
                        server.yt_browser = saved.yt_browser;
                        server.yt_anonymous = saved.yt_anonymous;
                        server
                            .apple_music_storefront
                            .clone_from(&saved.apple_music_storefront);
                        server
                            .apple_music_language
                            .clone_from(&saved.apple_music_language);
                    }
                }
            })
            .await?;
        self.publish_config(updated, vec!["servers".to_string()])
            .await;
        if backend_changed {
            self.reset_playback().await?;
        }
        self.library.invalidate(api::Table::Servers);
        self.source_info_for(&id).await
    }

    pub async fn delete_server(&self, id: &str) -> Result<(), ApiError> {
        self.config
            .ensure_unlocked(&["active_source", "server", "servers"])?;
        let current = self.current().await;
        let service = current
            .servers
            .iter()
            .find(|server| server.id == id)
            .map(|server| server.service);
        let was_active = current.active_source.server_id() == Some(id);
        let id_owned = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| {
                config.remove_saved_server(&id_owned);
                if was_active {
                    config.clear_active_server();
                }
            })
            .await?;
        self.publish_config(
            updated,
            vec!["servers".to_string(), "active_source".to_string()],
        )
        .await;
        if was_active {
            self.reset_playback().await?;
        }
        self.library.invalidate(api::Table::Servers);
        match service {
            Some(config::MusicService::YtMusic) => {
                let _ = server::ytmusic::isolated_profile::delete_profile(id);
            }
            Some(config::MusicService::SoundCloud) => {
                let _ = server::soundcloud::signin::delete_profile(id);
            }
            Some(config::MusicService::AppleMusic) => {
                let _ = server::applemusic::signin::delete_profile(id);
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn provision(
        &self,
        provision: api::CredentialProvision,
    ) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        if provision.secret.is_empty() {
            return Err(ApiError::invalid_input("credential is empty"));
        }
        let mut server = self
            .db
            .load_server(&provision.server_id)
            .await
            .map_err(Self::db_error)?
            .ok_or_else(|| ApiError::not_found("server not found"))?;
        let previous_user_id = server.user_id.clone();
        server.access_token = Some(provision.secret);
        server.user_id = provision.user_id;
        if let Some(browser) = provision.browser.as_deref() {
            server.yt_browser = Some(
                config::Browser::from_id(browser)
                    .ok_or_else(|| ApiError::invalid_input("unknown browser"))?,
            );
        }
        let active = self
            .current()
            .await
            .active_source
            .server_id()
            .is_some_and(|id| id == provision.server_id);
        let access_token = server.access_token.clone();
        let user_id = server.user_id.clone();
        let saved = config::SavedServer::from_music_server(&server);
        let live_server = server.clone();
        let updated = self
            .config
            .mutate_state(move |config| {
                if let Some(existing) = config.servers.iter_mut().find(|entry| entry.id == saved.id)
                {
                    *existing = saved;
                } else {
                    config.servers.push(saved);
                }
                if active {
                    config.server = Some(live_server);
                }
            })
            .await?;
        self.db
            .set_server_credentials(
                &provision.server_id,
                access_token.as_deref(),
                user_id.as_deref(),
            )
            .await
            .map_err(Self::db_error)?;
        if active {
            self.publish_config(updated, vec!["servers".to_string()])
                .await;
            if previous_user_id != server.user_id {
                self.reset_playback().await?;
            }
        } else {
            self.session
                .set_config(updated, vec!["servers".to_string()]);
        }
        self.library.invalidate(api::Table::Servers);
        self.source_info_for(&provision.server_id).await
    }

    pub async fn login_source(
        &self,
        request: api::SourceLoginRequest,
    ) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        if request.username.trim().is_empty() || request.password.is_empty() {
            return Err(ApiError::invalid_input(
                "source username and password are required",
            ));
        }
        let current = self.current().await;
        let server = self
            .db
            .load_server(&request.server_id)
            .await
            .map_err(Self::db_error)?
            .ok_or_else(|| ApiError::not_found("server not found"))?;
        let auth =
            server::provider::ProviderClient::new(server.service, server.url, current.device_id)
                .login(request.username.trim(), &request.password)
                .await
                .map_err(|error| ApiError::new(ErrorCode::Unauthorized, error))?;
        self.provision(api::CredentialProvision {
            server_id: request.server_id,
            secret: auth.access_token,
            user_id: Some(auth.user_id),
            browser: None,
        })
        .await
    }

    pub async fn clear_credentials(&self, id: &str) -> Result<(), ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        let mut server = self
            .db
            .load_server(id)
            .await
            .map_err(Self::db_error)?
            .ok_or_else(|| ApiError::not_found("server not found"))?;
        let had_credentials = server.access_token.is_some() || server.user_id.is_some();
        server.access_token = None;
        server.user_id = None;
        let active = self
            .current()
            .await
            .active_source
            .server_id()
            .is_some_and(|server_id| server_id == id);
        self.db
            .set_server_credentials(id, None, None)
            .await
            .map_err(Self::db_error)?;
        if active {
            let updated = self
                .config
                .mutate_state(move |config| config.server = Some(server))
                .await?;
            self.publish_config(updated, vec!["servers".to_string()])
                .await;
            if had_credentials {
                self.reset_playback().await?;
            }
        }
        self.library.invalidate(api::Table::Servers);
        Ok(())
    }

    pub async fn authenticate_source(&self, id: &str) -> Result<api::SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        #[cfg(target_os = "android")]
        {
            let _ = id;
            return Err(ApiError::unsupported(
                "daemon-owned browser authentication is unavailable on Android",
            ));
        }
        #[cfg(not(target_os = "android"))]
        {
            let server = self
                .db
                .load_server(id)
                .await
                .map_err(Self::db_error)?
                .ok_or_else(|| ApiError::not_found("server not found"))?;
            let browser = server.yt_browser.unwrap_or(config::Browser::Chrome);
            let (secret, user_id) = match server.service {
                config::MusicService::YtMusic => {
                    let secret = ensure_ytmusic_signed_in(server.access_token.clone(), browser, id)
                        .await
                        .map_err(ApiError::internal)?;
                    let user_id = server::ytmusic::derive_user_id(&secret)
                        .unwrap_or_else(|| "me".to_string());
                    (secret, user_id)
                }
                config::MusicService::SoundCloud => {
                    let secret = server::soundcloud::signin::launch_signin_and_extract(
                        browser,
                        id,
                        Duration::from_secs(300),
                    )
                    .await
                    .map_err(ApiError::internal)?;
                    let user_id = server::soundcloud::derive_user_id(&secret)
                        .await
                        .unwrap_or_else(|| "me".to_string());
                    (secret, user_id)
                }
                config::MusicService::AppleMusic => {
                    let secret = server::applemusic::signin::launch_signin_and_extract(
                        browser,
                        id,
                        Duration::from_secs(300),
                    )
                    .await
                    .map_err(ApiError::internal)?;
                    (secret, "me".to_string())
                }
                config::MusicService::Spotify => {
                    let auth = server::spotify::auth::launch_signin_and_extract(server.url)
                        .await
                        .map_err(ApiError::internal)?;
                    (
                        server::spotify::auth::pack_token(&auth.access_token, &auth.refresh_token),
                        auth.user_id,
                    )
                }
                _ => {
                    return Err(ApiError::unsupported(
                        "this source uses explicit credential provisioning",
                    ));
                }
            };
            self.provision(api::CredentialProvision {
                server_id: id.to_string(),
                secret,
                user_id: Some(user_id),
                browser: Some(browser.id().to_string()),
            })
            .await
        }
    }

    pub async fn browse_source(
        &self,
        id: &str,
        path: &str,
    ) -> Result<Vec<api::SourceFolderEntry>, ApiError> {
        let server = self
            .db
            .load_server(id)
            .await
            .map_err(Self::db_error)?
            .ok_or_else(|| ApiError::not_found("server not found"))?;
        if server.service != config::MusicService::Nextcloud {
            return Err(ApiError::unsupported(
                "folder browsing is only available for Nextcloud sources",
            ));
        }
        let user_id = server
            .user_id
            .as_deref()
            .ok_or_else(|| ApiError::new(ErrorCode::SourceAuthExpired, "missing user id"))?;
        let secret = server
            .access_token
            .as_deref()
            .ok_or_else(|| ApiError::new(ErrorCode::SourceAuthExpired, "missing password"))?;
        let paths = server::nextcloud::browse_folders(&server.url, user_id, secret, path)
            .await
            .map_err(ApiError::internal)?;
        Ok(paths
            .into_iter()
            .map(|path| api::SourceFolderEntry {
                name: server::nextcloud::folder_name(&path).to_string(),
                path,
            })
            .collect())
    }

    pub async fn validate_source(&self, id: &str) -> Result<api::SourceState, ApiError> {
        let (_, source) = self.source_for(id).await?;
        Ok(match source.validate().await {
            AuthOutcome::Valid => api::SourceState::Online,
            AuthOutcome::Expired => api::SourceState::AuthExpired,
            AuthOutcome::Unreachable => api::SourceState::Offline,
        })
    }
}

/// Accept `seed` cookies if they still validate, else try one keepalive
/// rotation before giving up on them.
#[cfg(not(target_os = "android"))]
async fn try_resume_ytmusic(seed: Option<String>) -> Option<String> {
    let cookies = seed?;
    if server::provider::validate_ytmusic_cookies(&cookies).await {
        return Some(cookies);
    }
    if let Ok(Some(rotated)) = server::ytmusic::verify_session_keepalive::tick(&cookies).await
        && server::provider::validate_ytmusic_cookies(&rotated).await
    {
        return Some(rotated);
    }
    None
}

/// The old settings-actions flow: resume from stored cookies, then from the
/// isolated browser profile, and only then force a full browser sign-in,
/// which must validate before it is trusted. Skipping the resume steps would
/// wipe the profile and demand password/2FA on every transient error.
#[cfg(not(target_os = "android"))]
async fn ensure_ytmusic_signed_in(
    config_cookies: Option<String>,
    browser: config::Browser,
    server_id: &str,
) -> Result<String, String> {
    if let Some(cookies) = try_resume_ytmusic(config_cookies).await {
        return Ok(cookies);
    }

    let profile = server::ytmusic::isolated_profile::profile_dir(server_id);
    if profile.is_dir() {
        let from_profile = server::ytmusic::cookies::extract_from(browser, &profile)
            .await
            .ok();
        if let Some(cookies) = try_resume_ytmusic(from_profile).await {
            return Ok(cookies);
        }
    }

    let cookies = server::ytmusic::isolated_profile::launch_signin_and_extract(
        browser,
        server_id,
        Duration::from_secs(300),
    )
    .await?;
    if !server::provider::validate_ytmusic_cookies(&cookies).await {
        return Err("sign-in completed but YouTube Music validation still failed".to_string());
    }
    Ok(cookies)
}
