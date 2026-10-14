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
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::{
    ApiError, CredentialProvision, ErrorCode, SourceCapabilities, SourceDraft, SourceFolderEntry,
    SourceInfo, SourceLoginRequest, SourceState, Table,
};
use server::source::AuthOutcome;
use server::sources::Sources;

use crate::config_service::ConfigService;
use crate::session::SessionHandle;

/// How long a browser sign-in may sit waiting for a person.
const SIGNIN_TIMEOUT: Duration = Duration::from_secs(300);

/// How often an active source that is not online is probed again, so it recovers without a switch.
const RETRY_PROBE: Duration = Duration::from_secs(60);

pub struct SourceService {
    db: db::Db,
    session: SessionHandle,
    config: Arc<ConfigService>,
    /// The authority on the sources; the session's copy follows it.
    current: tokio::sync::Mutex<Arc<Sources>>,
    /// Each source's last probe answer, tagged with the probe that wrote it.
    status: Mutex<HashMap<String, (u64, SourceState)>>,
    probes: AtomicU64,
}

fn db_error(error: db::DbError) -> ApiError {
    ApiError::internal(format!("database error: {error}"))
}

/// The stored sources, storing a device id the first time one is made.
pub async fn load(db: &db::Db) -> Result<Sources, db::DbError> {
    let stored = db.load_sources().await?;
    let fresh = stored.device_id.is_none();
    let sources = Sources::from(stored);
    if fresh {
        db.set_device_id(&sources.device_id).await?;
    }
    Ok(sources)
}

