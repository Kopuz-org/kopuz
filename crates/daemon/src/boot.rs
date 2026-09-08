//! Daemon startup: logging, socket, services, and the shutdown path.
//!
//! Lives in the library rather than the `kopuzd` binary because the app hosts
//! the very same core in its own process: [`assemble`] builds every service
//! and the [`crate::LocalApi`] over them, [`listen`] adds the socket, and
//! [`shutdown`] flushes. `kopuzd` is those three in a row; the app calls
//! `assemble` + `listen` and hands the api to its UI.
//!
//! Owns the real audio engine, the Kopuz database, and the configured source,
//! and serves the gRPC API from `crate::grpc` (see `proto/kopuz.proto`).
//! Queue contexts resolve from the library (albums, artists, genres,
//! playlists, filters, radio) with a fallback probe for ad-hoc local file
//! paths. Reflection is on, so:
//!
//! ```sh
//! kopuzd
//! grpcurl -unix -plaintext \
//!   $XDG_RUNTIME_DIR/kopuz/kopuzd.sock kopuz.v1.Kopuz/GetPlayerState
//! ```
//!
//! The socket path (logged at startup) is the whole rendezvous: a frontend
//! opens it or it does not exist. Its 0600 mode is the access control, so
//! the channel carries no credentials.
//!
//! Exclusive database access is enforced by [`crate::DatabaseLease`]: a
//! second process pointed at the same `KOPUZ_DB_PATH` fails to start rather
//! than becoming a second writer.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::{
    ConfigService, DbQueueStore, FavoritesService, JobRunner, LibraryService, LocalApi,
    PlaybackServices, QueueStore, SessionHandle, SourceRecorder,
};

async fn terminate_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        match signal(SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "could not install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    }

    #[cfg(not(unix))]
    std::future::pending::<()>().await;
}

/// What the daemon needs to start, however it was launched.
#[derive(Debug, Default)]
pub struct BootArgs {
    pub socket: Option<PathBuf>,
    pub db_path: Option<String>,
    /// Launched by a frontend: exit when that frontend goes away.
    pub supervised: bool,
}

pub fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };
    let console = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    let Some(dir) = log_dir() else {
        tracing_subscriber::registry()
            .with(console.with_filter(filter()))
            .init();
        return None;
    };
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing_subscriber::registry()
            .with(console.with_filter(filter()))
            .init();
        tracing::warn!(%error, path = %dir.display(), "no daemon log directory");
        return None;
    }
    utils::logs::rotate_session_log_named(
        &dir,
        utils::logs::DAEMON_LATEST,
        utils::logs::DAEMON_SESSION_PREFIX,
    );
    let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::never(
        &dir,
        utils::logs::DAEMON_LATEST,
    ));
    tracing_subscriber::registry()
        .with(console.with_filter(filter()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer)
                .with_filter(filter()),
        )
        .init();
    tracing::info!(path = %dir.join(utils::logs::DAEMON_LATEST).display(), "daemon log");
    Some(guard)
}

fn log_dir() -> Option<PathBuf> {
    Some(directories::BaseDirs::new()?.cache_dir().join("kopuz/logs"))
}

pub fn default_socket_path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join("kopuz"))
        .unwrap_or_else(|| base.cache_dir().join("kopuz"));
    Some(dir.join("kopuzd.sock"))
}

/// Everything a running daemon owns. Built by [`assemble`] whether or not a
/// socket is ever bound, so the app can host it in-process.
pub struct Core {
    pub api: Arc<LocalApi>,
    pub session: SessionHandle,
    pub artwork: Arc<crate::ArtworkService>,
    pub db: db::Db,
    pub config: config::AppConfig,
    pub supervisor: Option<Arc<crate::grpc::Supervisor>>,
    started: Instant,
    _lease: crate::DatabaseLease,
}

impl Core {
    fn grpc_state(&self) -> Arc<crate::grpc::GrpcState> {
        Arc::new(crate::grpc::GrpcState {
            api: self.api.clone(),
            artwork: Some(self.artwork.clone()),
            session: self.session.clone(),
            started: self.started,
            supervisor: self.supervisor.clone(),
        })
    }
}

