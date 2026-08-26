//! Headless Kopuz daemon.
//!
//! Owns the real audio engine, the Kopuz database, and the configured source,
//! and serves the gRPC API from `daemon::grpc` (see `proto/kopuz.proto`).
//! Queue contexts resolve from the library (albums, artists, genres,
//! playlists, filters, radio) with a fallback probe for ad-hoc local file
//! paths. Reflection is on, so with the token from the discovery file:
//!
//! ```sh
//! kopuzd
//! grpcurl -plaintext -H "authorization: Bearer $TOKEN" \
//!   127.0.0.1:<port> kopuz.v1.Kopuz/GetPlayerState
//! ```
//!
//! The discovery file (path is logged at startup) carries `{port, token, pid}`
//! with 0600 permissions, so local frontends can attach without configuration.
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

struct Args {
    bind: String,
    token: Option<String>,
    db_path: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        bind: "127.0.0.1:0".to_string(),
        token: None,
        db_path: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--bind" => {
                args.bind = iter.next().ok_or("--bind requires an address")?;
            }
            "--token" => {
                args.token = Some(iter.next().ok_or("--token requires a value")?);
            }
            "--db-path" => {
                args.db_path = Some(iter.next().ok_or("--db-path requires a path")?);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

/// Resolves when the process receives SIGTERM (a service manager stop);
/// pends forever on platforms without unix signals.
async fn terminate_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "SIGTERM handler unavailable");
                std::future::pending::<()>().await;
            }
        }
    }
    #[cfg(not(unix))]
    std::future::pending::<()>().await
}

fn main() -> ExitCode {
    let cli_args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = daemon::ctl::run(&cli_args) {
        return ExitCode::from(code.clamp(0, 255) as u8);
    }
    if cli_args.len() == 1 && matches!(cli_args[0].as_str(), "--help" | "-h") {
        daemon::ctl::print_daemon_usage();
        return ExitCode::SUCCESS;
    }

    let log_filter = std::env::var("KOPUZ_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok()
        .and_then(|value| tracing_subscriber::EnvFilter::try_new(value).ok())
        .unwrap_or_else(|| {
            let debug = std::env::var("KOPUZ_DEBUG")
                .is_ok_and(|value| !value.is_empty() && value != "0" && value != "false");
            tracing_subscriber::EnvFilter::new(if debug { "debug" } else { "info" })
        });
    tracing_subscriber::fmt().with_env_filter(log_filter).init();
    tracing::info!(
        mode = "headless",
        version = utils::build_info::VERSION,
        commit = utils::build_info::COMMIT,
        "daemon logging initialized"
    );

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
    let discovery_path = daemon::discovery::path();
    if let Some(path) = discovery_path.as_deref() {
        match daemon::discovery::read(path) {
            Some(existing) if daemon::discovery::is_serving(&existing).await => {
                return Err(format!(
                    "another kopuz daemon is already serving on port {}",
                    existing.port
                )
                .into());
            }
            Some(existing) => {
                let _ = daemon::discovery::remove_record(path, &existing);
            }
            None if path.exists() => {
                let _ = daemon::discovery::remove_invalid(path);
            }
            None => {}
        }
    }
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
    tracing::info!(
        mode = "headless",
        source = %daemon::active_source_label(&config),
        source_id = %config.active_source.as_str(),
        configured_roots = config.music_directory.len()
            + config
                .local_sources
                .iter()
                .map(|source| source.directories.len())
                .sum::<usize>(),
        "daemon configuration loaded"
    );

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
    let scrobbler = daemon::Scrobbler::new(database.clone());
    let recorder = Arc::new(SourceRecorder::new(database.clone()));
    let services = PlaybackServices {
        config,
        active_source: Some(active_source.clone()),
        station_registry,
        queue_store: Some(queue_store.clone()),
        recorder: Some(recorder.clone()),
        scrobbler: Some(scrobbler.clone()),
    };

    let session = SessionHandle::try_spawn(library.clone(), services)
        .map_err(|error| format!("audio engine init failed: {error:?}"))?;
    library.attach_session(session.clone());
    recorder.attach_session(session.clone());
    scrobbler.attach_session(session.clone());
    {
        let scrobbler = scrobbler.clone();
        let config = session.config_watch().borrow().clone();
        tokio::spawn(async move {
            scrobbler.drain_queue(&config).await;
        });
    }
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
    daemon::integrations::spawn_jellyfin_reporter(
        &session,
        database.clone(),
        session.config_watch(),
    );
    daemon::integrations::spawn_discord_presence(&session, session.config_watch());
    daemon::integrations::spawn_credential_maintenance(config_service.clone(), session.clone());
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
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let artwork = daemon::ArtworkService::new(
        database.clone(),
        session.clone(),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("artwork"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-artwork")),
    );
    let frontend = daemon::FrontendService::new(
        database.clone(),
        config_service.clone(),
        library.clone(),
        session.clone(),
        directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("uploaded_artwork"))
            .unwrap_or_else(|| std::env::temp_dir().join("kopuz-uploaded-artwork")),
    );
    frontend.reload_radio().await?;
    tracing::info!(
        mode = "headless",
        "daemon services ready: playback, library, config, jobs, downloads, favorites, artwork, scrobbling, integrations, OS media"
    );
    let state = Arc::new(daemon::grpc::GrpcState {
        api: Arc::new(
            LocalApi::new(session.clone())
                .with_library(library)
                .with_config(config_service)
                .with_jobs(jobs)
                .with_favorites(favorites)
                .with_downloads(downloads)
                .with_frontend(frontend)
                .with_artwork(artwork.clone()),
        ),
        session,
        token: args.token.unwrap_or_else(daemon::discovery::random_token),
        started: Instant::now(),
        shutdown: Some(shutdown.clone()),
    });

    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    let addr = listener.local_addr()?;

    let _discovery_lease = match discovery_path.as_deref() {
        Some(path) => {
            let lease = daemon::discovery::DiscoveryLease::claim(path, addr.port(), &state.token)
                .map_err(|error| format!("could not claim the discovery file: {error}"))?;
            tracing::info!(path = %path.display(), "discovery file written");
            Some(lease)
        }
        None => {
            tracing::warn!("no usable directory for the discovery file");
            None
        }
    };
    tracing::info!(%addr, "kopuzd listening (bearer token in the discovery file)");

    let result = tokio::select! {
        served = daemon::grpc::serve(listener, state) => served.map_err(Into::into),
        signal = tokio::signal::ctrl_c() => {
            signal?;
            tracing::info!("shutting down");
            Ok(())
        }
        _ = terminate_signal() => {
            tracing::info!("SIGTERM received; shutting down");
            Ok(())
        }
        _ = shutdown.notified() => {
            tracing::info!("shutdown requested over the API");
            Ok(())
        }
    };

    flush_session.persist_now().await;
    tracing::info!("daemon shutdown flush finished");

    result
}
