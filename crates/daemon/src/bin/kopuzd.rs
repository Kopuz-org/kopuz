//! Headless Kopuz daemon (Phase 3 preview).
//!
//! Owns the real audio engine, the Kopuz database, and the configured source,
//! and serves the HTTP/JSON + SSE API from `daemon::http`. Queue contexts
//! resolve from the library (albums, artists, genres, playlists, filters,
//! radio) with a fallback probe for ad-hoc local file paths:
//!
//! ```sh
//! kopuzd
//! TOKEN=$(python3 -c "import json;print(json.load(open('<discovery>'))['token'])")
//! curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:<port>/v1/player
//! curl -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
//!   -d '{"context":{"kind":"album","id":"<album id>"}}' \
//!   http://127.0.0.1:<port>/v1/queue
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
            "--help" | "-h" => {
                return Err(
                    "usage: kopuzd [--bind 127.0.0.1:0] [--token <hex>] [--db-path <file>]"
                        .to_string(),
                );
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn random_token() -> String {
    use rand::RngExt;
    let token: u128 = rand::rng().random();
    format!("{token:032x}")
}

fn discovery_path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join("kopuz"))
        .unwrap_or_else(|| base.cache_dir().join("kopuz"));
    Some(dir.join("daemon.json"))
}

fn write_discovery(path: &Path, port: u16, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "port": port,
        "token": token,
        "pid": std::process::id(),
    });
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Created 0600 so the token is never world-readable, not even between
    // create and chmod; the explicit set below repairs a pre-existing file
    // left behind with wider permissions.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(body.to_string().as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

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
    let db_path = args
        .db_path
        .map(PathBuf::from)
        .unwrap_or_else(db::default_db_path);
    tracing::info!(path = %db_path.display(), "opening library database (expects exclusive access)");
    let database = db::init(&db_path).await?;
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
    let state = Arc::new(daemon::http::HttpState {
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
        token: args.token.unwrap_or_else(random_token),
        started: Instant::now(),
    });

    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    let addr = listener.local_addr()?;

    let discovery = discovery_path();
    match discovery.as_deref() {
        Some(path) => match write_discovery(path, addr.port(), &state.token) {
            Ok(()) => tracing::info!(path = %path.display(), "discovery file written"),
            Err(error) => tracing::warn!(%error, "could not write the discovery file"),
        },
        None => tracing::warn!("no usable directory for the discovery file"),
    }
    tracing::info!(%addr, "kopuzd listening (bearer token in the discovery file)");

    let result = tokio::select! {
        served = daemon::http::serve(listener, state) => served.map_err(Into::into),
        signal = tokio::signal::ctrl_c() => {
            signal?;
            tracing::info!("shutting down");
            Ok(())
        }
    };

    flush_session.persist_now().await;

    if let Some(path) = discovery {
        let _ = std::fs::remove_file(path);
    }
    result
}
