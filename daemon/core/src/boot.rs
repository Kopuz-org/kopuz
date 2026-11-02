//! Building a core: the database, the audio engine, and every service over
//! them, ending at a [`LocalApi`].
//!
//! No socket and no signal handling. `assemble` gives whoever calls it a
//! running daemon; `kopuz-kopuzd` puts that behind gRPC and adds a process
//! lifecycle, the app hosts it beside its UI. That split is why this is a
//! library function and not the body of a `main`.
//!
//! Exclusive database access is enforced by [`crate::DatabaseLease`]: a second
//! process pointed at the same `KOPUZ_DB_PATH` fails to start rather than
//! becoming a second writer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{
    ConfigService, DbQueueStore, FavoritesService, JobRunner, LibraryService, LocalApi,
    PlaybackServices, QueueStore, SessionHandle, SourceRecorder,
};

/// Which library to open. Everything else about a core is the same wherever
/// it runs.
#[derive(Debug, Default, Clone)]
pub struct CoreArgs {
    pub db_path: Option<String>,
    /// Start in offline mode, as `--offline` asks: listings and playback keep to downloads and cached songs.
    pub offline: bool,
}

/// Everything a running daemon owns. The services are public because a host
/// that runs the core in-process still reaches some of them directly.
pub struct Core {
    pub api: Arc<LocalApi>,
    pub session: SessionHandle,
    pub artwork: Arc<crate::ArtworkService>,
    pub db: db::Db,
    pub settings: config::Settings,
    /// The authority on the running settings, as opposed to the `settings` snapshot above.
    pub config_service: Arc<ConfigService>,
    pub library: Arc<LibraryService>,
    pub jobs: Arc<JobRunner>,
    pub favorites: Arc<FavoritesService>,
    pub scrobbler: Arc<crate::Scrobbler>,
    pub station_registry: Arc<radio::registry::StationRegistry>,
    _lease: crate::DatabaseLease,
}

/// Open the library, recreating it only when SQLite says it is corrupt -- any
/// other failure keeps the file, because discarding a healthy library over a
/// transient error is unrecoverable.
async fn open_database(path: &Path) -> Result<db::Db, Box<dyn std::error::Error>> {
    let error = match db::init(path).await {
        Ok(handle) => return Ok(handle),
        Err(error) => error,
    };
    let message = error.to_string().to_lowercase();
    let corrupt = message.contains("malformed")
        || message.contains("not a database")
        || message.contains("corrupt");
    if !corrupt {
        return Err(error.into());
    }
    tracing::error!(%error, "library database is corrupt; moving it aside and recreating");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    for extension in ["", "-wal", "-shm"] {
        let mut from = path.as_os_str().to_os_string();
        from.push(extension);
        let mut to = path.as_os_str().to_os_string();
        to.push(format!(".corrupt-{stamp}{extension}"));
        let _ = std::fs::rename(from, to);
    }
    Ok(db::init(path).await?)
}

/// What the core needs done on the thread that builds its runtime, before it
/// is built. Today that is the JS platform: every thread that will run V8
/// has to descend from the one that initialised it, and the runtime's workers
/// are the ancestors of everything the core spawns.
pub fn prepare_thread() {
    #[cfg(not(target_os = "android"))]
    server::ytmusic::ensure_v8_platform();
}

