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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use api::{
    ApiError, ApiEvent, BrowserSession, CredentialProvision, ErrorCode, ErrorReason, NetworkStatus,
    OfflineMode, SourceCapabilities, SourceDraft, SourceFolderEntry, SourceInfo,
    SourceLoginRequest, SourceState, Table,
};
use server::source::{AuthOutcome, MediaSource};
use server::sources::Sources;

use crate::config_service::ConfigService;
use crate::network::{self, Verdict};
use crate::recovery::{Recovery, once_more};
use crate::session::SessionHandle;

/// How long a browser sign-in may sit waiting for a person.
const SIGNIN_TIMEOUT: Duration = Duration::from_secs(300);

const LOCAL_SOURCES: [config::SettingKey; 1] = [config::SettingKey::Library(
    config::LibraryKey::LocalSources,
)];

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
    upstream: Arc<dyn Upstream>,
    recovery: Recovery,
    /// The core's runtime. The desktop UI calls this service in-process on
    /// its own thread, and work that blocks has to run here instead.
    runtime: tokio::runtime::Handle,
    /// Itself, for the network checks a failed request starts in the background.
    me: Weak<SourceService>,
    network: Mutex<NetworkStatus>,
    checker: network::Checker,
    rechecking: AtomicBool,
}

#[async_trait::async_trait]
pub(crate) trait Upstream: Send + Sync {
    async fn validate(&self, source: &dyn MediaSource) -> AuthOutcome;

    /// The stored cookies after one `verify_session` rotation, else a fresh read of
    /// the profile it signed in with.
    async fn recover_ytmusic(&self, server: &config::MusicServer, id: &str) -> Option<String>;

    /// Ask the connectivity check addresses whether the internet is there.
    async fn internet(&self, urls: &[String]) -> Verdict;
}

struct Network;

#[async_trait::async_trait]
impl Upstream for Network {
    async fn validate(&self, source: &dyn MediaSource) -> AuthOutcome {
        source.validate().await
    }

