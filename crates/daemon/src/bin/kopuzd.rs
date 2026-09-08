//! Headless Kopuz daemon.
//!
//! Owns the real audio engine, the Kopuz database, and the configured source,
//! and serves the gRPC API from `daemon::grpc` (see `proto/kopuz.proto`).
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
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use daemon::{
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

struct Args {
    socket: Option<PathBuf>,
    db_path: Option<String>,
    /// Launched by a frontend: exit when that frontend goes away.
    supervised: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        socket: None,
        db_path: None,
        supervised: false,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--socket" => {
                args.socket = Some(PathBuf::from(
                    iter.next().ok_or("--socket requires a path")?,
                ));
            }
            "--supervised" => {
                args.supervised = true;
            }
            "--db-path" => {
                args.db_path = Some(iter.next().ok_or("--db-path requires a path")?);
            }
            "--help" | "-h" => {
                return Err("usage: kopuzd [--socket <path>] [--db-path <file>]".to_string());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

/// The daemon's own log, next to the frontend's in the same directory.
/// Returns the appender guard, which must outlive `main`.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
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

fn default_socket_path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join("kopuz"))
        .unwrap_or_else(|| base.cache_dir().join("kopuz"));
    Some(dir.join("kopuzd.sock"))
}

fn main() -> ExitCode {
    // Kept out of the frontend's file on purpose: two processes with two
    // lifetimes interleaved into one log make either side's history
    // unreadable, and a supervised daemon's exit is exactly what you want to
    // read after the GUI is gone.
    let _log_guard = init_logging();

    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            tracing::error!("{message}");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "failed to build the tokio runtime");
            return ExitCode::FAILURE;
        }
    };

    // macOS: Now Playing and the media-key command center need the process
    // main thread to run a CFRunLoop, so async work moves to a worker thread
    // and the main thread parks in the loop. Elsewhere the runtime keeps the
    // main thread.
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
        ExitCode::SUCCESS
    }

    #[cfg(not(target_os = "macos"))]
    match runtime.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "kopuzd exited with an error");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
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
    let downloads = daemon::DownloadsService::new(
        database.clone(),
        session.clone(),
        config_service.clone(),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("offline_tracks"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-offline")),
    );
    let favorites = FavoritesService::new(database.clone(), session.clone());
    favorites.spawn_reconciler();
    daemon::os_media::spawn(&session);
    daemon::integrations::spawn_jellyfin_reporter(&session, active_source, session.config_watch());
    daemon::integrations::spawn_discord_presence(&session, session.config_watch());
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
    let artwork = daemon::ArtworkService::new(
        database.clone(),
        session.clone(),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("artwork"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-artwork")),
    );
    let state = Arc::new(daemon::grpc::GrpcState {
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
            .then(|| Arc::new(daemon::grpc::Supervisor::default())),
    });

    let socket = match args.socket.or_else(default_socket_path) {
        Some(path) => path,
        None => {
            return Err("no usable runtime directory for the daemon socket".into());
        }
    };
    let listener = daemon::grpc::bind_socket(&socket)?;
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
        served = daemon::grpc::serve(listener, state) => served.map_err(Into::into),
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
