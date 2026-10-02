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
}

/// Everything a running daemon owns. The services are public because a host
/// that runs the core in-process still reaches some of them directly.
pub struct Core {
    pub api: Arc<LocalApi>,
    pub session: SessionHandle,
    pub artwork: Arc<crate::ArtworkService>,
    pub db: db::Db,
    pub config: config::AppConfig,
    /// The authority on the running config, as opposed to the `config`
    /// snapshot above. A host needs it to flush the volume on the way out.
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
/// is built. The JS platform: every thread that will run V8 has to descend
/// from the one that initialised it, and the runtime's workers are the
/// ancestors of everything the core spawns. And the TLS crypto backend:
/// reqwest brings rustls with aws-lc-rs and librespot brings it with ring,
/// and with both linked rustls refuses to guess, so it is told here before
/// the first connection.
pub fn prepare_thread() {
    #[cfg(not(target_os = "android"))]
    server::ytmusic::ensure_v8_platform();
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        tracing::debug!("a rustls crypto provider was already installed");
    }
}

/// Open the library, start the audio engine, and wire every service onto it.
pub async fn assemble(args: &CoreArgs) -> Result<Core, Box<dyn std::error::Error>> {
    let using_default_database =
        args.db_path.is_none() && std::env::var_os("KOPUZ_DB_PATH").is_none();
    if using_default_database {
        for line in db::legacy::migrate_identity() {
            tracing::info!("{line}");
        }
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
    let config = database.load_config().await?.unwrap_or_default();

    let settings_path =
        config::store::settings_path_for(db_path.parent().unwrap_or_else(|| Path::new(".")));
    let config_service = Arc::new(ConfigService::new(
        database.clone(),
        settings_path,
        config.clone(),
    ));
    let station_registry = Arc::new(radio::registry::StationRegistry::default());
    let cover_cache = directories::ProjectDirs::from("moe", "kopuz", "kopuz")
        .map(|dirs| dirs.cache_dir().join("covers"))
        .unwrap_or_else(|| std::env::temp_dir().join("kopuz-covers"));
    let _ = std::fs::create_dir_all(&cover_cache);
    let library = Arc::new(LibraryService::new(
        database.clone(),
        config.active_source.clone(),
        station_registry.clone(),
        cover_cache,
    ));
    server::ytmusic::player::init_tier_store(database.clone());
    db::cache::init(database.clone());
    let active_source: server::source::ActiveSource =
        Arc::from(server::source::active(database.clone(), &config));
    let queue_store: Arc<dyn QueueStore> = Arc::new(DbQueueStore::new(database.clone()));
    let scrobbler = crate::Scrobbler::new(database.clone());
    let services = PlaybackServices {
        config: config.clone(),
        active_source: Some(active_source.clone()),
        station_registry: station_registry.clone(),
        queue_store: Some(queue_store.clone()),
        recorder: Some(Arc::new(SourceRecorder::new(active_source.clone()))),
        scrobbler: Some(scrobbler.clone()),
    };

    let session = SessionHandle::try_spawn(library.clone(), services)
        .map_err(|error| format!("audio engine init failed: {error:?}"))?;
    library.attach_session(session.clone());
    scrobbler.attach_session(session.clone());
    {
        let scrobbler = scrobbler.clone();
        let config = session.config_watch().borrow().clone();
        tokio::spawn(async move {
            scrobbler.drain_queue(&config).await;
        });
    }
    let jobs = Arc::new(JobRunner::new(session.clone()));
    let downloads = crate::DownloadsService::new(
        database.clone(),
        session.clone(),
        config_service.clone(),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("offline_tracks"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-offline")),
    );
    let favorites = FavoritesService::new(database.clone(), session.clone());
    favorites.spawn_reconciler();
    let playlists = crate::PlaylistService::new(database.clone(), session.clone());
    spawn_volume_persistence(&session, config_service.clone());
    crate::os_media::spawn(&session);
    crate::integrations::spawn_jellyfin_reporter(&session, active_source, session.config_watch());
    crate::integrations::spawn_discord_presence(&session, session.config_watch());
    if let Some(snapshot) = queue_store.load().await
        && !snapshot.queue.is_empty()
    {
        let restored = snapshot.queue.len();
        // A restored queue can hold rows the library never stored -- last
        // session's radio mix, say. Register them the way a live listing is,
        // or hearting the track that is playing answers "unknown track key".
        library.register_transient(&snapshot.queue);
        match session.restore_queue(snapshot).await {
            Ok(_) => tracing::info!(tracks = restored, "queue restored from the last session"),
            Err(error) => tracing::warn!(%error, "queue restore failed"),
        }
    }
    let artwork = crate::ArtworkService::new(
        database.clone(),
        session.clone(),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("artwork"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-artwork")),
    );
    let mutations = crate::MutationService::new(
        database.clone(),
        session.clone(),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.data_dir().join("artwork-uploads"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-artwork-uploads")),
    );
    let sources =
        crate::SourceService::new(database.clone(), session.clone(), config_service.clone());
    sources.spawn_credential_upkeep();
    let config_service_for_api = config_service.clone();
    let catalog = crate::CatalogService::new(database.clone(), session.clone(), library.clone());
    let radio_service = crate::RadioService::new(config_service.clone(), library.clone());
    library.attach_catalog(catalog.clone());
    artwork.attach_library(library.clone());
    artwork.attach_catalog(catalog.clone());
    artwork.attach_radio(radio_service.clone());
    // The registry starts empty and is built from config here, rather than
    // by whichever frontend happened to be open.
    radio_service.watch_config(session.config_watch());
    let radio_boot = radio_service.clone();
    tokio::spawn(async move {
        if let Err(error) = radio_boot.reload().await {
            tracing::warn!(%error, "radio registry could not be loaded");
        }
    });
    let downloader = crate::UrlDownloadService::new(session.clone(), config_service.clone());
    downloader.attach_rescan(library.clone(), jobs.clone());
    let api = Arc::new(
        LocalApi::new(session.clone())
            .with_library(library.clone())
            .with_config(config_service.clone())
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
            .with_integrations(crate::IntegrationService::new(
                config_service_for_api,
                session.clone(),
            )),
    );

    Ok(Core {
        api,
        session,
        artwork,
        db: database,
        config,
        config_service,
        library,
        jobs,
        favorites,
        scrobbler,
        station_registry,
        _lease: lease,
    })
}

/// Persist volume changes the engine made. Volume arrives as a player
/// command from whichever frontend is attached, so the daemon has to be the
/// one that remembers it; the debounce keeps a drag off the database.
fn spawn_volume_persistence(session: &SessionHandle, config: Arc<ConfigService>) {
    let mut events = session.subscribe();
    let mut last = session.state().volume;
    tokio::spawn(async move {
        loop {
            let volume = match events.recv().await {
                Ok(api::ApiEvent::PlayerState(state)) => state.volume,
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            if (volume - last).abs() < f32::EPSILON {
                continue;
            }
            last = volume;
            tokio::time::sleep(std::time::Duration::from_millis(750)).await;
            if let Err(error) = config.set_volume(last).await {
                tracing::warn!(%error, "volume persist failed");
            }
        }
    });
}

/// Persist the volume the engine is at right now, ahead of the debounce.
///
/// [`spawn_volume_persistence`] holds a change for 750 ms so that dragging a
/// slider is one write instead of fifty, so a close inside that window leaves
/// the newest value with nothing but the engine holding it. The service is
/// also where a whole-config write reads volume back from, which is why the
/// frontend's own copy cannot stand in for this.
pub async fn flush_volume(session: &SessionHandle, config: &ConfigService) {
    if let Err(error) = config.set_volume(session.state().volume).await {
        tracing::warn!(%error, "flushing the volume on shutdown failed");
    }
}

/// Flush what the core owns. Callers that bound a socket unlink it themselves.
pub async fn shutdown(core: Core) {
    flush_volume(&core.session, &core.config_service).await;
    core.session.persist_now().await;
}
