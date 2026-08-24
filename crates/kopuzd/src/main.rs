//! Headless Kopuz daemon (Phase 3 preview).
//!
//! Owns the real audio engine and serves the HTTP/JSON + SSE API from
//! `daemon::http`. Library, config, and source services have not moved in yet,
//! so queue contexts resolve local file paths only:
//!
//! ```sh
//! kopuzd
//! TOKEN=$(python3 -c "import json;print(json.load(open('<discovery>'))['token'])")
//! curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:<port>/v1/player
//! curl -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
//!   -d '{"context":{"kind":"tracks","keys":["/path/to/song.flac"]}}' \
//!   http://127.0.0.1:<port>/v1/queue
//! ```
//!
//! The discovery file (path is logged at startup) carries `{port, token, pid}`
//! with 0600 permissions, so local frontends can attach without configuration.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use api::{ApiError, QueueContext};
use daemon::{PlaybackServices, QueueMaterializer, SessionHandle};
use reader::Track;

struct LocalFiles;

#[async_trait::async_trait]
impl QueueMaterializer for LocalFiles {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        let QueueContext::Tracks { keys } = context else {
            return Err(ApiError::unsupported(
                "this preview daemon resolves only local file paths (context kind \"tracks\")",
            ));
        };
        let keys = keys.clone();
        let tracks = tokio::task::spawn_blocking(move || {
            let cover_cache = std::env::temp_dir();
            let mut library = reader::Library::default();
            keys.iter()
                .filter_map(|key| {
                    let path = Path::new(key);
                    path.is_file()
                        .then(|| reader::read(path, &cover_cache, &mut library))
                        .flatten()
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|_| ApiError::internal("track probe task failed"))?;
        if tracks.is_empty() {
            return Err(ApiError::invalid_input(
                "no readable local audio files in request",
            ));
        }
        Ok(tracks)
    }
}

struct Args {
    bind: String,
    token: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        bind: "127.0.0.1:0".to_string(),
        token: None,
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
            "--help" | "-h" => {
                return Err("usage: kopuzd [--bind 127.0.0.1:0] [--token <hex>]".to_string());
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

    match runtime.block_on(run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "kopuzd exited with an error");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let session = SessionHandle::try_spawn(Arc::new(LocalFiles), PlaybackServices::default())
        .map_err(|error| format!("audio engine init failed: {error:?}"))?;
    let state = Arc::new(daemon::http::HttpState {
        api: Arc::new(daemon::LocalApi::new(session.clone())),
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

    if let Some(path) = discovery {
        let _ = std::fs::remove_file(path);
    }
    result
}
