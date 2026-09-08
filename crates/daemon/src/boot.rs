//! Daemon startup: logging, socket, services, and the shutdown path.
//!
//! Lives in the library rather than the `kopuzd` binary so a frontend can
//! host the same daemon in a child process of its own executable, which is
//! what lets one `cargo run` produce both halves.
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
//! Interim caveat: the daemon expects exclusive database access. Running it
//! alongside the GUI app against the same `KOPUZ_DB_PATH` means two writers
//! on one SQLite file; safe for reads, but not the supported end state.

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

pub async fn run(args: BootArgs) -> Result<(), Box<dyn std::error::Error>> {
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
        .map(PathBuf::from)
        .unwrap_or_else(db::default_db_path);
    tracing::info!(path = %db_path.display(), "opening library database (expects exclusive access)");
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
    let services = PlaybackServices {
        config,
        active_source: Some(active_source.clone()),
        station_registry,
        queue_store: Some(queue_store.clone()),
        recorder: Some(Arc::new(SourceRecorder::new(active_source.clone()))),
    };

    let session = SessionHandle::try_spawn(library.clone(), services)
        .map_err(|error| format!("audio engine init failed: {error:?}"))?;
    library.attach_session(session.clone());
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
    let flush_session = session.clone();
    let artwork = crate::ArtworkService::new(
        database.clone(),
        session.clone(),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("artwork"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-artwork")),
    );
    let state = Arc::new(crate::grpc::GrpcState {
        api: Arc::new(
            LocalApi::new(session.clone())
                .with_library(library)
                .with_config(config_service)
                .with_jobs(jobs)
                .with_favorites(favorites)
                .with_downloads(downloads),
        ),
        artwork: Some(artwork),
        session,
        started: Instant::now(),
        supervisor: args
            .supervised
            .then(|| Arc::new(crate::grpc::Supervisor::default())),
    });

    let socket = match args.socket.or_else(default_socket_path) {
        Some(path) => path,
        None => {
            return Err("no usable runtime directory for the daemon socket".into());
        }
    };
    let listener = crate::grpc::bind_socket(&socket)?;
    tracing::info!(path = %socket.display(), "kopuzd listening");

    let supervisor = state.supervisor.clone();
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
        served = crate::grpc::serve(listener, state) => served.map_err(Into::into),
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

    flush_session.persist_now().await;

    let _ = std::fs::remove_file(&socket);
    result
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
    runtime.block_on(run(args))
}
