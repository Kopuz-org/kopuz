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

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::{
    ApiError, BrowserSession, CredentialProvision, ErrorCode, SourceCapabilities, SourceDraft,
    SourceFolderEntry, SourceInfo, SourceLoginRequest, SourceState, Table,
};
use server::source::{AuthOutcome, MediaSource};

use crate::config_service::ConfigService;
use crate::recovery::{Recovery, once_more};
use crate::session::SessionHandle;

/// How long a browser sign-in may sit waiting for a person.
const SIGNIN_TIMEOUT: Duration = Duration::from_secs(300);

/// How often an active source that is not online is probed again, so it recovers without a switch.
const RETRY_PROBE: Duration = Duration::from_secs(60);

pub struct SourceService {
    db: db::Db,
    session: SessionHandle,
    config: Arc<ConfigService>,
    /// Each source's last probe answer, tagged with the probe that wrote it.
    status: Mutex<HashMap<String, (u64, SourceState)>>,
    probes: AtomicU64,
    upstream: Arc<dyn Upstream>,
    recovery: Recovery,
}

/// What probing and recovering a session ask of the network and of the
/// browser profiles on this machine.
#[async_trait::async_trait]
pub(crate) trait Upstream: Send + Sync {
    async fn validate(&self, source: &dyn MediaSource) -> AuthOutcome;

    /// Working cookies for `server`'s YouTube Music session: the stored ones
    /// after one `verify_session` rotation, else a fresh read of the profile
    /// it signed in with.
    async fn recover_ytmusic(&self, server: &config::MusicServer, id: &str) -> Option<String>;
}

struct Network;

#[async_trait::async_trait]
impl Upstream for Network {
    async fn validate(&self, source: &dyn MediaSource) -> AuthOutcome {
        source.validate().await
    }

    async fn recover_ytmusic(&self, server: &config::MusicServer, id: &str) -> Option<String> {
        let stored = server.access_token.clone();
        if let Some(cookies) = try_resume_ytmusic(stored).await {
            return Some(cookies);
        }
        #[cfg(target_os = "android")]
        {
            let _ = id;
            None
        }
        #[cfg(not(target_os = "android"))]
        {
            let browser = server::cookies::resolve_browser(server.yt_browser).await;
            resume_from_profiles(browser, id, server.yt_profile.as_deref()).await
        }
    }
}