    async fn internet(&self, urls: &[String]) -> Verdict {
        let Some(client) = network::check_client() else {
            return Verdict::Unchecked;
        };
        network::check(urls, |url| {
            let client = client.clone();
            async move { network::probe(&client, url).await }
        })
        .await
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
fn capabilities(source: &dyn server::source::MediaSource) -> SourceCapabilities {
    use api::{AlbumPresentation, ArtistPresentation, FavoritesSyncMode, PlaylistCapability};
    use server::source::{AlbumType, ArtistView, FavoritesSync, PlaylistOps};
    let caps = source.capabilities();
    SourceCapabilities {
        edit_tags: caps.edit_tags,
        delete_from_disk: caps.delete_from_disk,
        scan_folders: caps.scan_folders,
        folders: caps.folders,
        browse_folders: caps.browse_folders,
        external_devices: caps.external_devices,
        browser_playback: caps.browser_playback,
        accounts: caps.accounts,
        stream_quality: caps.stream_quality,
        explicit_flags: caps.explicit_flags,
        watch_history: caps.watch_history,
        complete_play_history: caps.complete_play_history,
        sync: caps.sync,
        downloads: caps.downloads,
        uploads: caps.uploads,
        storage_quota: caps.storage_quota,
        discover: caps.discover,
        dont_recommend: caps.dont_recommend,
        rate: caps.library_actions.rate,
        follow: caps.library_actions.follow,
        save: caps.library_actions.save,
        remove_from_history: caps.library_actions.remove_from_history,
        playlist_details: caps.library_actions.playlist_details,
        music_videos: caps.music_videos,
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
        pages: source
            .catalog_pages()
            .into_iter()
            .map(|page| api::PageEntry {
                id: page.id,
                label: api::Text::key(page.label),
                icon: api::Icon::Class(page.icon.to_string()),
            })
            .collect(),
        search_filters: source
            .search_filters()
            .into_iter()
            .map(|filter| api::SearchFilter {
                id: filter.id.to_string(),
                label: api::Text::key(filter.label),
            })
            .collect(),
    }
}

pub(crate) fn source_error(error: server::source::SourceError) -> ApiError {
    use server::source::SourceError;
    match error {
        SourceError::Unsupported(what) => ApiError::unsupported(what),
        SourceError::InvalidInput(message) => ApiError::invalid_input(message),
        SourceError::Backend(message) => ApiError::internal(message),
        SourceError::Auth | SourceError::Connectivity => {
            ApiError::new(source_error_code(&error), error.to_string())
        }
        SourceError::Refused(_, ref message) => {
            ApiError::new(source_error_code(&error), message.clone())
        }
    }
}

/// The wire code for a source failure, for paths that carry only a code and
/// a message, such as a failed track load.
pub(crate) fn source_error_code(error: &server::source::SourceError) -> ErrorCode {
    use server::source::{Refusal, SourceError};
    match error {
        SourceError::Unsupported(_) => ErrorCode::Unsupported,
        SourceError::InvalidInput(_) => ErrorCode::InvalidInput,
        SourceError::Backend(_) => ErrorCode::Internal,
        SourceError::Auth | SourceError::Refused(Refusal::SessionExpired, _) => {
            ErrorCode::SourceAuthExpired
        }
        SourceError::Connectivity | SourceError::Refused(Refusal::Network, _) => {
            ErrorCode::SourceUnreachable
        }
        SourceError::Refused(Refusal::Challenged, _) => ErrorCode::SourceChallenged,
        SourceError::Refused(Refusal::ServiceRefused, _) => ErrorCode::SourceRefused,
        SourceError::Refused(Refusal::Unavailable, _) => ErrorCode::ItemUnavailable,
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
        Self::with_upstream(db, session, config, sources, Arc::new(Network))
    }

    pub(crate) fn with_upstream(
        db: db::Db,
        session: SessionHandle,
        config: Arc<ConfigService>,
        sources: Arc<Sources>,
        upstream: Arc<dyn Upstream>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            db,
            session,
            config,
            current: tokio::sync::Mutex::new(sources),
            status: Mutex::new(HashMap::new()),
            probes: AtomicU64::new(0),
            upstream,
            recovery: Recovery::default(),
            runtime: tokio::runtime::Handle::current(),
            me: me.clone(),
            network: Mutex::new(NetworkStatus::default()),
            checker: network::Checker::default(),
            rechecking: AtomicBool::new(false),
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
        let adopted = self.session.adopt_sources(next.clone());
        drop(held);
        // A caller told of the change reads it back at once, so the library reads have to have moved first.
        let _ = adopted.await;
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

    async fn current(&self) -> config::Settings {
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
                    .library
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
            .library
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
            capabilities: capabilities(source.as_ref()),
            state: self.status(key.as_str()),
            ..Default::default()
        };
        match &key {
            config::Source::LocalLibrary(local_id) => {
                let saved = current
                    .library
                    .local_sources
                    .iter()
                    .find(|saved| saved.id == *local_id)
                    .ok_or_else(|| ApiError::not_found("no such local source"))?;
                info.name = saved.name.clone();
                info.service = crate::services::folders_ref();
                info.authenticated = true;
                info.works_offline = true;
                info.permanent = saved.id == config::DEFAULT_LOCAL_ID;
                let paths: Vec<String> = saved
                    .directories
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                info.settings = vec![crate::services::folder_source_field(&paths)];
            }
            config::Source::Server(server_id) => {
                let server = resolved
                    .server
                    .as_ref()
                    .ok_or_else(|| ApiError::not_found("no such server"))?;
                let view = crate::services::ServerView::from(server);
                info.name = server.name.clone();
                info.needs_network = true;
                info.works_offline = network::is_local(&key, Some(&server.url));
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
                let signed_in = server
                    .access_token
                    .as_deref()
                    .is_some_and(|t| !t.is_empty());
                if info.active
                    && signed_in
                    && !server.yt_anonymous
                    && source.capabilities().account_avatar
                {
                    info.avatar = Some(crate::artwork::account_ref(
                        &info.id,
                        server.user_id.as_deref().unwrap_or_default(),
                    ));
                }
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

    /// A switch someone asked for; while the internet is down, only to a source that works without it.
    pub async fn switch_source(&self, id: &str) -> Result<SourceInfo, ApiError> {
        if self.network_status().internet_down {
            let (target, _) = self.resolve(id).await?;
            let url = target.server.as_ref().map(|server| server.url.as_str());
            if !network::is_local(&target.active, url)
                && self.check_network_spaced(network::NUDGE_SPACING).await == Verdict::Down
            {
                return Err(ApiError::new(
                    ErrorCode::SourceUnreachable,
                    "the internet is down, and only sources on this machine or its network work without it",
                )
                .because(ErrorReason::NetworkDown));
            }
        }
        let info = self.switch_to(id).await?;
        self.forget_left_source(None);
        Ok(info)
    }

    async fn switch_to(&self, id: &str) -> Result<SourceInfo, ApiError> {
        let previous = self.snapshot().await.active.clone();
        let (target, _) = self.resolve(id).await?;
        let source = target.active.clone();
        let changed = previous != source;
        let server = target.server.clone();
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
        self.config.ensure_unlocked(&LOCAL_SOURCES).await?;
        if let Some(problem) = crate::services::check_folders(&draft).first() {
            return Err(problem.error());
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
            .mutate_state(&LOCAL_SOURCES, move |config| {
                match config
                    .library
                    .local_sources
                    .iter_mut()
                    .find(|source| source.id == saved.id)
                {
                    Some(existing) => *existing = saved,
                    None => config.library.local_sources.push(saved),
                }
            })
            .await?;
        self.refresh_active().await;
        self.session.invalidate(Table::Servers);
        self.source_info(&id).await
    }

    async fn delete_folder_source(&self, id: &str) -> Result<(), ApiError> {
        self.config.ensure_unlocked(&LOCAL_SOURCES).await?;
        if id == config::DEFAULT_LOCAL_ID {
            return Err(
                ApiError::invalid_input("the default folder source cannot be deleted")
                    .because(ErrorReason::DefaultSourceUndeletable),
            );
        }
        let current = self.current().await;
        if !current
            .library
            .local_sources
            .iter()
            .any(|source| source.id == id)
        {
            return Err(ApiError::not_found("no such source"));
        }
        let was_active = self.snapshot().await.active.local_library_id() == Some(id);
        let id_owned = id.to_string();
        // The folder list goes first, so the session forgets the queue of the source it leaves rather than parking it.
        self.config
            .mutate_state(&LOCAL_SOURCES, move |config| {
                config.library.remove_local_source(&id_owned);
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
            return Err(
                ApiError::invalid_input("a source directory cannot be empty")
                    .because(ErrorReason::EmptySourceFolder),
            );
        }
        self.config.ensure_unlocked(&LOCAL_SOURCES).await?;
        let current = self.current().await;
        if !current
            .library
            .local_sources
            .iter()
            .any(|saved| saved.id == id)
        {
            return Err(ApiError::not_found("no such source"));
        }
        let target = id.to_string();
        self.config
            .mutate_state(&LOCAL_SOURCES, move |config| {
                if let Some(saved) = config
                    .library
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
                problems: crate::services::problems(&crate::services::check_folders(&draft)),
            });
        }
        let service = Self::drafted_service(&draft)?;
        let (sign_in, problems) = crate::services::check(service, &draft);
        Ok(api::DraftCheck {
            sign_in,
            problems: crate::services::problems(&problems),
        })
    }

    pub async fn probe_source_draft(
        &self,
        draft: SourceDraft,
    ) -> Result<api::DraftProbe, ApiError> {
        use crate::services::{PROBE_FILE_CAP, PROBE_WALK_BUDGET, ProbeTarget};
        match crate::services::probe_target(&draft)? {
            ProbeTarget::Folders(paths) => {
                let walk = move || {
                    paths
                        .iter()
                        .map(|path| {
                            let deadline = std::time::Instant::now() + PROBE_WALK_BUDGET;
                            crate::services::probe_folder(path, PROBE_FILE_CAP, deadline)
                        })
                        .collect()
                };
                let probes = self.runtime.spawn_blocking(walk).await.map_err(|error| {
                    ApiError::internal(format!("a folder probe stopped: {error}"))
                })?;
                Ok(api::DraftProbe::Folders(probes))
            }
            ProbeTarget::Server(url) => {
                let client = crate::services::probe_client()?;
                Ok(api::DraftProbe::Server(
                    crate::services::probe_server(&client, &url).await,
                ))
            }
            ProbeTarget::Nothing => Ok(api::DraftProbe::NotProbeable),
        }
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
        }?;
        self.forget_left_source(Some(id));
        self.session.offline().cache().forget_source(id).await;
        Ok(())
    }

    async fn upsert_server(&self, draft: SourceDraft) -> Result<SourceInfo, ApiError> {
        let service = Self::drafted_service(&draft)?;
        let (_, problems) = crate::services::check(service, &draft);
        if let Some(problem) = problems.first() {
            return Err(problem.error());
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
                // The form carries neither the imported profile nor the
                // account choice, so an edit keeps them unless the source
                // now speaks another service.
                Some(existing) => {
                    let same_service = existing.service == saved.service;
                    let profile = existing.yt_profile.take().filter(|_| same_service);
                    let account = existing.account.take().filter(|_| same_service);
                    *existing = saved.clone();
                    existing.yt_profile = profile;
                    existing.account = account;
                }
                None => sources.servers.push(saved.clone()),
            }
            if sources.active.server_id() == Some(saved.id.as_str())
                && let Some(server) = sources.server.as_mut()
            {
                if server.service != saved.service {
                    server.account = None;
                }
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
            return Err(
                ApiError::invalid_input("a source directory cannot be empty")
                    .because(ErrorReason::EmptySourceFolder),
            );
        }
        self.config.ensure_unlocked(&config_keys).await?;
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
        self.store_credentials(provision, None).await
    }

    /// A browser sign-in names no profile, which forgets an earlier import's.
    async fn store_credentials(
        &self,
        provision: CredentialProvision,
        profile: Option<String>,
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
            // Signed in through a browser, so no longer anonymous.
            server.yt_anonymous = false;
            server.yt_profile = profile;
        }
        // A different login owns none of the old one's brand accounts.
        let user_changed = previous_user != server.user_id;
        if user_changed {
            server.account = None;
        }
        let active = self
            .snapshot()
            .await
            .active
            .server_id()
            .is_some_and(|id| id == provision.server_id);
        let token = server.access_token.clone();
        let user = server.user_id.clone();
        let mut saved = config::SavedServer::from_music_server(&server);
        let mut live = server.clone();
        self.mutate(move |sources| {
            match sources
                .servers
                .iter_mut()
                .find(|entry| entry.id == saved.id)
            {
                Some(existing) => {
                    // `server` was loaded before the lock, so a switch_account in between wins.
                    if !user_changed {
                        saved.account = existing.account.clone();
                        live.account = existing.account.clone();
                    }
                    *existing = saved;
                }
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
        if user_changed {
            self.db
                .discard_pending_favorites(&provision.server_id)
                .await
                .map_err(db_error)?;
        }
        // A different account is a different library, so what is loaded
        // from the old one stops.
        if active && user_changed {
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
                .map_err(source_error)?
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
            // Kept when an existing session resumed, so the next expiry can
            // still re-read the imported profile.
            let mut profile = None;
            let (secret, user_id) = match server.service {
                config::MusicService::YtMusic => {
                    let (secret, fresh) = ensure_ytmusic_signed_in(
                        server.access_token.clone(),
                        browser,
                        id,
                        server.yt_profile.as_deref(),
                    )
                    .await
                    .map_err(ApiError::internal)?;
                    if !fresh {
                        profile = server.yt_profile.clone();
                    }
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
            self.store_credentials(
                CredentialProvision {
                    server_id: id.to_string(),
                    secret,
                    user_id: Some(user_id),
                    browser: Some(browser.id().to_string()),
                },
                profile,
            )
            .await
        }
    }

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

    /// Like a browser sign-in, the secret never leaves this process.
    pub async fn import_browser_session(
        &self,
        id: &str,
        session: &str,
    ) -> Result<SourceInfo, ApiError> {
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
                .map_err(|error| {
                    use server::ytmusic::browser_sessions::ImportError;
                    let reason = match error {
                        ImportError::ProfileGone => ErrorReason::BrowserProfileGone,
                        ImportError::CookiesUnreadable(_) => ErrorReason::BrowserCookiesUnreadable,
                        ImportError::SignedOut { .. } => ErrorReason::BrowserProfileSignedOut,
                    };
                    ApiError::invalid_input(error.to_string()).because(reason)
                })?;
            let secret = try_resume_ytmusic(Some(cookies)).await.ok_or_else(|| {
                ApiError::invalid_input(format!(
                    "YouTube Music did not accept the session from {}. Open music.youtube.com there to refresh it, then try again.",
                    browser.label()
                ))
                .because(ErrorReason::BrowserProfileSignedOut)
            })?;
            self.store_import(id, session, browser, secret).await
        }
    }

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
            .map_err(source_error)?;
        Ok(paths
            .into_iter()
            .map(|path| SourceFolderEntry {
                name: server::nextcloud::folder_name(&path).to_string(),
                path,
            })
            .collect())
    }

    pub async fn accounts(&self, id: &str) -> Result<Vec<api::SourceAccount>, ApiError> {
        let (_, source) = self.resolve(id).await?;
        if !source.capabilities().accounts {
            return Err(ApiError::unsupported(
                "this source has no accounts to switch between",
            ));
        }
        let accounts = source
            .accounts()
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?;
        Ok(accounts
            .into_iter()
            .map(|account| api::SourceAccount {
                id: account.id,
                name: account.name,
                handle: account.handle,
                active: account.active,
            })
            .collect())
    }

    /// Run the source's own connection test. Unlike a probe it records
    /// nothing and recovers nothing: it reports what is there.
    pub async fn test_connection(&self, id: &str) -> Result<Vec<api::ConnectionCheck>, ApiError> {
        use server::source::{CheckOutcome, CheckStep};
        let (_, source) = self.resolve(id).await?;
        // The stream steps run YouTube's player JS in V8 synchronously, which
        // froze the window when it ran on the caller's thread.
        let checks = self
            .runtime
            .spawn(async move { source.diagnose().await })
            .await
            .map_err(|error| ApiError::internal(format!("connection test stopped: {error}")))?;
        Ok(checks
            .into_iter()
            .map(|check| api::ConnectionCheck {
                step: match check.step {
                    CheckStep::Reach => api::CheckStep::Reach,
                    CheckStep::Session => api::CheckStep::Session,
                    CheckStep::SignedInPlayback => api::CheckStep::SignedInPlayback,
                    CheckStep::AnonymousPlayback => api::CheckStep::AnonymousPlayback,
                    CheckStep::PlaybackToken => api::CheckStep::PlaybackToken,
                },
                outcome: match check.outcome {
                    CheckOutcome::Passed => api::CheckOutcome::Passed,
                    CheckOutcome::Failed => api::CheckOutcome::Failed,
                    CheckOutcome::Skipped => api::CheckOutcome::Skipped,
                },
                detail: check.detail,
            })
            .collect())
    }

    /// True when the active source's account changed and its library needs a resync.
    pub async fn switch_account(
        &self,
        id: &str,
        account: Option<String>,
    ) -> Result<(SourceInfo, bool), ApiError> {
        let (_, source) = self.resolve(id).await?;
        if !source.capabilities().accounts {
            return Err(ApiError::unsupported(
                "this source has no accounts to switch between",
            ));
        }
        let account = account.filter(|account| !account.is_empty());
        let current = self.snapshot().await;
        let previous = current
            .find_saved_server(id)
            .ok_or_else(|| ApiError::not_found("no such server"))?
            .account
            .clone();
        if previous == account {
            return Ok((self.source_info(id).await?, false));
        }
        let active = current.active.server_id() == Some(id);
        let target = id.to_string();
        // Likes queued under the old account would otherwise be pushed to the new one.
        self.db
            .discard_pending_favorites(id)
            .await
            .map_err(db_error)?;
        self.mutate(move |sources| {
            if let Some(saved) = sources.servers.iter_mut().find(|s| s.id == target) {
                saved.account.clone_from(&account);
            }
            if sources.active.server_id() == Some(target.as_str())
                && let Some(server) = sources.server.as_mut()
            {
                server.account = account;
            }
        })
        .await?;
        if active {
            self.session.reset_playback().await?;
            self.finish_source_change();
        } else {
            self.session.invalidate(Table::Servers);
        }
        Ok((self.source_info(id).await?, active))
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
        if state == SourceState::Offline {
            self.after_unreachable(id);
        }
        Ok(state)
    }

    pub async fn recovering<T, A>(&self, attempt: impl FnMut() -> A) -> Result<T, ApiError>
    where
        A: Future<Output = Result<T, ApiError>>,
    {
        let seen = self.recovery.epoch();
        let result = once_more(attempt, auth_expired, || self.recover_active(seen)).await;
        if self.network_status().internet_down {
            self.check_network_soon();
        } else if matches!(&result, Err(error) if error.code == ErrorCode::SourceUnreachable)
            && let Some(id) = self.snapshot().await.active.server_id()
        {
            self.after_unreachable(id);
        }
        result
    }

    pub fn network_status(&self) -> NetworkStatus {
        self.network
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn publish_network(&self) {
        self.session
            .emit_event(ApiEvent::NetworkStatus(self.network_status()));
    }

    /// The source to offer back is gone once someone picks a source, or once it is deleted.
    fn forget_left_source(&self, deleted: Option<&str>) {
        {
            let mut status = self.network.lock().unwrap_or_else(PoisonError::into_inner);
            match (&status.left_source, deleted) {
                (None, _) => return,
                (Some(left), Some(deleted)) if left != deleted => return,
                _ => status.left_source = None,
            }
        }
        self.publish_network();
    }

    /// A source failed to answer: ask about the internet in the background, if the source needs it and holds what it needs to answer.
    fn after_unreachable(&self, id: &str) {
        let Some(me) = self.me.upgrade() else {
            return;
        };
        let id = id.to_string();
        self.runtime.spawn(async move {
            if me.needs_internet(&id).await {
                me.check_network(false).await;
            }
        });
    }

    /// Someone used the app while the internet is down, which is a cue to look again now rather than at the next scheduled recheck.
    pub fn check_network_soon(&self) {
        if !self.network_status().internet_down {
            return;
        }
        let Some(me) = self.me.upgrade() else {
            return;
        };
        self.runtime.spawn(async move {
            me.check_network_spaced(network::NUDGE_SPACING).await;
        });
    }

    /// A server across the internet with a sign-in or anonymous access, so its silence says something about the connection.
    async fn needs_internet(&self, id: &str) -> bool {
        let source = config::Source::from_column(id);
        if source.server_id().is_none() {
            return false;
        }
        let Ok(Some(server)) = self.db.load_server(id).await else {
            return false;
        };
        let usable = server.access_token.is_some() || server.yt_anonymous;
        usable && !network::is_local(&source, Some(&server.url))
    }

    /// One spaced, shared connectivity check and what follows from it; `fresh` skips the spacing, as a scheduled recheck does.
    async fn check_network(&self, fresh: bool) -> Verdict {
        let spacing = match fresh {
            true => Duration::ZERO,
            false => network::CHECK_SPACING,
        };
        self.check_network_spaced(spacing).await
    }

    /// The check and what follows from it, joining one begun within `spacing`.
    async fn check_network_spaced(&self, spacing: Duration) -> Verdict {
        let urls = self.current().await.system.connectivity_check_urls;
        if urls.is_empty() {
            self.internet_came_back();
            return Verdict::Unchecked;
        }
        let upstream = self.upstream.clone();
        let verdict = self
            .checker
            .run(spacing, || async move { upstream.internet(&urls).await })
            .await;
        match verdict {
            Verdict::Down => self.internet_went_down().await,
            Verdict::Up | Verdict::Unchecked => self.internet_came_back(),
        }
        verdict
    }

    async fn internet_went_down(&self) {
        {
            let mut status = self.network.lock().unwrap_or_else(PoisonError::into_inner);
            if status.internet_down {
                return;
            }
            status.internet_down = true;
        }
        let sources = self.snapshot().await;
        let active = sources.active.clone();
        let url = sources.server.as_ref().map(|server| server.url.as_str());
        if !network::is_local(&active, url) {
            match self.local_fallback(&sources).await {
                Some(target) => match self.switch_to(target.as_str()).await {
                    Ok(_) => {
                        tracing::info!(
                            from = active.as_str(),
                            to = target.as_str(),
                            "the internet is down; moved to a source that works without it"
                        );
                        self.network
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .left_source = Some(active.as_str().to_string());
                    }
                    Err(error) => {
                        tracing::warn!(%error, "moving off a source that needs the internet failed")
                    }
                },
                None => self.enter_offline_mode(),
            }
        }
        self.publish_network();
        self.spawn_rechecks();
    }

    /// The best source that works without the internet and holds music; a library with nothing in it is no place to land.
    async fn local_fallback(&self, sources: &Sources) -> Option<config::Source> {
        for candidate in network::local_candidates(&self.current().await, &sources.servers) {
            let filter = db::TrackFilter {
                source: candidate.clone(),
                ..db::TrackFilter::default()
            };
            if self.db.tracks_count(&filter).await.is_ok_and(|n| n > 0) {
                return Some(candidate);
            }
        }
        None
    }

    /// No source works without the internet, so the active one stays and offline mode keeps it to the songs with a copy here.
    fn enter_offline_mode(&self) {
        {
            let mut status = self.network.lock().unwrap_or_else(PoisonError::into_inner);
            if status.offline_mode.is_on() {
                return;
            }
            status.offline_mode = OfflineMode::Automatic;
        }
        tracing::info!(
            "the internet is down and no source works without it; offline mode keeps to downloads and cached songs"
        );
        self.offline_mode_moved();
    }

    /// Enter offline mode at someone's asking, or leave it however it was entered; only this leaves it.
    pub fn set_offline(&self, on: bool) -> NetworkStatus {
        let next = match on {
            true => OfflineMode::Requested,
            false => OfflineMode::Off,
        };
        let moved = {
            let mut status = self.network.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::replace(&mut status.offline_mode, next).is_on() != next.is_on()
        };
        tracing::info!(on, "offline mode set");
        if moved {
            self.offline_mode_moved();
        } else {
            self.publish_network();
        }
        self.network_status()
    }

    /// Offline mode came on or went off: listings change what they hold, and every frontend hears it.
    fn offline_mode_moved(&self) {
        let on = self.network_status().offline_mode.is_on();
        self.session.offline().set_offline_mode(on);
        for table in [Table::Tracks, Table::Albums, Table::Artists, Table::Recents] {
            self.session.invalidate(table);
        }
        self.publish_network();
    }

    fn internet_came_back(&self) {
        {
            let mut status = self.network.lock().unwrap_or_else(PoisonError::into_inner);
            if !status.internet_down {
                return;
            }
            status.internet_down = false;
        }
        tracing::info!("the internet is back");
        self.publish_network();
    }

    /// While the internet is down, look again on a backoff until it is back.
    fn spawn_rechecks(&self) {
        if self.rechecking.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(me) = self.me.upgrade() else {
            self.rechecking.store(false, Ordering::SeqCst);
            return;
        };
        self.runtime.spawn(async move {
            let mut attempt = 0;
            while me.network_status().internet_down {
                tokio::time::sleep(network::recheck_delay(attempt)).await;
                attempt += 1;
                if me.check_network(true).await != Verdict::Down {
                    break;
                }
            }
            me.rechecking.store(false, Ordering::SeqCst);
            if me.network_status().internet_down {
                me.spawn_rechecks();
            }
        });
    }

    /// Ask about the internet when playback fails for want of a connection, as a failed request does.
    pub fn watch_network(self: &Arc<Self>) {
        use tokio::sync::broadcast::error::RecvError;
        let service = self.clone();
        let mut events = self.session.subscribe();
        self.runtime.spawn(async move {
            let mut failing = false;
            loop {
                match events.recv().await {
                    Ok(ApiEvent::PlayerState(state)) => {
                        let unreachable = state
                            .error
                            .as_ref()
                            .is_some_and(|error| error.code == ErrorCode::SourceUnreachable);
                        if unreachable
                            && !failing
                            && let Some(id) = service.snapshot().await.active.server_id()
                        {
                            service.after_unreachable(id);
                        }
                        failing = unreachable;
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }

    async fn recover_active(&self, seen: u64) -> bool {
        let Some(id) = self.snapshot().await.active.server_id().map(str::to_string) else {
            return false;
        };
        // Once recovery gives up, the re-probe retries it, so each request against
        // a dead session does not read a browser profile.
        if self.status(&id) == Some(SourceState::AuthExpired) {
            return false;
        }
        self.recover(&id, seen).await
    }

    /// At most one attempt at a time across the daemon.
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
                // Left unprobed, so the first retry after offline mode ends probes it.
                let held = service.session.offline().offline_mode()
                    && !network::is_local(
                        &next.0,
                        next.1.as_ref().map(|server| server.url.as_str()),
                    );
                if (moved || unsettled) && !held {
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
            let mut since_client_version = 0u32;
            loop {
                ticker.tick().await;
                if service.session.offline().offline_mode() {
                    continue;
                }
                if since_client_version == 0 {
                    service.refresh_ytmusic_client_version().await;
                }
                since_client_version = (since_client_version + 1) % 12;
                service.rotate_ytmusic().await;
                since_spotify += 1;
                if since_spotify >= 6 {
                    since_spotify = 0;
                    service.refresh_spotify().await;
                }
            }
        });
    }

    async fn refresh_ytmusic_client_version(&self) {
        if self.snapshot().await.active_service() != Some(config::MusicService::YtMusic) {
            return;
        }
        if let Err(error) = server::ytmusic::clients::refresh_live_version().await {
            tracing::warn!(%error, "reading the live YouTube Music client version failed; keeping the last one");
        }
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

/// Accept cookies YouTube does not call signed out, else try one keepalive
/// rotation before giving up on them.
async fn try_resume_ytmusic(seed: Option<String>) -> Option<String> {
    use server::ytmusic::SessionCheck;
    let cookies = seed?;
    match server::provider::check_ytmusic_session(&cookies).await {
        SessionCheck::Valid => return Some(cookies),
        SessionCheck::Unknown(error) => {
            tracing::warn!(%error, "YouTube Music gave no verdict on the stored session; keeping it");
            return Some(cookies);
        }
        SessionCheck::SignedOut => {}
    }
    let rotated = server::ytmusic::verify_session_keepalive::tick(&cookies)
        .await
        .ok()??;
    server::provider::check_ytmusic_session(&rotated)
        .await
        .is_usable()
        .then_some(rotated)
}

/// Resume from stored cookies, then from the isolated browser profile, and
/// only then force a full sign-in -- which must validate before it is
/// trusted. Skipping the resume steps would demand a password and 2FA on
/// every transient network error. The flag is true when a new sign-in ran.
#[cfg(not(target_os = "android"))]
async fn ensure_ytmusic_signed_in(
    stored: Option<String>,
    browser: config::Browser,
    server_id: &str,
    imported: Option<&str>,
) -> Result<(String, bool), String> {
    if let Some(cookies) = try_resume_ytmusic(stored).await {
        return Ok((cookies, false));
    }
    if let Some(cookies) = resume_from_profiles(browser, server_id, imported).await {
        return Ok((cookies, false));
    }
    let cookies = server::ytmusic::isolated_profile::launch_signin_and_extract(
        browser,
        server_id,
        SIGNIN_TIMEOUT,
    )
    .await?;
    match server::provider::check_ytmusic_session(&cookies).await {
        server::ytmusic::SessionCheck::SignedOut => {
            return Err(
                "sign-in finished but YouTube Music still answers as signed out".to_string(),
            );
        }
        server::ytmusic::SessionCheck::Unknown(error) => {
            tracing::warn!(%error, "YouTube Music gave no verdict on the new session; keeping it");
        }
        server::ytmusic::SessionCheck::Valid => {}
    }
    Ok((cookies, true))
}

/// The isolated sign-in profile first, else the imported browser profile.
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

    struct NoLibrary;

    #[async_trait::async_trait]
    impl crate::QueueMaterializer for NoLibrary {
        async fn materialize(&self, _: &api::QueueContext) -> Result<Vec<reader::Track>, ApiError> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct FakeUpstream {
        recovered: Option<String>,
        recoveries: std::sync::atomic::AtomicUsize,
        validations: Mutex<std::collections::VecDeque<AuthOutcome>>,
        /// What each connectivity check finds, in turn; up once the script runs out.
        internet: Mutex<std::collections::VecDeque<Verdict>>,
        internet_asks: std::sync::atomic::AtomicUsize,
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

        async fn internet(&self, _: &[String]) -> Verdict {
            self.internet_asks.fetch_add(1, Ordering::SeqCst);
            let mut script = self.internet.lock().expect("script");
            script.pop_front().unwrap_or(Verdict::Up)
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

    async fn signed_in(upstream: Arc<FakeUpstream>) -> Fixture {
        signed_in_with(upstream, config::Settings::default()).await
    }

    async fn signed_in_with(upstream: Arc<FakeUpstream>, settings: config::Settings) -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = db::init(&dir.path().join("sources.db")).await.expect("db");
        let mut seeded = youtube_music(false);
        if let Some(server) = seeded.server.as_ref() {
            seeded
                .servers
                .push(config::SavedServer::from_music_server(server));
        }
        persist(&db, &Sources::default(), &seeded)
            .await
            .expect("seed the database");
        let config = Arc::new(ConfigService::new(
            db.clone(),
            dir.path().join("settings.toml"),
            settings,
        ));
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
        let sources =
            SourceService::with_upstream(db.clone(), session, config, Arc::new(seeded), upstream);
        Fixture {
            _dir: dir,
            db,
            sources,
        }
    }

    fn expired() -> ApiError {
        ApiError::new(ErrorCode::SourceAuthExpired, "signed out")
    }

    /// Fails the first time, like a request against a session YouTube stopped taking.
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
                    .snapshot()
                    .await
                    .server
                    .clone()
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
                .snapshot()
                .await
                .find_saved_server("yt")
                .cloned()
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

    /// A caller told of a change reads it back straight away, so the session has to hold it by then.
    fn reached_the_session(session: &SessionHandle, active: &config::Source) {
        assert_eq!(session.sources_watch().borrow().active, *active);
    }

    #[tokio::test]
    async fn a_source_form_save_that_is_refused_says_why_by_reason() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("sources.db")).await.expect("db");
        let config = Arc::new(ConfigService::new(
            database.clone(),
            dir.path().join("settings.toml"),
            config::Settings::default(),
        ));
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("headless player starts");
        let session = SessionHandle::spawn_with_player(
            Arc::new(NoTracks),
            player,
            crate::PlaybackServices::default(),
        );
        config.attach_session(session.clone());
        let service = SourceService::new(database, session.clone(), config, session.sources());

        let draft = |service: &str, name: &str, values: Vec<api::FieldValue>| api::SourceDraft {
            id: None,
            name: name.into(),
            service: service.into(),
            values,
            secrets: Vec::new(),
        };
        let folders = |paths: &[&str]| {
            vec![api::FieldValue::new(
                crate::services::DIRECTORIES,
                api::schema::encode_directories(
                    &paths
                        .iter()
                        .map(|path| path.to_string())
                        .collect::<Vec<_>>(),
                ),
            )]
        };
        for (draft, reason) in [
            (
                draft(crate::services::FOLDERS, "Mine", folders(&[""])),
                ErrorReason::EmptySourceFolder,
            ),
            (
                draft(crate::services::FOLDERS, "Mine", folders(&[])),
                ErrorReason::SourceFolderRequired,
            ),
            (
                draft(crate::services::FOLDERS, " ", folders(&["/music"])),
                ErrorReason::SourceNameRequired,
            ),
            (
                draft(
                    "jellyfin",
                    "Jelly",
                    vec![api::FieldValue::new(crate::services::URL, "jelly.local")],
                ),
                ErrorReason::InvalidServerUrl,
            ),
        ] {
            let error = service.upsert_source(draft).await.expect_err("refused");
            assert_eq!(error.reason, Some(reason), "{}", error.message);
            assert!(
                !error.message.contains("_"),
                "a key is no message: {}",
                error.message
            );
        }
    }

    #[tokio::test]
    async fn a_switch_and_a_deleted_active_folder_source_are_stored_and_reach_the_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let database = db::init(&dir.path().join("sources.db")).await.expect("db");
        let mut seeded = config::Settings::default();
        seeded.library.local_sources.push(config::SavedLocalSource {
            id: "local:b".into(),
            name: "B".into(),
            directories: Vec::new(),
        });
        let settings_path = dir.path().join("settings.toml");
        config::store::save_settings(
            &settings_path,
            &seeded,
            &[config::SettingKey::Library(
                config::LibraryKey::LocalSources,
            )],
        )
        .expect("seed the settings file");
        let config = Arc::new(ConfigService::new(
            database.clone(),
            settings_path,
            seeded.clone(),
        ));
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .expect("headless player starts");
        let session = SessionHandle::spawn_with_player(
            Arc::new(NoTracks),
            player,
            crate::PlaybackServices {
                settings: seeded,
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

        let local_b = b.clone();
        service
            .mutate(move |sources| sources.set_active_local(local_b))
            .await
            .expect("switch");
        reached_the_session(&session, &b);
        service.mutate(Sources::clear_active).await.expect("back");

        let info = service.switch_source("local:b").await.expect("switch");
        assert!(info.active);
        assert_eq!(
            info.settings
                .iter()
                .map(|field| field.config_key.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("local_sources")],
            "a pin on the folder sources locks this field"
        );
        assert_eq!(database.load_sources().await.expect("load").active, b);
        reached_the_session(&session, &b);

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
                .library
                .local_sources
                .iter()
                .any(|saved| saved.id == "local:b")
        );
        reached_the_session(&session, &fallback);
    }

    mod network_down {
        use super::*;

        /// YouTube Music active, beside a default folder library that holds a track.
        async fn with_local_music(
            upstream: Arc<FakeUpstream>,
            settings: config::Settings,
        ) -> Fixture {
            let fixture = signed_in_with(upstream, settings).await;
            let song = reader::Track {
                id: reader::TrackId::Local("/music/song.flac".into()),
                cover: None,
                album_id: String::new(),
                title: "Song".into(),
                artist: "Ada".into(),
                album: String::new(),
                duration: 60,
                khz: 44,
                bitrate: 320,
                track_number: None,
                disc_number: None,
                musicbrainz_release_id: None,
                musicbrainz_recording_id: None,
                musicbrainz_track_id: None,
                playlist_item_id: None,
                credits: Vec::new(),
                artists: Vec::new(),
                replay_gain: config::ReplayGainInfo::default(),
                explicit: false,
                plays: None,
                counterpart: None,
            };
            fixture
                .db
                .upsert_tracks(&config::Source::default(), &[song])
                .await
                .expect("a local track");
            fixture
        }

        fn scripted(verdicts: &[Verdict]) -> Arc<FakeUpstream> {
            let upstream = FakeUpstream::default();
            upstream
                .internet
                .lock()
                .expect("script")
                .extend(verdicts.iter().copied());
            Arc::new(upstream)
        }

        fn asks(upstream: &FakeUpstream) -> usize {
            upstream.internet_asks.load(Ordering::SeqCst)
        }

        async fn active(fixture: &Fixture) -> config::Source {
            fixture.sources.snapshot().await.active.clone()
        }

        fn down(left: Option<&str>) -> NetworkStatus {
            NetworkStatus {
                internet_down: true,
                left_source: left.map(str::to_string),
                offline_mode: OfflineMode::Off,
            }
        }

        #[tokio::test]
        async fn the_internet_going_down_moves_to_the_default_folder_library() {
            let upstream = scripted(&[Verdict::Down]);
            let fixture = with_local_music(upstream.clone(), config::Settings::default()).await;
            let mut events = fixture.sources.session.subscribe();

            assert_eq!(fixture.sources.check_network(false).await, Verdict::Down);

            assert_eq!(active(&fixture).await, config::Source::default());
            assert_eq!(fixture.sources.network_status(), down(Some("yt")));
            let published = loop {
                match events.recv().await.expect("an event") {
                    ApiEvent::NetworkStatus(status) => break status,
                    _ => continue,
                }
            };
            assert_eq!(published, down(Some("yt")));
        }

        #[tokio::test]
        async fn with_no_source_that_works_offline_the_active_one_stays() {
            // The default folder library is there, but holds nothing to play.
            let fixture =
                signed_in_with(scripted(&[Verdict::Down]), config::Settings::default()).await;

            fixture.sources.check_network(false).await;

            assert_eq!(active(&fixture).await, config::Source::Server("yt".into()));
            let automatic = NetworkStatus {
                offline_mode: OfflineMode::Automatic,
                ..down(None)
            };
            assert_eq!(fixture.sources.network_status(), automatic);
            assert!(fixture.sources.session.offline().offline_mode());
        }

        #[tokio::test]
        async fn a_switch_to_a_source_that_needs_the_internet_is_refused_while_it_is_down() {
            let upstream = scripted(&[Verdict::Down]);
            let fixture = with_local_music(upstream.clone(), config::Settings::default()).await;
            fixture.sources.check_network(false).await;

            let refused = fixture
                .sources
                .switch_source("yt")
                .await
                .expect_err("the internet is down");

            assert_eq!(refused.reason, Some(ErrorReason::NetworkDown));
            assert_eq!(refused.code, ErrorCode::SourceUnreachable);
            assert_eq!(asks(&upstream), 1, "the recent check still stands");
            assert_eq!(active(&fixture).await, config::Source::default());
            fixture
                .sources
                .switch_source(config::DEFAULT_LOCAL_ID)
                .await
                .expect("a folder library works without it");
        }

        #[tokio::test]
        async fn the_internet_coming_back_is_published_and_nothing_switches_back() {
            let upstream = scripted(&[Verdict::Down, Verdict::Up]);
            let fixture = with_local_music(upstream.clone(), config::Settings::default()).await;
            let mut events = fixture.sources.session.subscribe();
            fixture.sources.check_network(false).await;

            assert_eq!(fixture.sources.check_network(true).await, Verdict::Up);

            let back = NetworkStatus {
                internet_down: false,
                left_source: Some("yt".into()),
                offline_mode: OfflineMode::Off,
            };
            assert_eq!(fixture.sources.network_status(), back);
            assert_eq!(active(&fixture).await, config::Source::default());
            let published = loop {
                match events.recv().await.expect("an event") {
                    ApiEvent::NetworkStatus(status) if !status.internet_down => break status,
                    _ => continue,
                }
            };
            assert_eq!(published, back, "the source to offer back stays");

            fixture
                .sources
                .switch_source("yt")
                .await
                .expect("online again");
            assert_eq!(fixture.sources.network_status(), NetworkStatus::default());
        }

        #[tokio::test]
        async fn failures_share_one_spaced_check() {
            let upstream = scripted(&[Verdict::Up]);
            let fixture = signed_in(upstream.clone()).await;

            let (a, b) = tokio::join!(
                fixture.sources.check_network(false),
                fixture.sources.check_network(false)
            );
            fixture.sources.check_network(false).await;

            assert_eq!((a, b), (Verdict::Up, Verdict::Up));
            assert_eq!(asks(&upstream), 1);
            assert_eq!(fixture.sources.network_status(), NetworkStatus::default());
        }

        #[tokio::test]
        async fn an_empty_check_list_asks_nothing_and_leaves_a_failure_plain_offline() {
            let mut settings = config::Settings::default();
            settings.system.connectivity_check_urls.clear();
            let upstream = scripted(&[Verdict::Down]);
            let fixture = with_local_music(upstream.clone(), settings).await;

            assert_eq!(
                fixture.sources.check_network(false).await,
                Verdict::Unchecked
            );

            assert_eq!(asks(&upstream), 0);
            assert_eq!(fixture.sources.network_status(), NetworkStatus::default());
            assert_eq!(active(&fixture).await, config::Source::Server("yt".into()));
        }

        #[tokio::test]
        async fn a_source_that_stops_answering_starts_the_check() {
            let upstream = scripted(&[Verdict::Down]);
            upstream
                .validations
                .lock()
                .expect("script")
                .push_back(AuthOutcome::Unreachable);
            let fixture = with_local_music(upstream.clone(), config::Settings::default()).await;
            let mut events = fixture.sources.session.subscribe();

            let state = fixture.sources.validate_source("yt").await.expect("probe");

            assert_eq!(state, SourceState::Offline);
            let published = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Ok(ApiEvent::NetworkStatus(status)) = events.recv().await {
                        break status;
                    }
                }
            })
            .await
            .expect("the check ran");
            assert_eq!(published, down(Some("yt")));
            assert_eq!(asks(&upstream), 1);
        }

        fn mode(fixture: &Fixture) -> OfflineMode {
            fixture.sources.network_status().offline_mode
        }

        #[tokio::test]
        async fn an_automatic_offline_mode_outlasts_the_outage_until_someone_leaves_it() {
            let upstream = scripted(&[Verdict::Down, Verdict::Up]);
            let fixture = signed_in_with(upstream, config::Settings::default()).await;
            let mut events = fixture.sources.session.subscribe();
            fixture.sources.check_network(false).await;
            assert_eq!(mode(&fixture), OfflineMode::Automatic);

            assert_eq!(fixture.sources.check_network(true).await, Verdict::Up);

            let back = fixture.sources.network_status();
            assert!(!back.internet_down);
            assert_eq!(
                back.offline_mode,
                OfflineMode::Automatic,
                "a frontend offers to leave"
            );
            assert!(fixture.sources.session.offline().offline_mode());
            let left = fixture.sources.set_offline(false);
            assert_eq!(left, NetworkStatus::default());
            assert!(!fixture.sources.session.offline().offline_mode());
            let mut invalidated = Vec::new();
            let published = loop {
                match events.recv().await.expect("an event") {
                    ApiEvent::LibraryInvalidated { table } => invalidated.push(table),
                    ApiEvent::NetworkStatus(status) if status.offline_mode == OfflineMode::Off => {
                        break status;
                    }
                    _ => continue,
                }
            };
            assert_eq!(published, NetworkStatus::default());
            assert!(invalidated.contains(&Table::Tracks), "{invalidated:?}");
        }

        #[tokio::test]
        async fn an_offline_mode_someone_asked_for_stays_through_the_internet_coming_back() {
            let upstream = scripted(&[Verdict::Down, Verdict::Up]);
            let fixture = with_local_music(upstream, config::Settings::default()).await;

            let status = fixture.sources.set_offline(true);
            assert_eq!(status.offline_mode, OfflineMode::Requested);
            assert!(fixture.sources.session.offline().offline_mode());
            fixture.sources.check_network(false).await;
            fixture.sources.check_network(true).await;

            assert_eq!(mode(&fixture), OfflineMode::Requested);
            assert_eq!(
                fixture.sources.set_offline(false).offline_mode,
                OfflineMode::Off
            );
        }

        #[tokio::test]
        async fn someone_using_the_app_while_it_is_down_brings_the_recheck_forward() {
            let upstream = scripted(&[Verdict::Down, Verdict::Up]);
            let fixture = with_local_music(upstream.clone(), config::Settings::default()).await;
            fixture.sources.check_network(false).await;

            fixture.sources.check_network_soon();
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(asks(&upstream), 1, "a check this recent still stands");

            tokio::time::sleep(network::NUDGE_SPACING).await;
            let mut events = fixture.sources.session.subscribe();
            fixture.sources.check_network_soon();
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Ok(ApiEvent::NetworkStatus(status)) = events.recv().await
                        && !status.internet_down
                    {
                        break;
                    }
                }
            })
            .await
            .expect("the internet is back without waiting for the scheduled recheck");
            assert_eq!(asks(&upstream), 2);

            fixture.sources.check_network_soon();
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(asks(&upstream), 2, "with the internet up nothing is asked");
        }

        #[tokio::test]
        async fn each_source_says_whether_it_works_without_the_internet() {
            let fixture = signed_in(scripted(&[])).await;
            let sources = fixture.sources.sources().await.expect("sources");
            let works_offline = |id: &str| {
                sources
                    .iter()
                    .find(|source| source.id == id)
                    .map(|source| source.works_offline)
            };
            assert_eq!(works_offline(config::DEFAULT_LOCAL_ID), Some(true));
            assert_eq!(works_offline("yt"), Some(false));
        }

        #[tokio::test]
        async fn a_move_to_the_folder_library_is_no_offline_mode() {
            let fixture =
                with_local_music(scripted(&[Verdict::Down]), config::Settings::default()).await;

            fixture.sources.check_network(false).await;

            assert_eq!(mode(&fixture), OfflineMode::Off);
            assert!(!fixture.sources.session.offline().offline_mode());
        }
    }
}
