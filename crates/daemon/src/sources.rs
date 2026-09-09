//! Configured sources, their credentials, and switching between them.
//!
//! This is where a media source is constructed, which is the whole point: a
//! frontend used to build one, hand it to the daemon, and hold every token it
//! took to sign in. Now it reads rows and calls methods, and no credential
//! ever reaches it -- `SourceInfo` says whether a source is authenticated,
//! never with what.
//!
//! Browser sign-in lives here too. It spawns a browser, drives an isolated
//! profile or a loopback listener, and ends holding a secret, which makes it
//! system-level work regardless of who triggered it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use api::{
    ApiError, CredentialProvision, ErrorCode, LocalSourceDraft, ServerDraft, SourceCapabilities,
    SourceFolderEntry, SourceInfo, SourceKind, SourceLoginRequest, SourceState, Table,
};
use server::source::AuthOutcome;

use crate::config_service::ConfigService;
use crate::session::SessionHandle;

/// How long a browser sign-in may sit waiting for a person.
const SIGNIN_TIMEOUT: Duration = Duration::from_secs(300);

pub struct SourceService {
    db: db::Db,
    session: SessionHandle,
    config: Arc<ConfigService>,
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

/// The in-process capability struct, as the wire describes it.
fn capabilities(caps: server::source::Capabilities) -> SourceCapabilities {
    use api::{AlbumPresentation, ArtistPresentation, FavoritesSyncMode, PlaylistCapability};
    use server::source::{AlbumType, ArtistView, FavoritesSync, PlaylistOps};
    SourceCapabilities {
        edit_tags: caps.edit_tags,
        delete_from_disk: caps.delete_from_disk,
        scan_folders: caps.scan_folders,
        folders: caps.folders,
        sync: caps.sync,
        downloads: caps.downloads,
        discover: caps.discover,
        track_radio: caps.radio.track,
        playlist_radio: caps.radio.playlist,
        playlists: match caps.playlists {
            PlaylistOps::None => PlaylistCapability::None,
            PlaylistOps::AddRemove => PlaylistCapability::AddRemove,
            PlaylistOps::Reorder => PlaylistCapability::Reorder,
        },
        artists: match caps.artist_view {
            ArtistView::Library => ArtistPresentation::Library,
            ArtistView::Remote => ArtistPresentation::Remote,
        },
        albums: match caps.albums {
            AlbumType::Standard => AlbumPresentation::Standard,
            AlbumType::YtMusic => AlbumPresentation::Remote,
        },
        favorites_sync: match caps.favorites_sync {
            FavoritesSync::Instant => FavoritesSyncMode::Instant,
            FavoritesSync::Paginated => FavoritesSyncMode::Paginated,
        },
    }
}

impl SourceService {
    pub fn new(db: db::Db, session: SessionHandle, config: Arc<ConfigService>) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            config,
        })
    }

    async fn current(&self) -> config::AppConfig {
        self.config.snapshot().await
    }

    /// A source id reserved for local libraries must not be claimed by a
    /// server, or the two namespaces collide.
    fn validate_server_id(id: &str) -> Result<(), ApiError> {
        if id.is_empty() || id == "local" || id.starts_with("local:") {
            return Err(ApiError::invalid_input(
                "that id is reserved for local sources",
            ));
        }
        Ok(())
    }

    /// The config as it would be with `id` active, and the source built from
    /// it. Used to describe a source without switching to it.
    async fn resolve(
        &self,
        id: &str,
    ) -> Result<(config::AppConfig, server::source::ActiveSource), ApiError> {
        let mut config = self.current().await;
        match config::Source::from_column(id) {
            config::Source::LocalLibrary(local_id) => {
                if !config
                    .local_sources
                    .iter()
                    .any(|saved| saved.id == local_id)
                {
                    return Err(ApiError::not_found("no such local source"));
                }
                config.set_active_local_source(config::Source::LocalLibrary(local_id));
            }
            config::Source::Server(server_id) => {
                let server = self
                    .db
                    .load_server(&server_id)
                    .await
                    .map_err(db_error)?
                    .ok_or_else(|| ApiError::not_found("no such server"))?;
                config.set_active_server_snapshot(server);
            }
            config::Source::Local => config.set_active_local_source(config::Source::Local),
        }
        let active = Arc::from(server::source::active(self.db.clone(), &config));
        Ok((config, active))
    }

    pub async fn sources(&self) -> Result<Vec<SourceInfo>, ApiError> {
        let config = self.current().await;
        let ids: Vec<String> = std::iter::once("local".to_string())
            .chain(config.local_sources.iter().map(|source| source.id.clone()))
            .chain(config.servers.iter().map(|server| server.id.clone()))
            .collect();
        let mut sources = Vec::with_capacity(ids.len());
        for id in ids {
            sources.push(self.source_info(&id).await?);
        }
        Ok(sources)
    }

    pub async fn source_info(&self, id: &str) -> Result<SourceInfo, ApiError> {
        let current = self.current().await;
        let (resolved, source) = self.resolve(id).await?;
        let key = resolved.active_source.clone();
        let mut info = SourceInfo {
            id: key.as_str().to_string(),
            active: current.active_source.as_str() == key.as_str(),
            capabilities: capabilities(source.capabilities()),
            ..Default::default()
        };
        match &key {
            config::Source::Local => {
                info.name = "Local Library".to_string();
                info.kind = SourceKind::Local;
                info.authenticated = true;
                info.directories = resolved
                    .music_directory
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
            }
            config::Source::LocalLibrary(local_id) => {
                let saved = resolved
                    .local_sources
                    .iter()
                    .find(|saved| saved.id == *local_id)
                    .ok_or_else(|| ApiError::not_found("no such local source"))?;
                info.name = saved.name.clone();
                info.kind = SourceKind::LocalLibrary;
                info.authenticated = true;
                info.directories = saved
                    .directories
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
            }
            config::Source::Server(server_id) => {
                let server = resolved
                    .server
                    .as_ref()
                    .ok_or_else(|| ApiError::not_found("no such server"))?;
                info.name = server.name.clone();
                info.kind = SourceKind::Server;
                info.service = Some(server.service);
                // An anonymous source needs no token to be usable, which is
                // why this is not simply "has a token".
                info.authenticated = server.access_token.is_some() || server.yt_anonymous;
                info.url = Some(server.url.clone());
                info.browser = server.yt_browser.map(|browser| browser.id().to_string());
                info.anonymous = server.yt_anonymous;
                info.storefront = Some(server.apple_music_storefront.clone());
                info.language = Some(server.apple_music_language.clone());
                info.directories = resolved.folders_for(server_id);
            }
        }
        Ok(info)
    }

    /// Push a config change into the session, rebuilding the media source so
    /// later loads resolve against the new backend.
    fn publish(&self, updated: config::AppConfig, changed: Vec<String>) {
        self.session
            .set_active_source(Some(Arc::from(server::source::active(
                self.db.clone(),
                &updated,
            ))));
        self.session.set_config(updated, changed);
    }

    /// Everything a client holds is about to be wrong, so say so once rather
    /// than leaving it to notice per table.
    async fn finish_source_change(
        &self,
        updated: config::AppConfig,
        changed: Vec<String>,
    ) -> Result<(), ApiError> {
        self.publish(updated, changed);
        self.session.reset_playback().await?;
        for table in [
            Table::Servers,
            Table::Tracks,
            Table::Albums,
            Table::Playlists,
            Table::Folders,
            Table::Favorites,
            Table::Recents,
        ] {
            self.session.invalidate(table);
        }
        Ok(())
    }

    pub async fn switch_source(&self, id: &str) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["active_source", "server"])?;
        let previous = self.current().await.active_source;
        let (target, _) = self.resolve(id).await?;
        let source = target.active_source.clone();
        let changed = previous != source;
        let server = target.server.clone();
        let updated = self
            .config
            .mutate_state(move |config| match source {
                config::Source::Local | config::Source::LocalLibrary(_) => {
                    config.set_active_local_source(source)
                }
                config::Source::Server(_) => {
                    if let Some(server) = server {
                        config.set_active_server_snapshot(server);
                    }
                }
            })
            .await?;
        if changed {
            self.finish_source_change(updated, vec!["active_source".to_string()])
                .await?;
        } else {
            self.publish(updated, vec!["active_source".to_string()]);
        }
        self.source_info(id).await
    }

    pub async fn upsert_local_source(
        &self,
        draft: LocalSourceDraft,
    ) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["local_sources"])?;
        let name = draft.name.trim();
        if name.is_empty() {
            return Err(ApiError::invalid_input("a local source needs a name"));
        }
        if draft.directories.is_empty()
            || draft.directories.iter().any(|path| path.trim().is_empty())
        {
            return Err(ApiError::invalid_input(
                "a local source needs at least one directory",
            ));
        }
        let id = draft
            .id
            .unwrap_or_else(|| format!("local:{}", uuid::Uuid::new_v4()));
        if !id.starts_with("local:") {
            return Err(ApiError::invalid_input("that is not a local source id"));
        }
        let saved = config::SavedLocalSource {
            id: id.clone(),
            name: name.to_string(),
            directories: draft.directories.into_iter().map(PathBuf::from).collect(),
        };
        let updated = self
            .config
            .mutate_state(move |config| {
                match config
                    .local_sources
                    .iter_mut()
                    .find(|source| source.id == saved.id)
                {
                    Some(existing) => *existing = saved,
                    None => config.local_sources.push(saved),
                }
            })
            .await?;
        self.publish(updated, vec!["local_sources".to_string()]);
        self.session.invalidate(Table::Servers);
        self.source_info(&id).await
    }

    pub async fn delete_local_source(&self, id: &str) -> Result<(), ApiError> {
        self.config
            .ensure_unlocked(&["active_source", "local_sources"])?;
        if id == "local" {
            return Err(ApiError::invalid_input(
                "the default local library cannot be deleted",
            ));
        }
        let current = self.current().await;
        if !current.local_sources.iter().any(|source| source.id == id) {
            return Err(ApiError::not_found("no such local source"));
        }
        let was_active = current.active_source.local_library_id() == Some(id);
        let id_owned = id.to_string();
        let updated = self
            .config
            .mutate_state(move |config| config.remove_local_source(&id_owned))
            .await?;
        if was_active {
            self.finish_source_change(
                updated,
                vec!["local_sources".to_string(), "active_source".to_string()],
            )
            .await?;
        } else {
            self.publish(updated, vec!["local_sources".to_string()]);
            self.session.invalidate(Table::Servers);
        }
        Ok(())
    }

    pub async fn set_source_directories(
        &self,
        id: &str,
        directories: Vec<String>,
    ) -> Result<SourceInfo, ApiError> {
        if directories.iter().any(|path| path.trim().is_empty()) {
            return Err(ApiError::invalid_input(
                "a source directory cannot be empty",
            ));
        }
        let source = config::Source::from_column(id);
        let key = match &source {
            config::Source::Local => "music_directory",
            config::Source::LocalLibrary(_) => "local_sources",
            config::Source::Server(_) => "server_folders",
        };
        self.config.ensure_unlocked(&[key])?;
        let current = self.current().await;
        match &source {
            config::Source::LocalLibrary(local_id)
                if !current
                    .local_sources
                    .iter()
                    .any(|saved| saved.id == *local_id) =>
            {
                return Err(ApiError::not_found("no such local source"));
            }
            config::Source::Server(server_id)
                if !current.servers.iter().any(|saved| saved.id == *server_id) =>
            {
                return Err(ApiError::not_found("no such server"));
            }
            _ => {}
        }
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
        self.publish(updated, vec![key.to_string()]);
        self.session.invalidate(Table::Servers);
        self.source_info(id).await
    }

    pub async fn upsert_server(&self, draft: ServerDraft) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        if draft.name.trim().is_empty() {
            return Err(ApiError::invalid_input("a server needs a name"));
        }
        if !draft.service.uses_browser_signin() && !draft.url.starts_with("http") {
            return Err(ApiError::invalid_input(
                "a server URL must be http or https",
            ));
        }
        let id = draft.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Self::validate_server_id(&id)?;
        let browser = match draft.browser.as_deref() {
            Some(browser) => Some(
                config::Browser::from_id(browser)
                    .ok_or_else(|| ApiError::invalid_input("no such browser"))?,
            ),
            None => None,
        };
        let saved = config::SavedServer {
            id: id.clone(),
            name: draft.name.trim().to_string(),
            url: draft.url.trim_end_matches('/').to_string(),
            service: draft.service,
            yt_browser: browser,
            yt_anonymous: draft.anonymous,
            apple_music_storefront: draft.storefront.unwrap_or_else(|| "us".to_string()),
            apple_music_language: draft.language.unwrap_or_else(|| "en".to_string()),
        };
        let current = self.current().await;
        // Changing where the active server points invalidates whatever is
        // loaded from it, so playback stops rather than carrying on against
        // the old backend.
        let backend_changed = current.active_source.server_id() == Some(id.as_str())
            && current
                .server
                .as_ref()
                .is_some_and(|server| server.service != saved.service || server.url != saved.url);
        let updated = self
            .config
            .mutate_state(move |config| {
                match config.servers.iter_mut().find(|entry| entry.id == saved.id) {
                    Some(existing) => *existing = saved.clone(),
                    None => config.servers.push(saved.clone()),
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
            })
            .await?;
        self.publish(updated, vec!["servers".to_string()]);
        if backend_changed {
            self.session.reset_playback().await?;
        }
        self.session.invalidate(Table::Servers);
        self.source_info(&id).await
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
        self.publish(
            updated,
            vec!["servers".to_string(), "active_source".to_string()],
        );
        if was_active {
            self.session.reset_playback().await?;
        }
        self.session.invalidate(Table::Servers);
        // The browser profile is this server's, so it goes with it rather
        // than being left behind holding a session.
        #[cfg(not(target_os = "android"))]
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
        #[cfg(target_os = "android")]
        let _ = service;
        Ok(())
    }

    /// Store a secret a caller obtained elsewhere. Write-only: nothing that
    /// comes back out of this service contains it.
    pub async fn provision_credentials(
        &self,
        provision: CredentialProvision,
    ) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        if provision.secret.is_empty() {
            return Err(ApiError::invalid_input("the credential is empty"));
        }
        let mut server = self
            .db
            .load_server(&provision.server_id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found("no such server"))?;
        let previous_user = server.user_id.clone();
        server.access_token = Some(provision.secret);
        server.user_id = provision.user_id;
        if let Some(browser) = provision.browser.as_deref() {
            server.yt_browser = Some(
                config::Browser::from_id(browser)
                    .ok_or_else(|| ApiError::invalid_input("no such browser"))?,
            );
        }
        let active = self
            .current()
            .await
            .active_source
            .server_id()
            .is_some_and(|id| id == provision.server_id);
        let token = server.access_token.clone();
        let user = server.user_id.clone();
        let saved = config::SavedServer::from_music_server(&server);
        let live = server.clone();
        let updated = self
            .config
            .mutate_state(move |config| {
                match config.servers.iter_mut().find(|entry| entry.id == saved.id) {
                    Some(existing) => *existing = saved,
                    None => config.servers.push(saved),
                }
                if active {
                    config.server = Some(live);
                }
            })
            .await?;
        self.db
            .set_server_credentials(&provision.server_id, token.as_deref(), user.as_deref())
            .await
            .map_err(db_error)?;
        if active {
            self.publish(updated, vec!["servers".to_string()]);
            // A different account is a different library, so what is loaded
            // from the old one stops.
            if previous_user != server.user_id {
                self.session.reset_playback().await?;
            }
        } else {
            self.session
                .set_config(updated, vec!["servers".to_string()]);
        }
        self.session.invalidate(Table::Servers);
        self.source_info(&provision.server_id).await
    }

    pub async fn login_source(&self, request: SourceLoginRequest) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        if request.username.trim().is_empty() || request.password.is_empty() {
            return Err(ApiError::invalid_input(
                "a username and password are required",
            ));
        }
        let current = self.current().await;
        let server = self
            .db
            .load_server(&request.server_id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found("no such server"))?;
        let auth =
            server::provider::ProviderClient::new(server.service, server.url, current.device_id)
                .login(request.username.trim(), &request.password)
                .await
                .map_err(|error| ApiError::new(ErrorCode::SourceAuthExpired, error))?;
        self.provision_credentials(CredentialProvision {
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
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found("no such server"))?;
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
            .map_err(db_error)?;
        if active {
            let updated = self
                .config
                .mutate_state(move |config| config.server = Some(server))
                .await?;
            self.publish(updated, vec!["servers".to_string()]);
            if had_credentials {
                self.session.reset_playback().await?;
            }
        }
        self.session.invalidate(Table::Servers);
        Ok(())
    }

    /// Sign in through a browser, and keep the result.
    ///
    /// The secret never leaves this process: the caller learns only that the
    /// source is now authenticated.
    pub async fn authenticate_source(&self, id: &str) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        #[cfg(target_os = "android")]
        {
            let _ = id;
            Err(ApiError::unsupported(
                "browser sign-in runs in the app on Android",
            ))
        }
        #[cfg(not(target_os = "android"))]
        {
            let server = self
                .db
                .load_server(id)
                .await
                .map_err(db_error)?
                .ok_or_else(|| ApiError::not_found("no such server"))?;
            let browser = server.yt_browser.unwrap_or(config::Browser::Chrome);
            let (secret, user_id) = match server.service {
                config::MusicService::YtMusic => {
                    let secret = ensure_ytmusic_signed_in(server.access_token.clone(), browser, id)
                        .await
                        .map_err(ApiError::internal)?;
                    let user = server::ytmusic::derive_user_id(&secret)
                        .unwrap_or_else(|| "me".to_string());
                    (secret, user)
                }
                config::MusicService::SoundCloud => {
                    let secret = server::soundcloud::signin::launch_signin_and_extract(
                        browser,
                        id,
                        SIGNIN_TIMEOUT,
                    )
                    .await
                    .map_err(ApiError::internal)?;
                    let user = server::soundcloud::derive_user_id(&secret)
                        .await
                        .unwrap_or_else(|| "me".to_string());
                    (secret, user)
                }
                config::MusicService::AppleMusic => {
                    let secret = server::applemusic::signin::launch_signin_and_extract(
                        browser,
                        id,
                        SIGNIN_TIMEOUT,
                    )
                    .await
                    .map_err(ApiError::internal)?;
                    (secret, "me".to_string())
                }
                // Spotify's "URL" field holds the client id its PKCE flow
                // needs, not an address.
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
                        "this source signs in with a username and password",
                    ));
                }
            };
            self.provision_credentials(CredentialProvision {
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
    ) -> Result<Vec<SourceFolderEntry>, ApiError> {
        let server = self
            .db
            .load_server(id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found("no such server"))?;
        if server.service != config::MusicService::Nextcloud {
            return Err(ApiError::unsupported(
                "only Nextcloud sources have folders to browse",
            ));
        }
        let user = server
            .user_id
            .as_deref()
            .ok_or_else(|| ApiError::new(ErrorCode::SourceAuthExpired, "not signed in"))?;
        let secret = server
            .access_token
            .as_deref()
            .ok_or_else(|| ApiError::new(ErrorCode::SourceAuthExpired, "not signed in"))?;
        let paths = server::nextcloud::browse_folders(&server.url, user, secret, path)
            .await
            .map_err(ApiError::internal)?;
        Ok(paths
            .into_iter()
            .map(|path| SourceFolderEntry {
                name: server::nextcloud::folder_name(&path).to_string(),
                path,
            })
            .collect())
    }

    pub async fn validate_source(&self, id: &str) -> Result<SourceState, ApiError> {
        let (_, source) = self.resolve(id).await?;
        let state = match source.validate().await {
            AuthOutcome::Valid => SourceState::Online,
            AuthOutcome::Expired => SourceState::AuthExpired,
            AuthOutcome::Unreachable => SourceState::Offline,
        };
        self.session.publish_source_status(id, state);
        Ok(state)
    }

    /// Keep a source signed in without anyone asking. YouTube rotates its
    /// cookies and Spotify's tokens expire, and a frontend that had to do
    /// this was a frontend that had to hold the credential.
    pub fn spawn_credential_upkeep(self: &Arc<Self>) {
        let service = self.clone();
        tokio::spawn(async move {
            // Long enough not to hammer either provider, short enough that a
            // session does not lapse between checks.
            let mut ticker = tokio::time::interval(Duration::from_secs(300));
            let mut since_spotify = 0u32;
            loop {
                ticker.tick().await;
                service.rotate_ytmusic().await;
                since_spotify += 1;
                if since_spotify >= 6 {
                    since_spotify = 0;
                    service.refresh_spotify().await;
                }
            }
        });
    }

    async fn rotate_ytmusic(&self) {
        let config = self.current().await;
        let Some(server) = config
            .server
            .as_ref()
            .filter(|server| server.service == config::MusicService::YtMusic)
        else {
            return;
        };
        let (Some(cookies), Some(id)) = (
            server.access_token.clone(),
            config.active_source.server_id().map(str::to_string),
        ) else {
            return;
        };
        match server::ytmusic::verify_session_keepalive::tick(&cookies).await {
            Ok(Some(rotated)) => {
                let user = server::ytmusic::derive_user_id(&rotated);
                if let Err(error) = self
                    .provision_credentials(CredentialProvision {
                        server_id: id,
                        secret: rotated,
                        user_id: user,
                        browser: None,
                    })
                    .await
                {
                    tracing::warn!(%error, "storing rotated YouTube Music cookies failed");
                }
            }
            Ok(None) => {}
            Err(error) => tracing::debug!(%error, "YouTube Music keepalive failed"),
        }
    }

    async fn refresh_spotify(&self) {
        let config = self.current().await;
        let Some(server) = config
            .server
            .as_ref()
            .filter(|server| server.service == config::MusicService::Spotify)
        else {
            return;
        };
        let (Some(packed), Some(id)) = (
            server.access_token.clone(),
            config.active_source.server_id().map(str::to_string),
        ) else {
            return;
        };
        match server::spotify::auth::refresh_packed(&packed, server.url.clone()).await {
            Ok(refreshed) if refreshed != packed => {
                if let Err(error) = self
                    .provision_credentials(CredentialProvision {
                        server_id: id,
                        secret: refreshed,
                        user_id: server.user_id.clone(),
                        browser: None,
                    })
                    .await
                {
                    tracing::warn!(%error, "storing the refreshed Spotify token failed");
                }
            }
            Ok(_) => {}
            Err(error) => tracing::debug!(%error, "Spotify token refresh failed"),
        }
    }
}

