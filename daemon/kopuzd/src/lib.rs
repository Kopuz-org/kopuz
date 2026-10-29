//! Serving a [`daemon::boot::Core`] over gRPC on a Unix domain socket, or
//! on Windows a named pipe.
//!
//! The core knows nothing about this crate: it is a library of services with
//! a `LocalApi` over them. Here it gains a wire (see `proto/kopuz.proto`), a
//! socket, and -- in the `kopuzd` binary -- a process lifecycle.
//!
//! The socket path is the whole rendezvous: a frontend opens it or it does
//! not exist. Its 0600 mode is the access control, so the channel carries no
//! credentials; the pipe carries a DACL that draws the same line (see
//! `proto::pipe`). A daemon started with `--listen` also serves plain
//! HTTP/2 on whatever address it is given, with no credentials either.
//! Reflection is on:
//!
//! ```sh
//! kopuzd
//! grpcurl -unix -plaintext \
//!   $XDG_RUNTIME_DIR/kopuz/kopuzd.sock kopuz.v1.Kopuz/GetPlayerState
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use daemon::boot::{Core, CoreArgs};

pub mod identity;
pub mod service;
pub mod updates;

pub use service::{GrpcState, bind_socket, serve, serve_tcp};

/// What the daemon binary needs on top of a core.
#[derive(Debug, Default)]
pub struct ServeArgs {
    pub socket: Option<PathBuf>,
    /// A TCP address to serve on as well.
    pub listen: Option<std::net::SocketAddr>,
    pub db_path: Option<String>,
    pub identity: Option<identity::AppIdentity>,
    /// Start in offline mode: listings and playback keep to downloads and cached songs until `SetOffline` leaves it.
    pub offline: bool,
    /// Exit once no app has been connected for a while and nothing plays: the daemon an app starts for itself.
    pub exit_with_app: bool,
}

impl ServeArgs {
    fn core(&self) -> CoreArgs {
        CoreArgs {
            db_path: self.db_path.clone(),
            offline: self.offline,
        }
    }
}

pub use proto::default_socket_path;

/// Only the kopuzd binary updates itself: a frontend that hosts the core ships its own updater.
fn state(
    core: &Core,
    updater: Option<&Arc<updates::Updater>>,
    clients: &service::Clients,
) -> Arc<GrpcState> {
    Arc::new(GrpcState {
        api: core.api.clone(),
        artwork: Some(core.artwork.clone()),
        session: core.session.clone(),
        updater: updater.cloned(),
        started: Instant::now(),
        clients: clients.clone(),
    })
}

/// Bind the socket, then hand back the future that serves the core on it
/// until the server stops. The address is released with the listener, so a
/// path that could not be bound is never touched.
pub fn listen(
    core: &Core,
    socket: &Path,
) -> std::io::Result<impl std::future::Future<Output = std::io::Result<()>> + use<>> {
    let listener = bind_socket(socket)?;
    tracing::info!(path = %socket.display(), "kopuzd listening");
    Ok(serve(
        listener,
        state(core, None, &service::Clients::default()),
    ))
}

async fn bind_tcp(address: std::net::SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(bound = %listener.local_addr()?, "kopuzd listening on tcp");
    Ok(listener)
}

async fn serve_tcp_if(
    tcp: Option<tokio::net::TcpListener>,
    state: Arc<GrpcState>,
) -> std::io::Result<()> {
    match tcp {
        Some(listener) => serve_tcp(listener, state).await,
        None => std::future::pending().await,
    }
}

/// No app connected and nothing playing for this long ends a daemon started with `--exit-with-app`.
const APP_GONE_AFTER: std::time::Duration = std::time::Duration::from_secs(15);

/// Resolves once the apps a `--exit-with-app` daemon serves have all gone and nothing is playing; never otherwise.
async fn apps_gone(exit_with_app: bool, core: &Core, clients: &service::Clients) {
    use api::prelude::*;
    if !exit_with_app {
        return std::future::pending().await;
    }
    let playing_now = async || {
        core.api
            .player_state()
            .await
            .is_ok_and(|state| state.phase == api::Phase::Playing)
    };
    // The phase follows the session's events, so the once-a-second check only reads a counter.
    let mut events = core.session.subscribe();
    let mut playing = playing_now().await;
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    let mut idle_since: Option<Instant> = None;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(api::ApiEvent::PlayerState(state)) => playing = state.phase == api::Phase::Playing,
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => playing = playing_now().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return std::future::pending().await;
                }
            },
            _ = tick.tick() => match clients.count() == 0 && !playing {
                true if idle_since.get_or_insert_with(Instant::now).elapsed() >= APP_GONE_AFTER => {
                    return;
                }
                true => {}
                false => idle_since = None,
            },
        }
    }
}

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

pub async fn run(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let core = daemon::boot::assemble(&args.core()).await?;

    let Some(socket) = args.socket.clone().or_else(default_socket_path) else {
        return Err("no usable address for the daemon socket".into());
    };

    let listener = bind_socket(&socket)?;
    let clients = service::Clients::default();
    let updater = updates::Updater::new(core.session.clone());
    updater.spawn_checks(core.config_service.clone());
    tracing::info!(path = %socket.display(), "kopuzd listening");
    let tcp = match args.listen {
        Some(address) => Some(bind_tcp(address).await?),
        None => None,
    };
    let result = tokio::select! {
        served = serve(listener, state(&core, Some(&updater), &clients)) => served.map_err(Into::into),
        served = serve_tcp_if(tcp, state(&core, Some(&updater), &clients)) => served.map_err(Into::into),
        signal = tokio::signal::ctrl_c() => {
            signal?;
            tracing::info!("shutting down");
            Ok(())
        }
        () = apps_gone(args.exit_with_app, &core, &clients) => {
            tracing::info!("no app left and nothing playing; shutting down");
            Ok(())
        }
        () = updater.exit_requested() => {
            tracing::info!("shutting down to apply an update");
            Ok(())
        }
        () = terminate_signal() => {
            tracing::info!("shutting down");
            Ok(())
        }
    };

    daemon::boot::shutdown(core).await;
    result
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
    Some(
        directories::BaseDirs::new()?
            .cache_dir()
            .join(config::flavor::Flavor::current().name())
            .join("logs"),
    )
}

/// Build a runtime and run the daemon to completion.
///
/// macOS Now Playing and the media-key command center need the process main
/// thread running a CFRunLoop, so there the async work moves to a worker and
/// the main thread parks; elsewhere the runtime keeps the main thread.
pub fn block_on_run(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(any(windows, target_os = "linux"))]
    if let Some(identity) = &args.identity {
        identity.apply();
    }
    daemon::boot::prepare_thread();
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

        runtime.shutdown_background();
        result
    }
}