/// Write what moved between `before` and `after`; a removed server goes last, once nothing else names it.
async fn persist(db: &db::Db, before: &Sources, after: &Sources) -> Result<(), db::DbError> {
    for saved in &after.servers {
        if before.find_saved_server(&saved.id) != Some(saved) {
            db.save_server(saved).await?;
        }
    }
    if let Some(server) = &after.server
        && before.server.as_ref() != Some(server)
    {
        db.save_active_server(server).await?;
    }
    for (id, folders) in &after.folders {
        if before.folders.get(id) != Some(folders) {
            db.set_server_folders(id, folders).await?;
        }
    }
    for id in before.folders.keys() {
        if !after.folders.contains_key(id) {
            db.set_server_folders(id, &[]).await?;
        }
    }
    if before.active != after.active {
        db.set_active_source(&after.active).await?;
    }
    if before.device_id != after.device_id {
        db.set_device_id(&after.device_id).await?;
    }
    let active_id = after
        .server
        .as_ref()
        .and_then(|server| server.id.as_deref());
    for saved in &before.servers {
        if after.find_saved_server(&saved.id).is_none() && active_id != Some(saved.id.as_str()) {
            db.delete_server(&saved.id).await?;
        }
    }
    Ok(())
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
        uploads: caps.uploads,
        storage_quota: caps.storage_quota,
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

fn source_error(error: server::source::SourceError) -> ApiError {
    use server::source::SourceError;
    match &error {
        SourceError::Unsupported(what) => ApiError::unsupported(*what),
        SourceError::Auth => ApiError::new(ErrorCode::SourceAuthExpired, error.to_string()),
        SourceError::Connectivity => ApiError::new(ErrorCode::SourceUnreachable, error.to_string()),
        SourceError::InvalidInput(message) => ApiError::invalid_input(message.clone()),
        SourceError::Backend(message) => ApiError::internal(message.clone()),
    }
}

impl SourceService {
    pub async fn storage_quota(&self, id: &str) -> Result<api::StorageQuota, ApiError> {
        let (_, source) = self.resolve(id).await?;
        let quota = source.storage_quota().await.map_err(source_error)?;
        Ok(api::StorageQuota {
            used_bytes: quota.used_bytes,
            quota_bytes: quota.quota_bytes,
            remaining_bytes: quota.remaining_bytes,
        })
    }

    pub async fn upload_track(&self, upload: api::TrackUpload) -> Result<(), ApiError> {
        if upload.content.is_empty() || upload.content.len() > api::MAX_MUSIC_UPLOAD_BYTES {
            return Err(ApiError::invalid_input(
                "audio files must be between 1 byte and 500 MiB",
            ));
        }
        let (_, source) = self.resolve(&upload.source_id).await?;
        source
            .upload_track(upload.filename, upload.content)
            .await
            .map_err(source_error)
    }

    pub fn new(
        db: db::Db,
        session: SessionHandle,
        config: Arc<ConfigService>,
        sources: Arc<Sources>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            config,
            current: tokio::sync::Mutex::new(sources),
            status: Mutex::new(HashMap::new()),
            probes: AtomicU64::new(0),
        })
    }

    pub async fn snapshot(&self) -> Arc<Sources> {
        self.current.lock().await.clone()
    }

    /// Change the sources, persist what moved, and pass them on while still locked, so the session sees changes in the order they were saved.
    pub async fn mutate(
        &self,
        mutate: impl FnOnce(&mut Sources) + Send,
    ) -> Result<Arc<Sources>, ApiError> {
        let mut held = self.current.lock().await;
        let mut next = Sources::clone(&held);
        mutate(&mut next);
        if next == **held {
            return Ok(held.clone());
        }
        persist(&self.db, &held, &next).await.map_err(db_error)?;
        let next = Arc::new(next);
        *held = next.clone();
        self.session
            .set_active_source(Some(self.build_active(&next)));
        self.session.set_sources(next.clone());
        Ok(next)
    }

    fn build_active(&self, sources: &Sources) -> server::source::ActiveSource {
        Arc::from(server::source::active(self.db.clone(), sources))
    }

    /// Rebuild the active media source after the folder sources changed beside it.
    async fn refresh_active(&self) {
        let held = self.current.lock().await;
        self.session
            .set_active_source(Some(self.build_active(&held)));
    }

    /// Remember that the user picked a source, which only the first pick has to write.
    async fn mark_chosen(&self) -> Result<(), ApiError> {
        if self.current().await.source_explicitly_set {
            return Ok(());
        }
        self.config
            .mutate_state(&["source_explicitly_set"], |config| {
                config.source_explicitly_set = true;
            })
            .await?;
        Ok(())
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

    /// The sources as they would be with `id` active and the source built from them, to describe one without switching to it.
    async fn resolve(&self, id: &str) -> Result<(Sources, server::source::ActiveSource), ApiError> {
        let mut sources = Sources::clone(&*self.snapshot().await);
        match config::Source::from_column(id) {
            config::Source::LocalLibrary(local_id) => {
                if !self
                    .current()
                    .await
                    .local_sources
                    .iter()
                    .any(|saved| saved.id == local_id)
                {
                    return Err(ApiError::not_found("no such local source"));
                }
                sources.set_active_local(config::Source::LocalLibrary(local_id));
            }
            config::Source::Server(server_id) => {
                let server = self
                    .db
                    .load_server(&server_id)
                    .await
                    .map_err(db_error)?
                    .ok_or_else(|| ApiError::not_found("no such server"))?;
                sources.set_active_server(server);
            }
        }
        let active = self.build_active(&sources);
        Ok((sources, active))
    }

    pub async fn sources(&self) -> Result<Vec<SourceInfo>, ApiError> {
        let config = self.current().await;
        let servers = self.snapshot().await;
        let ids: Vec<String> = config
            .local_sources
            .iter()
            .map(|source| source.id.clone())
            .chain(servers.servers.iter().map(|server| server.id.clone()))
            .collect();
        let mut sources = Vec::with_capacity(ids.len());
        for id in ids {
            sources.push(self.source_info(&id).await?);
        }
        Ok(sources)
    }

    pub async fn source_info(&self, id: &str) -> Result<SourceInfo, ApiError> {
        let current = self.current().await;
        let active = self.snapshot().await.active.clone();
        let (resolved, source) = self.resolve(id).await?;
        let key = resolved.active.clone();
        let mut info = SourceInfo {
            id: key.as_str().to_string(),
            active: active.as_str() == key.as_str(),
            capabilities: capabilities(source.capabilities()),
            state: self.status(key.as_str()),
            ..Default::default()
        };
        match &key {
            config::Source::LocalLibrary(local_id) => {
                let saved = current
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
        let previous = self.snapshot().await.active.clone();
        let (target, _) = self.resolve(id).await?;
        let source = target.active.clone();
        let changed = previous != source;
        let server = target.server.clone();
        self.mark_chosen().await?;
        self.mutate(move |sources| match source {
            config::Source::LocalLibrary(_) => sources.set_active_local(source),
            config::Source::Server(_) => {
                if let Some(server) = server {
                    sources.set_active_server(server);
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
        self.refresh_active().await;
        self.session.invalidate(Table::Servers);
        self.source_info(&id).await
    }

    async fn delete_folder_source(&self, id: &str) -> Result<(), ApiError> {
        self.config.ensure_unlocked(&["local_sources"])?;
        if id == config::DEFAULT_LOCAL_ID {
            return Err(ApiError::invalid_input(
                "the default folder source cannot be deleted",
            ));
        }
        let current = self.current().await;
        if !current.local_sources.iter().any(|source| source.id == id) {
            return Err(ApiError::not_found("no such source"));
        }
        let was_active = self.snapshot().await.active.local_library_id() == Some(id);
        let id_owned = id.to_string();
        let keys: &[&str] = if was_active {
            &["local_sources", "source_explicitly_set"]
        } else {
            &["local_sources"]
        };
        // The folder list goes first, so the session forgets the queue of the source it leaves rather than parking it.
        self.config
            .mutate_state(keys, move |config| {
                config.remove_local_source(&id_owned);
                if was_active {
                    config.source_explicitly_set = true;
                }
            })
            .await?;
        // The session drops this source's queue itself once its pending saves land, so none outlives the purge.
        if was_active {
            self.mutate(Sources::clear_active).await?;
        } else {
            self.refresh_active().await;
        }
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
        self.refresh_active().await;
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
        let current = self.snapshot().await;
        // Changing where the active server points invalidates whatever is
        // loaded from it, so playback stops rather than carrying on against
        // the old backend.
        let backend_changed = current.active.server_id() == Some(id.as_str())
            && current
                .server
                .as_ref()
                .is_some_and(|server| server.service != saved.service || server.url != saved.url);
        self.mutate(move |sources| {
            match sources
                .servers
                .iter_mut()
                .find(|entry| entry.id == saved.id)
            {
                Some(existing) => *existing = saved.clone(),
                None => sources.servers.push(saved.clone()),
            }
            if sources.active.server_id() == Some(saved.id.as_str())
                && let Some(server) = sources.server.as_mut()
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

    /// Answer a source's own options, leaving absent keys alone; the Spotify ones are settings, so they go through the config first.
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
        let current = self.snapshot().await;
        let Some(existing) = current.find_saved_server(id) else {
            return Err(ApiError::not_found("no such server"));
        };
        let config_keys = crate::services::config_keys(existing, &values);
        let service = existing.service;
        let folders = api::schema::value_of(&values, crate::services::DIRECTORIES)
            .map(api::schema::decode_directories);
        if let Some(folders) = &folders
            && folders.iter().any(|path| path.trim().is_empty())
        {
            return Err(ApiError::invalid_input(
                "a source directory cannot be empty",
            ));
        }
        self.config.ensure_unlocked(&config_keys)?;
        if !config_keys.is_empty() {
            let answers = values.clone();
            self.config
                .mutate_state(&config_keys, move |config| {
                    crate::services::apply_config_settings(service, &answers, config);
                })
                .await?;
        }
        let target = id.to_string();
        self.mutate(move |sources| {
            let Some(index) = sources
                .servers
                .iter()
                .position(|server| server.id == target)
            else {
                return;
            };
            let mut saved = sources.servers[index].clone();
            crate::services::apply_server_settings(&values, &mut saved);
            sources.servers[index] = saved.clone();
            if let Some(folders) = folders {
                sources.set_folders_for(&saved.id, folders);
            }
            if sources.active.server_id() == Some(saved.id.as_str())
                && let Some(server) = sources.server.as_mut()
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
        let current = self.snapshot().await;
        let service = current
            .servers
            .iter()
            .find(|server| server.id == id)
            .map(|server| server.service);
        let was_active = current.active.server_id() == Some(id);
        let id_owned = id.to_string();
        if was_active {
            self.mark_chosen().await?;
        }
        self.mutate(move |sources| {
            sources.remove_saved_server(&id_owned);
            if was_active {
                sources.clear_active();
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
            .snapshot()
            .await
            .active
            .server_id()
            .is_some_and(|id| id == provision.server_id);
        let token = server.access_token.clone();
        let user = server.user_id.clone();
        let saved = config::SavedServer::from_music_server(&server);
        let live = server.clone();
        self.mutate(move |sources| {
            match sources
                .servers
                .iter_mut()
                .find(|entry| entry.id == saved.id)
            {
                Some(existing) => *existing = saved,
                None => sources.servers.push(saved),
            }
            if active {
                sources.server = Some(live);
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
        if request.username.trim().is_empty() || request.password.is_empty() {
            return Err(ApiError::invalid_input(
                "a username and password are required",
            ));
        }
        let device_id = self.snapshot().await.device_id.clone();
        let server = self
            .db
            .load_server(&request.server_id)
            .await
            .map_err(db_error)?
            .ok_or_else(|| ApiError::not_found("no such server"))?;
        let auth = if server.service == config::MusicService::Clippsly {
            let client = server::clippsly::ClippslyClient::new(&server.url, &request.password)
                .map_err(source_error)?;
            let account = client.account().await.map_err(source_error)?;
            server::provider::AuthSession {
                access_token: request.password,
                user_id: account.id.to_string(),
            }
        } else {
            server::provider::ProviderClient::new(server.service, server.url, device_id)
                .login(request.username.trim(), &request.password)
                .await
                .map_err(|error| ApiError::new(ErrorCode::SourceAuthExpired, error))?
        };
        self.provision_credentials(CredentialProvision {
            server_id: request.server_id,
            secret: auth.access_token,
            user_id: Some(auth.user_id),
            browser: None,
        })
        .await
    }

    pub async fn clear_credentials(&self, id: &str) -> Result<(), ApiError> {
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
            .snapshot()
            .await
            .active
            .server_id()
            .is_some_and(|server_id| server_id == id);
        self.db
            .set_server_credentials(id, None, None)
            .await
            .map_err(db_error)?;
        if active {
            self.mutate(move |sources| sources.server = Some(server))
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
        let (_, source) = self.resolve(id).await?;
        let state = match source.validate().await {
            AuthOutcome::Valid => SourceState::Online,
            AuthOutcome::Expired => SourceState::AuthExpired,
            AuthOutcome::Unreachable => SourceState::Offline,
        };
        self.record_status(id, probe, state);
        Ok(state)
    }

    /// Probe the active source whenever it or its server entry changes, and retry one that is not online.
    pub fn watch_active(self: &Arc<Self>, mut sources: tokio::sync::watch::Receiver<Arc<Sources>>) {
        let service = self.clone();
        tokio::spawn(async move {
            let key = |sources: &Sources| (sources.active.clone(), sources.server.clone());
            let mut probed: Option<(config::Source, Option<config::MusicServer>)> = None;
            let mut retry = tokio::time::interval(RETRY_PROBE);
            retry.tick().await;
            let mut retry_due = false;
            loop {
                let next = key(&sources.borrow_and_update());
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
                    changed = sources.changed() => {
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
        let Some((cookies, id)) = ytmusic_rotation(&*self.snapshot().await) else {
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
        let sources = self.snapshot().await;
        let Some(server) = sources
            .server
            .as_ref()
            .filter(|server| server.service == config::MusicService::Spotify)
        else {
            return;
        };
        let (Some(packed), Some(id)) = (
            server.access_token.clone(),
            sources.active.server_id().map(str::to_string),
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

fn ytmusic_rotation(sources: &server::sources::Sources) -> Option<(String, String)> {
    let cookies = sources.server.as_ref()?.ytmusic_cookies()?.to_string();
    let id = sources.active.server_id()?.to_string();
    Some((cookies, id))
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

    fn youtube_music(anonymous: bool) -> server::sources::Sources {
        let mut server = config::MusicServer::new_with_service(
            "YouTube Music".into(),
            "https://music.youtube.com".into(),
            config::MusicService::YtMusic,
        );
        server.id = Some("yt".into());
        server.access_token = Some("SAPISID=leftover".into());
        server.yt_anonymous = anonymous;
        server::sources::Sources {
            server: Some(server),
            active: config::Source::Server("yt".into()),
            ..Default::default()
        }
    }

    /// Switching to anonymous keeps the old sign-in's row; rotating it would keep that session alive.
    #[test]
    fn an_anonymous_source_never_rotates_a_leftover_token() {
        assert_eq!(ytmusic_rotation(&youtube_music(true)), None);
        assert_eq!(
            ytmusic_rotation(&youtube_music(false)),
            Some(("SAPISID=leftover".into(), "yt".into()))
        );
    }

    struct NoTracks;

    #[async_trait::async_trait]
    impl crate::session::QueueMaterializer for NoTracks {
        async fn materialize(&self, _: &api::QueueContext) -> Result<Vec<reader::Track>, ApiError> {
            Ok(Vec::new())
        }
    }

    async fn reaches_the_session(session: &SessionHandle, active: &config::Source) {
        let mut sources = session.sources_watch();
        tokio::time::timeout(
            Duration::from_secs(5),
            sources.wait_for(|s| s.active == *active),
        )
        .await
        .expect("the session adopts the change")
        .expect("the session is running");
    }

    #[tokio::test]
    async fn a_switch_and_a_deleted_active_folder_source_are_stored_and_reach_the_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("sources.db")).await.expect("db");
        let mut seeded = config::AppConfig::default();
        seeded.local_sources.push(config::SavedLocalSource {
            id: "local:b".into(),
            name: "B".into(),
            directories: Vec::new(),
        });
        let config = Arc::new(ConfigService::new(
            database.clone(),
            dir.path().join("settings.toml"),
            seeded.clone(),
        ));
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("headless player starts");
        let session = SessionHandle::spawn_with_player(
            Arc::new(NoTracks),
            player,
            crate::PlaybackServices {
                config: seeded,
                ..Default::default()
            },
        );
        config.attach_session(session.clone());
        let service = SourceService::new(
            database.clone(),
            session.clone(),
            config.clone(),
            session.sources(),
        );
        let b = config::Source::LocalLibrary("local:b".into());

        let info = service.switch_source("local:b").await.expect("switch");
        assert!(info.active);
        assert_eq!(database.load_sources().await.expect("load").active, b);
        assert!(config.snapshot().await.source_explicitly_set);
        reaches_the_session(&session, &b).await;

        service.delete_source("local:b").await.expect("delete");
        let fallback = config::Source::default();
        assert_eq!(
            database.load_sources().await.expect("load").active,
            fallback
        );
        assert!(
            !config
                .snapshot()
                .await
                .local_sources
                .iter()
                .any(|saved| saved.id == "local:b")
        );
        reaches_the_session(&session, &fallback).await;
    }
}