/// Open the library, start the audio engine, and wire every service onto it.
/// No socket: a frontend that hosts the core in its own process stops here.
pub async fn assemble(args: &BootArgs) -> Result<Core, Box<dyn std::error::Error>> {
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
    let database = db::init(&db_path).await?;
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
    utils::db_cache::init(database.clone());
    let active_source: server::source::ActiveSource =
        Arc::from(server::source::active(database.clone(), &config));
    let queue_store: Arc<dyn QueueStore> = Arc::new(DbQueueStore::new(database.clone()));
    let scrobbler = crate::Scrobbler::new(database.clone());
    let services = PlaybackServices {
        config: config.clone(),
        active_source: Some(active_source.clone()),
        station_registry,
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
    spawn_volume_persistence(&session, config_service.clone());
    crate::os_media::spawn(&session);
    crate::integrations::spawn_jellyfin_reporter(&session, active_source, session.config_watch());
    crate::integrations::spawn_discord_presence(&session, session.config_watch());
    if let Some(snapshot) = queue_store.load().await
        && !snapshot.queue.is_empty()
    {
        let restored = snapshot.queue.len();
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
    let api = Arc::new(
        LocalApi::new(session.clone())
            .with_library(library)
            .with_config(config_service)
            .with_jobs(jobs)
            .with_favorites(favorites)
            .with_downloads(downloads),
    );

    Ok(Core {
        api,
        session,
        artwork,
        db: database,
        config,
        supervisor: args
            .supervised
            .then(|| Arc::new(crate::grpc::Supervisor::default())),
        started: Instant::now(),
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

/// Bind the socket and serve the core on it until the server stops.
pub async fn listen(core: &Core, socket: &Path) -> std::io::Result<()> {
    let listener = crate::grpc::bind_socket(socket)?;
    tracing::info!(path = %socket.display(), "kopuzd listening");
    crate::grpc::serve(listener, core.grpc_state()).await
}

/// Flush what the core owns and release the socket.
pub async fn shutdown(core: Core, socket: Option<&Path>) {
    core.session.persist_now().await;
    if let Some(socket) = socket {
        let _ = std::fs::remove_file(socket);
    }
}

pub async fn run(args: BootArgs) -> Result<(), Box<dyn std::error::Error>> {
    let core = assemble(&args).await?;

    let socket = match args.socket.clone().or_else(default_socket_path) {
        Some(path) => path,
        None => {
            return Err("no usable runtime directory for the daemon socket".into());
        }
    };

    let supervisor = core.supervisor.clone();
    let orphaned = async {
        match supervisor {
            // A supervised daemon exists to serve the frontend that started
            // it, so losing that frontend is a reason to exit, not an idle
            // state to sit in.
            Some(supervisor) => supervisor.orphaned().await,
            None => std::future::pending().await,
        }
    };

    let result = tokio::select! {
        served = listen(&core, &socket) => served.map_err(Into::into),
        () = orphaned => {
            tracing::info!("frontend detached; supervised daemon exiting");
            Ok(())
        }
        signal = tokio::signal::ctrl_c() => {
            signal?;
            tracing::info!("shutting down");
            Ok(())
        }
        () = terminate_signal() => {
            tracing::info!("shutting down");
            Ok(())
        }
    };

    shutdown(core, Some(&socket)).await;
    result
}

/// Exit now, skipping the C-library atexit handlers.
///
/// Returning from `main` leaves the process parked in `sigsuspend` inside an
/// exit handler registered by one of the audio/JS dependencies, so a daemon
/// that has finished its own shutdown would never actually terminate. Every
/// piece of state we own is already flushed by the time this is called.
pub fn exit_now(code: i32) -> ! {
    unsafe { libc::_exit(code) }
}

/// Build a runtime and run the daemon to completion.
///
/// macOS Now Playing and the media-key command center need the process main
/// thread running a CFRunLoop, so there the async work moves to a worker and
/// the main thread parks; elsewhere the runtime keeps the main thread.
pub fn block_on_run(args: BootArgs) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    #[cfg(target_os = "macos")]
    {
        player::systemint::init();
        std::thread::spawn(move || {
            let code = match runtime.block_on(run(args)) {
                Ok(()) => 0,
                Err(error) => {
                    tracing::error!(%error, "kopuzd exited with an error");
                    1
                }
            };
            std::process::exit(code);
        });
        player::systemint::park_main_loop();
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let result = runtime.block_on(run(args));
        // Dropping the runtime waits for every blocking task, and the audio
        // engine owns threads that never finish, so a daemon that has
        // decided to exit would hang in teardown instead of exiting.
        runtime.shutdown_background();
        result
    }
}