/// Open the library, start the audio engine, and wire every service onto it.
pub async fn assemble(args: &CoreArgs) -> Result<Core, Box<dyn std::error::Error>> {
    let using_default_database =
        args.db_path.is_none() && std::env::var_os("KOPUZ_DB_PATH").is_none();
    if using_default_database {
        for line in db::legacy::migrate_identity() {
            tracing::info!("{line}");
        }
        crate::nightly_seed::seed_from_stable().await;
        db::legacy::migrate_locations();
    }
    let db_path = args
        .db_path
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(db::default_db_path);
    let Some(lease) = crate::DatabaseLease::claim_with_retry(&db_path).await? else {
        return Err(format!(
            "another kopuz process already owns {}; close it first",
            db_path.display()
        )
        .into());
    };
    tracing::info!(path = %db_path.display(), "opening library database");
    let database = open_database(&db_path).await?;
    if using_default_database {
        db::legacy::migrate_json_store(&database, &db::config_dir()).await;
    }
    let settings_path =
        config::store::settings_path_for(db_path.parent().unwrap_or_else(|| Path::new(".")));
    let settings = crate::config_service::load_settings(&settings_path)?;
    let sources = Arc::new(crate::sources::load(&database).await?);
    let credentials = crate::CredentialStore::load(&database).await?;
    let cache =
        crate::OfflineCache::load(&database, crate::offline_cache::default_dir(), &settings)
            .await?;
    let cache_changed = cache.changed();
    let offline = crate::OfflineIndex::load(&database)
        .await?
        .with_cache(cache);
    // Before anything below starts reaching the network.
    offline.set_offline_mode(args.offline);
    let downloads_pruned = offline.pruned();
    let (volume, muted) = database.playback_state().await?;
    let presence_pause = crate::integrations::PresencePause::load(&database).await?;

    let config_service = Arc::new(ConfigService::new(
        database.clone(),
        settings_path,
        settings.clone(),
    ));
    let station_registry = Arc::new(radio::registry::StationRegistry::default());
    let cover_cache = config::flavor::project_dirs()
        .map(|dirs| dirs.cache_dir().join("covers"))
        .unwrap_or_else(|| std::env::temp_dir().join("kopuz-covers"));
    let _ = std::fs::create_dir_all(&cover_cache);
    let library = Arc::new(LibraryService::new(
        database.clone(),
        sources.active.clone(),
        station_registry.clone(),
        cover_cache,
        credentials.clone(),
    ));
    server::ytmusic::player::init_tier_store(database.clone());
    db::cache::init(database.clone());
    let active_source: server::source::ActiveSource =
        Arc::from(server::source::active(database.clone(), &sources));
    let queue_store: Arc<dyn QueueStore> = Arc::new(DbQueueStore::new(database.clone()));
    let scrobbler = crate::Scrobbler::new(database.clone(), credentials.clone());
    let services = PlaybackServices {
        settings: settings.clone(),
        volume,
        muted,
        sources: sources.clone(),
        active_source: Some(active_source.clone()),
        station_registry: station_registry.clone(),
        queue_store: Some(queue_store.clone()),
        recorder: Some(Arc::new(SourceRecorder)),
        scrobbler: Some(scrobbler.clone()),
        offline,
    };

    let session = SessionHandle::try_spawn(library.clone(), services)
        .map_err(|error| format!("audio engine init failed: {error:?}"))?;
    library.attach_session(session.clone());
    {
        let session = session.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = downloads_pruned.notified() => {}
                    _ = cache_changed.notified() => {}
                }
                session.invalidate(api::Table::Tracks);
                session.invalidate(api::Table::Albums);
            }
        });
    }
    library.spawn_artist_photo_upkeep();
    config_service.attach_session(session.clone());
    scrobbler.attach_session(session.clone());
    {
        let scrobbler = scrobbler.clone();
        tokio::spawn(async move {
            scrobbler.drain_queue().await;
        });
    }
    let jobs = Arc::new(JobRunner::new(session.clone()));
    let downloads = crate::DownloadsService::new(
        database.clone(),
        session.clone(),
        config::flavor::project_dirs()
            .map(|dirs| dirs.cache_dir().join("offline_tracks"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-offline")),
    );
    let favorites = FavoritesService::new(database.clone(), session.clone());
    favorites.spawn_reconciler();
    let playlists = crate::PlaylistService::new(database.clone(), session.clone());
    playlists.attach_favorites(favorites.clone());
    spawn_playback_state_persistence(&session, database.clone());
    crate::os_media::spawn(&session);
    crate::integrations::spawn_jellyfin_reporter(
        &session,
        active_source.clone(),
        session.sources_watch(),
    );
    crate::integrations::spawn_youtube_reporter(
        &session,
        database.clone(),
        session.sources_watch(),
        session.settings_watch(),
    );
    crate::integrations::spawn_discord_presence(
        &session,
        session.settings_watch(),
        session.sources_watch(),
        presence_pause.watch(),
    );
    let radio_service = crate::RadioService::new(
        database.clone(),
        config_service.clone(),
        library.clone(),
        session.clone(),
    );
    let mut radio_loaded = false;
    let restored_source = session.sources().active.clone();
    match queue_store.load(&restored_source).await {
        Ok(snapshot) if !snapshot.queue.is_empty() => {
            // A row saved before rows kept their station can only resolve through the registry, so it must be loaded first.
            if crate::session::has_unrecorded_station(&snapshot) {
                if let Err(error) = radio_service.reload().await {
                    tracing::warn!(%error, "radio registry could not be loaded");
                }
                radio_loaded = true;
            }
            let restored = snapshot.queue.len();

            library.register_transient(&snapshot.queue);
            match session.restore_queue(snapshot).await {
                Ok(_) => tracing::info!(tracks = restored, "queue restored from the last session"),
                Err(error) => tracing::warn!(%error, "queue restore failed"),
            }
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%error, "the last session's queue could not be read");
            session.hold_unread_queue();
        }
    }
    // Asking the source for its queue costs a round trip, so startup does not wait on it.
    {
        let session = session.clone();
        let library = library.clone();
        let source = active_source.clone();
        let store = queue_store.clone();
        tokio::spawn(async move {
            crate::persistence::adopt_remote_queue(&session, &library, &source, store.as_ref())
                .await;
        });
    }
    let artwork = crate::ArtworkService::new(
        database.clone(),
        session.clone(),
        config::flavor::project_dirs()
            .map(|dirs| dirs.cache_dir().join("artwork"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-artwork")),
    );
    downloads.attach_artwork(artwork.clone());
    let mutations = crate::MutationService::new(
        database.clone(),
        session.clone(),
        config::flavor::project_dirs()
            .map(|dirs| dirs.data_dir().join("artwork-uploads"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-artwork-uploads")),
    );
    let sources = crate::SourceService::new(
        database.clone(),
        session.clone(),
        config_service.clone(),
        sources,
    );
    sources.spawn_credential_upkeep();
    sources.watch_removable();
    sources.watch_active(session.sources_watch());
    sources.watch_network();
    if args.offline {
        sources.set_offline(true);
    }
    crate::auto_sync::spawn(
        database.clone(),
        jobs.clone(),
        library.clone(),
        playlists.clone(),
        favorites.clone(),
        session.settings_watch(),
        session.sources_watch(),
    );
    // A failed settings read aborts boot above, so reaching here means the settings are real.
    crate::folder_scan::spawn(
        jobs.clone(),
        library.clone(),
        session.settings_watch(),
        session.subscribe(),
        true,
    );
    config_service.spawn_file_watch(session.settings_watch());
    let config_service_for_api = config_service.clone();
    let catalog = crate::CatalogService::new(database.clone(), session.clone(), library.clone());
    library.attach_catalog(catalog.clone());
    artwork.attach_library(library.clone());
    artwork.attach_catalog(catalog.clone());
    artwork.attach_radio(radio_service.clone());
    // The registry starts empty and is built from config here, rather than
    // by whichever frontend happened to be open.
    radio_service.watch_config(session.settings_watch());
    if !radio_loaded {
        let radio_boot = radio_service.clone();
        tokio::spawn(async move {
            if let Err(error) = radio_boot.reload().await {
                tracing::warn!(%error, "radio registry could not be loaded");
            }
        });
    }
    let downloader =
        crate::UrlDownloadService::new(database.clone(), config_service.clone(), sources.clone());
    downloader.attach_rescan(library.clone(), jobs.clone());
    // Spotify plays itself, so the session is told where to send a track it
    // cannot decode. Nothing starts until one is actually queued.
    let spotify = crate::SpotifySink::new(session.clone(), sources.clone());
    session.set_external_sink(spotify.clone());
    spotify.spawn_discovery();
    let api = Arc::new(
        LocalApi::new(session.clone())
            .with_library(library.clone())
            .with_config(config_service.clone())
            .with_prefs(crate::PrefsService::new(database.clone(), session.clone()))
            .with_jobs(jobs.clone())
            .with_favorites(favorites.clone())
            .with_downloads(downloads)
            .with_artwork(artwork.clone())
            .with_playlists(playlists)
            .with_catalog(catalog)
            .with_radio(radio_service)
            .with_mutations(mutations)
            .with_sources(sources.clone())
            .with_downloader(downloader)
            .with_spotify(spotify)
            .with_video(crate::VideoService::new(database.clone(), session.clone()))
            .with_integrations(crate::IntegrationService::new(
                config_service_for_api,
                credentials,
                presence_pause,
            )),
    );

    Ok(Core {
        api,
        session,
        artwork,
        db: database,
        settings,
        config_service,
        library,
        jobs,
        favorites,
        scrobbler,
        station_registry,
        _lease: lease,
    })
}

/// How long a volume or mute change waits for the next one before it is written, so a slider drag is one write.
const PLAYBACK_STATE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(750);

fn playback_state(session: &SessionHandle) -> (f32, bool) {
    let state = session.state();
    (state.volume, state.muted)
}

/// Persist the volume and mute the engine moved to; a frontend only sends commands, so the daemon remembers them.
fn spawn_playback_state_persistence(session: &SessionHandle, database: db::Db) {
    let mut events = session.subscribe();
    let mut last = playback_state(session);
    let session = session.clone();
    tokio::spawn(async move {
        loop {
            let next = match events.recv().await {
                Ok(api::ApiEvent::PlayerState(state)) => (state.volume, state.muted),
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            if (next.0 - last.0).abs() < f32::EPSILON && next.1 == last.1 {
                continue;
            }
            tokio::time::sleep(PLAYBACK_STATE_DEBOUNCE).await;
            // What arrived during the wait is older than the state read below.
            while !matches!(
                events.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty
                    | tokio::sync::broadcast::error::TryRecvError::Closed)
            ) {}
            last = playback_state(&session);
            if let Err(error) = database.set_playback_state(last.0, last.1).await {
                tracing::warn!(%error, "persisting the volume and mute failed");
            }
        }
    });
}

/// Persist the volume and mute the engine is at right now, ahead of the debounce a close can land inside.
pub async fn flush_playback_state(session: &SessionHandle, database: &db::Db) {
    let (volume, muted) = playback_state(session);
    // An unchanged state writes nothing, so a never-configured library stays without a state row.
    if let Ok((stored, stored_muted)) = database.playback_state().await
        && (stored - volume).abs() < f32::EPSILON
        && stored_muted == muted
    {
        return;
    }
    if let Err(error) = database.set_playback_state(volume, muted).await {
        tracing::warn!(%error, "flushing the volume and mute on shutdown failed");
    }
}

/// Flush what the core owns. Callers that bound a socket unlink it themselves.
pub async fn shutdown(core: Core) {
    flush_playback_state(&core.session, &core.db).await;
    core.session.persist_now().await;
}