fn auth_expired<T>(result: &Result<T, ApiError>) -> bool {
    matches!(result, Err(error) if error.code == ErrorCode::SourceAuthExpired)
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
        browse_folders: caps.browse_folders,
        external_devices: caps.external_devices,
        browser_playback: caps.browser_playback,
        sync: caps.sync,
        downloads: caps.downloads,
        discover: caps.discover,
        dont_recommend: caps.dont_recommend,
        track_radio: caps.radio.track,
        playlist_radio: caps.radio.playlist,
        search_radio: caps.radio.track && caps.radio.search,
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
        Self::with_upstream(db, session, config, Arc::new(Network))
    }

    pub(crate) fn with_upstream(
        db: db::Db,
        session: SessionHandle,
        config: Arc<ConfigService>,
        upstream: Arc<dyn Upstream>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            config,
            status: Mutex::new(HashMap::new()),
            probes: AtomicU64::new(0),
            upstream,
            recovery: Recovery::default(),
        })
    }

    /// The last probe answer for `id`, if it has been probed.
    pub fn status(&self, id: &str) -> Option<SourceState> {
        let status = self.status.lock().ok()?;
        status.get(id).map(|(_, state)| *state)
    }

    /// Store a probe's answer unless a later probe of the same source already wrote; announce it when it moved.
    fn record_status(&self, id: &str, probe: u64, state: SourceState) {
        let Ok(mut status) = self.status.lock() else {
            return;
        };
        let previous = status.get(id).copied();
        if previous.is_some_and(|(at, _)| at > probe) {
            return;
        }
        status.insert(id.to_string(), (probe, state));
        drop(status);
        if previous.map(|(_, was)| was) != Some(state) {
            self.session.publish_source_status(id, state);
        }
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
        }
        let active = Arc::from(server::source::active(self.db.clone(), &config));
        Ok((config, active))
    }

    pub async fn sources(&self) -> Result<Vec<SourceInfo>, ApiError> {
        let config = self.current().await;
        let ids: Vec<String> = config
            .local_sources
            .iter()
            .map(|source| source.id.clone())
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
            state: self.status(key.as_str()),
            ..Default::default()
        };
        match &key {
            config::Source::LocalLibrary(local_id) => {
                let saved = resolved
                    .local_sources
                    .iter()
                    .find(|saved| saved.id == *local_id)
                    .ok_or_else(|| ApiError::not_found("no such local source"))?;
                info.name = saved.name.clone();
                info.service = crate::services::folders_ref();
                info.authenticated = true;
                info.permanent = saved.id == config::DEFAULT_LOCAL_ID;
                let paths: Vec<String> = saved
                    .directories
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                info.settings = vec![crate::services::directories_field(&paths)];
            }
            config::Source::Server(server_id) => {
                let server = resolved
                    .server
                    .as_ref()
                    .ok_or_else(|| ApiError::not_found("no such server"))?;
                let view = crate::services::ServerView::from(server);
                info.name = server.name.clone();
                info.needs_network = true;
                info.service = crate::services::service_ref(server.service);
                // An anonymous source needs no token to be usable, which is
                // why this is not simply "has a token".
                info.authenticated = server.access_token.is_some() || server.yt_anonymous;
                info.sign_in = crate::services::sign_in(&view, info.authenticated);
                // What signing in again takes, for credentials that went stale.
                info.reauth = crate::services::sign_in(&view, false);
                info.detail = crate::services::detail(&view);
                info.anonymous = server.yt_anonymous;
                info.settings = crate::services::settings(&view, &current);
                if info.capabilities.browse_folders {
                    info.settings.push(crate::services::directories_field(
                        &resolved.folders_for(server_id),
                    ));
                }
            }
        }
        Ok(info)
    }

    /// Whether a browser sign-in can run here at all. A sandboxed daemon with
    /// no host access cannot spawn one, and a client should say so before
    /// offering a source whose only sign-in is a browser one.
    pub async fn can_open_browser(&self) -> bool {
        #[cfg(target_os = "android")]
        {
            true
        }
        #[cfg(not(target_os = "android"))]
        {
            server::cookies::has_host_spawn().await
        }
    }

    /// Everything a client holds is about to be wrong, so say so once rather
    /// than leaving it to notice per table.
    fn finish_source_change(&self) {
        self.session.clear_error();
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
    }

    pub async fn switch_source(&self, id: &str) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["active_source", "server"])?;
        let previous = self.current().await.active_source;
        let (target, _) = self.resolve(id).await?;
        let source = target.active_source.clone();
        let changed = previous != source;
        let server = target.server.clone();
        self.config
            .mutate_state(&["active_source", "server"], move |config| match source {
                config::Source::LocalLibrary(_) => config.set_active_local_source(source),
                config::Source::Server(_) => {
                    if let Some(server) = server {
                        config.set_active_server_snapshot(server);
                    }
                }
            })
            .await?;
        if changed {
            self.finish_source_change();
        }
        self.source_info(id).await
    }

    async fn upsert_folder_source(&self, draft: SourceDraft) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["local_sources"])?;
        if let Some(problem) = crate::services::check_folders(&draft).first() {
            return Err(ApiError::invalid_input(crate::services::problem_text(
                problem,
            )));
        }
        let id = draft
            .id
            .clone()
            .unwrap_or_else(|| format!("local:{}", uuid::Uuid::new_v4()));
        if !id.starts_with("local:") && id != config::DEFAULT_LOCAL_ID {
            return Err(ApiError::invalid_input("that is not a folder source id"));
        }
        let saved = config::SavedLocalSource {
            id: id.clone(),
            name: draft.name.trim().to_string(),
            directories: crate::services::folder_paths(&draft.values)
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        };
        self.config
            .mutate_state(&["local_sources"], move |config| {
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
        self.session.invalidate(Table::Servers);
        self.source_info(&id).await
    }

    async fn delete_folder_source(&self, id: &str) -> Result<(), ApiError> {
        self.config
            .ensure_unlocked(&["active_source", "local_sources"])?;
        if id == config::DEFAULT_LOCAL_ID {
            return Err(ApiError::invalid_input(
                "the default folder source cannot be deleted",
            ));
        }
        let current = self.current().await;
        if !current.local_sources.iter().any(|source| source.id == id) {
            return Err(ApiError::not_found("no such source"));
        }
        let was_active = current.active_source.local_library_id() == Some(id);
        let id_owned = id.to_string();
        // The session drops this source's queue itself once its pending saves land, so none outlives the purge.
        self.config
            .mutate_state(&["local_sources", "active_source"], move |config| {
                config.remove_local_source(&id_owned)
            })
            .await?;
        self.db
            .purge_source(&config::Source::from_column(id))
            .await
            .map_err(db_error)?;
        if was_active {
            self.finish_source_change();
        } else {
            self.session.invalidate(Table::Servers);
        }
        Ok(())
    }

    /// Replace a folder source's folders.
    async fn set_folder_settings(
        &self,
        id: &str,
        values: &[api::FieldValue],
    ) -> Result<SourceInfo, ApiError> {
        let Some(folders) = api::schema::value_of(values, crate::services::DIRECTORIES) else {
            return self.source_info(id).await;
        };
        let Ok(directories) = serde_json::from_str::<Vec<String>>(folders) else {
            return Err(ApiError::invalid_input(
                "source directories must be a list of paths",
            ));
        };
        if directories.iter().any(|path| path.trim().is_empty()) {
            return Err(ApiError::invalid_input(
                "a source directory cannot be empty",
            ));
        }
        self.config.ensure_unlocked(&["local_sources"])?;
        let current = self.current().await;
        if !current.local_sources.iter().any(|saved| saved.id == id) {
            return Err(ApiError::not_found("no such source"));
        }
        let target = id.to_string();
        self.config
            .mutate_state(&["local_sources"], move |config| {
                if let Some(saved) = config
                    .local_sources
                    .iter_mut()
                    .find(|saved| saved.id == target)
                {
                    saved.directories = directories.into_iter().map(PathBuf::from).collect();
                }
            })
            .await?;
        self.session.invalidate(Table::Servers);
        self.source_info(id).await
    }

    /// Which service a draft names, refused as invalid input if it is not one
    /// this daemon has.
    fn drafted_service(draft: &SourceDraft) -> Result<config::MusicService, ApiError> {
        config::MusicService::from_id(&draft.service)
            .ok_or_else(|| ApiError::invalid_input("no such service"))
    }

    pub async fn services(&self) -> Vec<api::ServiceInfo> {
        crate::services::all()
    }

    pub async fn check_source_draft(
        &self,
        draft: SourceDraft,
    ) -> Result<api::DraftCheck, ApiError> {
        if draft.service == crate::services::FOLDERS {
            return Ok(api::DraftCheck {
                sign_in: api::SignInKind::None,
                problems: crate::services::check_folders(&draft),
            });
        }
        let service = Self::drafted_service(&draft)?;
        let (sign_in, problems) = crate::services::check(service, &draft);
        Ok(api::DraftCheck { sign_in, problems })
    }

    pub async fn upsert_source(&self, draft: SourceDraft) -> Result<SourceInfo, ApiError> {
        if draft.service == crate::services::FOLDERS {
            return self.upsert_folder_source(draft).await;
        }
        self.upsert_server(draft).await
    }

    pub async fn delete_source(&self, id: &str) -> Result<(), ApiError> {
        match config::Source::from_column(id) {
            config::Source::LocalLibrary(_) => self.delete_folder_source(id).await,
            config::Source::Server(_) => self.delete_server(id).await,
        }
    }

    async fn upsert_server(&self, draft: SourceDraft) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        let service = Self::drafted_service(&draft)?;
        let (_, problems) = crate::services::check(service, &draft);
        if let Some(problem) = problems.first() {
            return Err(ApiError::invalid_input(crate::services::problem_text(
                problem,
            )));
        }
        let id = draft
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Self::validate_server_id(&id)?;
        let mut saved = config::SavedServer::new(String::new(), String::new(), service);
        saved.id = id.clone();
        crate::services::apply(service, &draft, &mut saved);
        let secret = api::schema::value_of(&draft.secrets, crate::services::TOKEN)
            .map(str::trim)
            .filter(|secret| !secret.is_empty())
            .map(str::to_string);
        let current = self.current().await;
        // Changing where the active server points invalidates whatever is
        // loaded from it, so playback stops rather than carrying on against
        // the old backend.
        let backend_changed = current.active_source.server_id() == Some(id.as_str())
            && current
                .server
                .as_ref()
                .is_some_and(|server| server.service != saved.service || server.url != saved.url);
        self.config
            .mutate_state(&["servers", "server"], move |config| {
                match config.servers.iter_mut().find(|entry| entry.id == saved.id) {
                    // The form does not carry the imported profile, so an edit
                    // keeps it unless the source now speaks another service.
                    Some(existing) => {
                        let profile = existing
                            .yt_profile
                            .take()
                            .filter(|_| existing.service == saved.service);
                        *existing = saved.clone();
                        existing.yt_profile = profile;
                    }
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
        if backend_changed {
            self.session.reset_playback().await?;
        }
        self.session.invalidate(Table::Servers);
        // A secret answered in the form is stored the same way one obtained
        // any other way is, and never travels back out.
        if let Some(secret) = secret {
            return self
                .provision_credentials(api::CredentialProvision {
                    server_id: id,
                    secret,
                    user_id: Some("me".to_string()),
                    browser: None,
                })
                .await;
        }
        self.source_info(&id).await
    }

    /// Answer a source's own options. An absent key is left alone; the Spotify
    /// ones live on the config rather than the server row, so both are written
    /// in the one mutation.
    pub async fn set_source_settings(
        &self,
        id: &str,
        values: Vec<api::FieldValue>,
    ) -> Result<SourceInfo, ApiError> {
        if matches!(
            config::Source::from_column(id),
            config::Source::LocalLibrary(_)
        ) {
            return self.set_folder_settings(id, &values).await;
        }
        let current = self.current().await;
        let Some(existing) = current.servers.iter().find(|server| server.id == id) else {
            return Err(ApiError::not_found("no such server"));
        };
        let mut keys = vec!["servers".to_string()];
        keys.extend(
            crate::services::config_keys(existing, &values)
                .into_iter()
                .map(str::to_string),
        );
        let folders = api::schema::value_of(&values, crate::services::DIRECTORIES)
            .map(api::schema::decode_directories);
        if let Some(folders) = &folders {
            if folders.iter().any(|path| path.trim().is_empty()) {
                return Err(ApiError::invalid_input(
                    "a source directory cannot be empty",
                ));
            }
            keys.push("server_folders".to_string());
        }
        let locked: Vec<&str> = keys.iter().map(String::as_str).collect();
        self.config.ensure_unlocked(&locked)?;
        let target = id.to_string();
        self.config
            .mutate_state(&locked, move |config| {
                let Some(index) = config.servers.iter().position(|server| server.id == target)
                else {
                    return;
                };
                let mut saved = config.servers[index].clone();
                crate::services::apply_server_settings(&values, &mut saved);
                crate::services::apply_config_settings(saved.service, &values, config);
                config.servers[index] = saved.clone();
                if let Some(folders) = folders {
                    config.set_folders_for(&saved.id, folders);
                }
                if config.active_source.server_id() == Some(saved.id.as_str())
                    && let Some(server) = config.server.as_mut()
                {
                    server.yt_browser = saved.yt_browser;
                    server
                        .apple_music_storefront
                        .clone_from(&saved.apple_music_storefront);
                    server
                        .apple_music_language
                        .clone_from(&saved.apple_music_language);
                }
            })
            .await?;
        self.session.invalidate(Table::Servers);
        self.source_info(id).await
    }

    async fn delete_server(&self, id: &str) -> Result<(), ApiError> {
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
        self.config
            .mutate_state(&["servers", "active_source", "server"], move |config| {
                config.remove_saved_server(&id_owned);
                if was_active {
                    config.clear_active_server();
                }
            })
            .await?;
        if was_active {
            self.finish_source_change();
        } else {
            self.session.invalidate(Table::Servers);
        }
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
        self.store_credentials(provision, None).await
    }

    /// [`Self::provision_credentials`], recording which browser profile a
    /// session came from when it names a browser. A browser sign-in names
    /// none, which forgets an earlier import's profile.
    async fn store_credentials(
        &self,
        provision: CredentialProvision,
        profile: Option<String>,
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
            // Signed in through a browser, so no longer anonymous.
            server.yt_anonymous = false;
            server.yt_profile = profile;
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
        self.config
            .mutate_state(&["servers", "server"], move |config| {
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
        // A different account is a different library, so what is loaded
        // from the old one stops.
        if active && previous_user != server.user_id {
            self.session.reset_playback().await?;
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
            self.config
                .mutate_state(&["server"], move |config| config.server = Some(server))
                .await?;
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
            let source = self
                .db
                .load_server(id)
                .await
                .map_err(db_error)?
                .ok_or_else(|| ApiError::not_found("no such server"))?;
            let (secret, user_id) = crate::android_signin::sign_in(source.service, source.url)
                .await
                .map_err(ApiError::internal)?;
            self.provision_credentials(CredentialProvision {
                server_id: id.to_string(),
                secret,
                user_id,
                browser: None,
            })
            .await
        }
        #[cfg(not(target_os = "android"))]
        {
            let server = self
                .db
                .load_server(id)
                .await
                .map_err(db_error)?
                .ok_or_else(|| ApiError::not_found("no such server"))?;
            // No stored choice means "the system default", resolved here and
            // persisted with the credential below, so a later cookie read goes
            // back to the browser that actually holds the session.
            let browser = server::cookies::resolve_browser(server.yt_browser).await;
            let (secret, user_id) = match server.service {
                config::MusicService::YtMusic => {
                    let secret = ensure_ytmusic_signed_in(
                        server.access_token.clone(),
                        browser,
                        id,
                        server.yt_profile.as_deref(),
                    )
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

    /// Browser profiles on this machine already signed in to `service`.
    pub async fn browser_sessions(&self, service: &str) -> Result<Vec<BrowserSession>, ApiError> {
        if config::MusicService::from_id(service) != Some(config::MusicService::YtMusic) {
            return Ok(Vec::new());
        }
        #[cfg(target_os = "android")]
        {
            Ok(Vec::new())
        }
        #[cfg(not(target_os = "android"))]
        {
            let scratch = server::ytmusic::browser_sessions::scratch_dir();
            Ok(server::ytmusic::browser_sessions::signed_in(&scratch)
                .await
                .into_iter()
                .map(|profile| BrowserSession {
                    id: profile.id(),
                    browser: profile.browser.label().to_string(),
                    profile: profile.name,
                    account: profile.email,
                })
                .collect())
        }
    }

    /// Sign `id` in with the session a browser profile on this machine holds.
    ///
    /// Like a browser sign-in, the secret never leaves this process.
    pub async fn import_browser_session(
        &self,
        id: &str,
        session: &str,
    ) -> Result<SourceInfo, ApiError> {
        self.config.ensure_unlocked(&["server", "servers"])?;
        let server = self
            .db
            .load_server(id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found("no such server"))?;
        if server.service != config::MusicService::YtMusic {
            return Err(ApiError::unsupported(
                "this source cannot take a session from a browser profile",
            ));
        }
        #[cfg(target_os = "android")]
        {
            let _ = session;
            Err(ApiError::unsupported(
                "there are no browser profiles to take a session from here",
            ))
        }
        #[cfg(not(target_os = "android"))]
        {
            let scratch = server::ytmusic::browser_sessions::scratch_dir();
            let (browser, cookies) = server::ytmusic::browser_sessions::import(session, &scratch)
                .await
                .map_err(ApiError::invalid_input)?;
            let secret = try_resume_ytmusic(Some(cookies)).await.ok_or_else(|| {
                ApiError::invalid_input(format!(
                    "YouTube Music did not accept the session from {}. Open music.youtube.com there to refresh it, then try again.",
                    browser.label()
                ))
            })?;
            self.store_import(id, session, browser, secret).await
        }
    }

    /// Keep a session imported from the browser profile `profile` names, and
    /// where it came from, so recovery can read that profile again.
    async fn store_import(
        &self,
        id: &str,
        profile: &str,
        browser: config::Browser,
        secret: String,
    ) -> Result<SourceInfo, ApiError> {
        let user = server::ytmusic::derive_user_id(&secret).unwrap_or_else(|| "me".to_string());
        self.store_credentials(
            CredentialProvision {
                server_id: id.to_string(),
                secret,
                user_id: Some(user),
                browser: Some(browser.id().to_string()),
            },
            Some(profile.to_string()),
        )
        .await
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
        let (_, built) = self.resolve(id).await?;
        if !built.capabilities().browse_folders {
            return Err(ApiError::unsupported(
                "this source's library is not a folder tree",
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
        self.probe(id, false).await
    }

    /// Probe `id` and record the answer; `announce` shows it as checking meanwhile, for a source with no answer of its own yet.
    async fn probe(&self, id: &str, announce: bool) -> Result<SourceState, ApiError> {
        let probe = self.probes.fetch_add(1, Ordering::Relaxed) + 1;
        if announce {
            self.record_status(id, probe, SourceState::Checking);
        }
        let seen = self.recovery.epoch();
        let outcome = once_more(
            || async {
                let (_, source) = self.resolve(id).await?;
                Ok::<_, ApiError>(self.upstream.validate(source.as_ref()).await)
            },
            |outcome| matches!(outcome, Ok(AuthOutcome::Expired)),
            || self.recover(id, seen),
        )
        .await?;
        let state = match outcome {
            AuthOutcome::Valid => SourceState::Online,
            AuthOutcome::Expired => SourceState::AuthExpired,
            AuthOutcome::Unreachable => SourceState::Offline,
        };
        self.record_status(id, probe, state);
        Ok(state)
    }

    /// Run `attempt`, a call that reaches the active source. If it answers
    /// that the session expired, recover the session and run it once more.
    pub async fn recovering<T, A>(&self, attempt: impl FnMut() -> A) -> Result<T, ApiError>
    where
        A: Future<Output = Result<T, ApiError>>,
    {
        let seen = self.recovery.epoch();
        once_more(attempt, auth_expired, || self.recover_active(seen)).await
    }

    async fn recover_active(&self, seen: u64) -> bool {
        let Some(id) = self
            .current()
            .await
            .active_source
            .server_id()
            .map(str::to_string)
        else {
            return false;
        };
        // Once recovery has given up, the re-probe retries it, so requests
        // against a dead session do not each read a browser profile.
        if self.status(&id) == Some(SourceState::AuthExpired) {
            return false;
        }
        self.recover(&id, seen).await
    }

    /// Get `id`'s session back and store it, at most one attempt at a time
    /// across the daemon. A source with nothing to recover answers false.
    async fn recover(&self, id: &str, seen: u64) -> bool {
        self.recovery
            .run(seen, async {
                let recovered = self.recover_session(id).await;
                if !recovered {
                    let probe = self.probes.fetch_add(1, Ordering::Relaxed) + 1;
                    self.record_status(id, probe, SourceState::AuthExpired);
                }
                recovered
            })
            .await
    }

    async fn recover_session(&self, id: &str) -> bool {
        let server = match self.db.load_server(id).await {
            Ok(Some(server))
                if server.service == config::MusicService::YtMusic
                    && server.access_token.is_some() =>
            {
                server
            }
            Ok(_) => return false,
            Err(error) => {
                tracing::warn!(%error, source = %id, "loading a source to recover its session failed");
                return false;
            }
        };
        let Some(cookies) = self.upstream.recover_ytmusic(&server, id).await else {
            tracing::warn!(source = %id, "the YouTube Music session expired and could not be recovered");
            return false;
        };
        let user = server::ytmusic::derive_user_id(&cookies).or(server.user_id);
        match self
            .provision_credentials(CredentialProvision {
                server_id: id.to_string(),
                secret: cookies,
                user_id: user,
                browser: None,
            })
            .await
        {
            Ok(_) => {
                tracing::info!(source = %id, "recovered the YouTube Music session");
                true
            }
            Err(error) => {
                tracing::warn!(%error, source = %id, "storing the recovered YouTube Music session failed");
                false
            }
        }
    }

    /// Probe the active source whenever it or its server entry changes, and retry one that is not online.
    pub fn watch_active(
        self: &Arc<Self>,
        mut config: tokio::sync::watch::Receiver<config::AppConfig>,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            let key =
                |config: &config::AppConfig| (config.active_source.clone(), config.server.clone());
            let mut probed: Option<(config::Source, Option<config::MusicServer>)> = None;
            let mut retry = tokio::time::interval(RETRY_PROBE);
            retry.tick().await;
            let mut retry_due = false;
            loop {
                let next = key(&config.borrow_and_update());
                let id = next.0.as_str().to_string();
                let moved = probed.as_ref() != Some(&next);
                let unsettled = retry_due && service.status(&id) != Some(SourceState::Online);
                if moved || unsettled {
                    let fresh = probed.as_ref().is_none_or(|(source, _)| *source != next.0);
                    probed = Some(next);
                    // Spawned, so a slow unreachable server cannot hold up probing the next source.
                    let service = service.clone();
                    tokio::spawn(async move {
                        if let Err(error) = service.probe(&id, fresh).await {
                            tracing::debug!(%error, source = %id, "probing the active source failed");
                        }
                    });
                }
                retry_due = tokio::select! {
                    changed = config.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        false
                    }
                    _ = retry.tick() => true,
                };
            }
        });
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
    imported: Option<&str>,
) -> Result<String, String> {
    if let Some(cookies) = try_resume_ytmusic(stored).await {
        return Ok(cookies);
    }
    if let Some(cookies) = resume_from_profiles(browser, server_id, imported).await {
        return Ok(cookies);
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

/// A session read again from the isolated sign-in profile, else from the
/// browser profile `imported` names. Either has to validate.
#[cfg(not(target_os = "android"))]
async fn resume_from_profiles(
    browser: config::Browser,
    server_id: &str,
    imported: Option<&str>,
) -> Option<String> {
    let isolated = server::ytmusic::isolated_profile::profile_dir(server_id);
    if isolated.is_dir() {
        let from_profile = server::ytmusic::cookies::extract_from(browser, &isolated)
            .await
            .ok();
        if let Some(cookies) = try_resume_ytmusic(from_profile).await {
            return Some(cookies);
        }
    }
    let scratch = server::ytmusic::browser_sessions::scratch_dir();
    let (_, cookies) = server::ytmusic::browser_sessions::import(imported?, &scratch)
        .await
        .ok()?;
    try_resume_ytmusic(Some(cookies)).await
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

    fn youtube_music(anonymous: bool) -> config::AppConfig {
        let mut server = config::MusicServer::new_with_service(
            "YouTube Music".into(),
            "https://music.youtube.com".into(),
            config::MusicService::YtMusic,
        );
        server.id = Some("yt".into());
        server.access_token = Some("SAPISID=leftover".into());
        server.yt_anonymous = anonymous;
        config::AppConfig {
            server: Some(server),
            active_source: config::Source::Server("yt".into()),
            ..Default::default()
        }
    }

    struct NoLibrary;

    #[async_trait::async_trait]
    impl crate::QueueMaterializer for NoLibrary {
        async fn materialize(&self, _: &api::QueueContext) -> Result<Vec<reader::Track>, ApiError> {
            Ok(Vec::new())
        }
    }

    /// Answers probes from a script and recovers to a fixed session.
    #[derive(Default)]
    struct FakeUpstream {
        recovered: Option<String>,
        recoveries: std::sync::atomic::AtomicUsize,
        validations: Mutex<std::collections::VecDeque<AuthOutcome>>,
    }

    impl FakeUpstream {
        fn recovering_to(cookies: Option<&str>) -> Self {
            Self {
                recovered: cookies.map(str::to_string),
                ..Default::default()
            }
        }

        fn recoveries(&self) -> usize {
            self.recoveries.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Upstream for FakeUpstream {
        async fn validate(&self, _: &dyn MediaSource) -> AuthOutcome {
            let mut script = self.validations.lock().expect("script");
            script.pop_front().unwrap_or(AuthOutcome::Valid)
        }

        async fn recover_ytmusic(&self, _: &config::MusicServer, _: &str) -> Option<String> {
            tokio::task::yield_now().await;
            self.recoveries.fetch_add(1, Ordering::SeqCst);
            self.recovered.clone()
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        db: db::Db,
        sources: Arc<SourceService>,
    }

    impl Fixture {
        async fn token(&self) -> Option<String> {
            self.db
                .load_server("yt")
                .await
                .expect("load")
                .expect("server")
                .access_token
        }
    }

    /// A signed-in YouTube Music source, active, over a real database.
    async fn signed_in(upstream: Arc<FakeUpstream>) -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = db::init(&dir.path().join("sources.db")).await.expect("db");
        let mut seeded = youtube_music(false);
        if let Some(server) = seeded.server.as_ref() {
            seeded
                .servers
                .push(config::SavedServer::from_music_server(server));
        }
        let config = Arc::new(ConfigService::new(
            db.clone(),
            dir.path().join("settings.toml"),
            seeded,
        ));
        config
            .mutate_state(&[], |_| {})
            .await
            .expect("seed the database");
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("headless player starts");
        let session = SessionHandle::spawn_with_factory(
            Arc::new(NoLibrary),
            player,
            crate::PlaybackServices::default(),
            Arc::new(|_| None),
        );
        config.attach_session(session.clone());
        let sources = SourceService::with_upstream(db.clone(), session, config, upstream);
        Fixture {
            _dir: dir,
            db,
            sources,
        }
    }

    fn expired() -> ApiError {
        ApiError::new(ErrorCode::SourceAuthExpired, "signed out")
    }

    /// What the active source's config holds, failing the first time, the
    /// way a request against a session YouTube stopped taking does.
    async fn request(
        fixture: &Fixture,
        attempts: &std::sync::atomic::AtomicUsize,
    ) -> Result<Option<String>, ApiError> {
        fixture
            .sources
            .recovering(|| async {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(expired());
                }
                Ok(fixture
                    .sources
                    .current()
                    .await
                    .server
                    .and_then(|server| server.access_token))
            })
            .await
    }

    #[tokio::test]
    async fn an_expired_request_recovers_the_session_and_runs_once_more() {
        let upstream = Arc::new(FakeUpstream::recovering_to(Some("SAPISID=fresh")));
        let fixture = signed_in(upstream.clone()).await;
        let attempts = std::sync::atomic::AtomicUsize::new(0);

        let answer = request(&fixture, &attempts).await.expect("retried");

        assert_eq!(
            answer.as_deref(),
            Some("SAPISID=fresh"),
            "the retry saw the recovered session"
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(upstream.recoveries(), 1);
        assert_eq!(fixture.token().await.as_deref(), Some("SAPISID=fresh"));
    }

    #[tokio::test]
    async fn a_failed_recovery_is_not_retried_and_marks_the_source_expired() {
        let upstream = Arc::new(FakeUpstream::recovering_to(None));
        let fixture = signed_in(upstream.clone()).await;
        let attempts = std::sync::atomic::AtomicUsize::new(0);

        let error = request(&fixture, &attempts)
            .await
            .expect_err("still expired");

        assert_eq!(error.code, ErrorCode::SourceAuthExpired);
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "no retry without a session"
        );
        assert_eq!(fixture.sources.status("yt"), Some(SourceState::AuthExpired));
        assert_eq!(fixture.token().await.as_deref(), Some("SAPISID=leftover"));

        let again = std::sync::atomic::AtomicUsize::new(0);
        request(&fixture, &again).await.expect_err("still expired");
        assert_eq!(
            upstream.recoveries(),
            1,
            "the re-probe owns the next attempt"
        );
    }

    #[tokio::test]
    async fn concurrent_expired_requests_share_one_recovery() {
        let upstream = Arc::new(FakeUpstream::recovering_to(Some("SAPISID=fresh")));
        let fixture = signed_in(upstream.clone()).await;
        let counters: Vec<_> = (0..8)
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect();

        let answers = futures_util::future::join_all(
            counters.iter().map(|attempts| request(&fixture, attempts)),
        )
        .await;

        for answer in answers {
            assert_eq!(answer.expect("retried").as_deref(), Some("SAPISID=fresh"));
        }
        assert_eq!(upstream.recoveries(), 1);
        for attempts in &counters {
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
        }
    }

    #[tokio::test]
    async fn a_probe_that_finds_the_session_expired_recovers_it_and_probes_again() {
        let upstream = Arc::new(FakeUpstream::recovering_to(Some("SAPISID=fresh")));
        upstream
            .validations
            .lock()
            .expect("script")
            .extend([AuthOutcome::Expired, AuthOutcome::Valid]);
        let fixture = signed_in(upstream.clone()).await;

        let state = fixture.sources.validate_source("yt").await.expect("probe");

        assert_eq!(state, SourceState::Online);
        assert_eq!(fixture.sources.status("yt"), Some(SourceState::Online));
        assert_eq!(upstream.recoveries(), 1);
        assert!(upstream.validations.lock().expect("script").is_empty());
        assert_eq!(fixture.token().await.as_deref(), Some("SAPISID=fresh"));
    }

    #[tokio::test]
    async fn a_probe_whose_recovery_fails_reports_the_session_expired() {
        let upstream = Arc::new(FakeUpstream::recovering_to(None));
        upstream
            .validations
            .lock()
            .expect("script")
            .extend([AuthOutcome::Expired, AuthOutcome::Valid]);
        let fixture = signed_in(upstream.clone()).await;

        let state = fixture.sources.validate_source("yt").await.expect("probe");

        assert_eq!(state, SourceState::AuthExpired);
        assert_eq!(fixture.sources.status("yt"), Some(SourceState::AuthExpired));
        assert_eq!(
            upstream.validations.lock().expect("script").len(),
            1,
            "not probed again without a recovered session"
        );
    }

    #[tokio::test]
    async fn an_import_records_the_profile_it_came_from() {
        let fixture = signed_in(Arc::new(FakeUpstream::default())).await;
        let profile = "brave:/profiles/Default";
        let stored_profile = || async {
            let server = fixture
                .db
                .load_server("yt")
                .await
                .expect("load")
                .expect("server");
            let saved = fixture
                .sources
                .current()
                .await
                .servers
                .into_iter()
                .find(|saved| saved.id == "yt")
                .expect("saved");
            assert_eq!(saved.yt_profile, server.yt_profile);
            server.yt_profile
        };

        fixture
            .sources
            .store_import(
                "yt",
                profile,
                config::Browser::Brave,
                "SAPISID=imported".into(),
            )
            .await
            .expect("import");
        assert_eq!(stored_profile().await.as_deref(), Some(profile));

        fixture
            .sources
            .provision_credentials(CredentialProvision {
                server_id: "yt".into(),
                secret: "SAPISID=rotated".into(),
                user_id: None,
                browser: None,
            })
            .await
            .expect("rotate");
        assert_eq!(
            stored_profile().await.as_deref(),
            Some(profile),
            "a rotation keeps where the session came from"
        );

        fixture
            .sources
            .provision_credentials(CredentialProvision {
                server_id: "yt".into(),
                secret: "SAPISID=signed-in".into(),
                user_id: None,
                browser: Some(config::Browser::Brave.id().to_string()),
            })
            .await
            .expect("sign in");
        assert_eq!(
            stored_profile().await,
            None,
            "a browser sign-in is not an import"
        );
    }
}