/// Accept cookies that still validate, else try one keepalive rotation before
/// giving up on them.
#[cfg(not(target_os = "android"))]
async fn try_resume_ytmusic(seed: Option<String>) -> Option<String> {
    let cookies = seed?;
    if server::provider::validate_ytmusic_cookies(&cookies).await {
        return Some(cookies);
    }
    let rotated = server::ytmusic::verify_session_keepalive::tick(&cookies)
        .await
        .ok()??;
    server::provider::validate_ytmusic_cookies(&rotated)
        .await
        .then_some(rotated)
}

/// Resume from stored cookies, then from the isolated browser profile, and
/// only then force a full sign-in -- which must validate before it is
/// trusted. Skipping the resume steps would demand a password and 2FA on
/// every transient network error.
#[cfg(not(target_os = "android"))]
async fn ensure_ytmusic_signed_in(
    stored: Option<String>,
    browser: config::Browser,
    server_id: &str,
) -> Result<String, String> {
    if let Some(cookies) = try_resume_ytmusic(stored).await {
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
        SIGNIN_TIMEOUT,
    )
    .await?;
    if !server::provider::validate_ytmusic_cookies(&cookies).await {
        return Err("sign-in finished but YouTube Music still rejected the session".to_string());
    }
    Ok(cookies)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Local source ids live in their own namespace; a server that claimed
    /// one would shadow a library.
    #[test]
    fn a_server_cannot_take_a_local_source_id() {
        for id in ["", "local", "local:library"] {
            let error = SourceService::validate_server_id(id).expect_err("reserved");
            assert_eq!(error.code, ErrorCode::InvalidInput);
        }
        assert!(SourceService::validate_server_id("jellyfin-1").is_ok());
    }
}
