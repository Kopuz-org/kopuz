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
//! HTTP/2 on a port, where a bearer token stands in for the OS boundary
//! (see [`Token`]). Reflection is on:
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

pub mod service;

pub use service::{GrpcState, Token, bind_socket, serve, serve_tcp};

/// What the daemon binary needs on top of a core.
#[derive(Debug, Default)]
pub struct ServeArgs {
    pub socket: Option<PathBuf>,
    /// A TCP address to serve on as well, gated on the token.
    pub listen: Option<std::net::SocketAddr>,
    pub token_path: Option<PathBuf>,
    pub db_path: Option<String>,
}

impl ServeArgs {
    fn core(&self) -> CoreArgs {
        CoreArgs {
            db_path: self.db_path.clone(),
        }
    }
}

#[cfg(windows)]
pub fn default_socket_path() -> Option<PathBuf> {
    proto::pipe::default_name().ok().map(PathBuf::from)
}

#[cfg(not(windows))]
pub fn default_socket_path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join("kopuz"))
        .unwrap_or_else(|| base.cache_dir().join("kopuz"));
    Some(dir.join("kopuzd.sock"))
}

/// Where the TCP token lives unless told otherwise: beside the socket on
/// Unix, in the user's cache dir on Windows.
pub fn default_token_path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let dir = base
        .runtime_dir()
        .map(|runtime| runtime.join("kopuz"))
        .unwrap_or_else(|| base.cache_dir().join("kopuz"));
    Some(dir.join("kopuzd.token"))
}

fn state(core: &Core) -> Arc<GrpcState> {
    Arc::new(GrpcState {
        api: core.api.clone(),
        artwork: Some(core.artwork.clone()),
        session: core.session.clone(),
        started: Instant::now(),
    })
}

/// Bind the socket and serve the core on it until the server stops. The
/// address is released with the listener, so a path that could not be
/// bound is never touched.
pub async fn listen(core: &Core, socket: &Path) -> std::io::Result<()> {
    let listener = bind_socket(socket)?;
    tracing::info!(path = %socket.display(), "kopuzd listening");
    serve(listener, state(core)).await
}

/// Bind the port and settle the token that guards it. A port beyond
/// loopback is reachable from the network, which is worth a warning since
/// the token is then the only thing between it and the library.
async fn bind_tcp(
    address: std::net::SocketAddr,
    token_path: Option<PathBuf>,
) -> Result<(tokio::net::TcpListener, Token), Box<dyn std::error::Error>> {
    let Some(token_path) = token_path.or_else(default_token_path) else {
        return Err("no usable path for the daemon token".into());
    };
    let token = Token::load_or_create(&token_path)?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    let bound = listener.local_addr()?;
    if !bound.ip().is_loopback() {
        tracing::warn!(%bound, "listening beyond loopback; the token is the only guard on this port");
    }
    tracing::info!(%bound, token = %token_path.display(), "kopuzd listening on tcp");
    Ok((listener, token))
}

async fn serve_tcp_if(
    tcp: Option<(tokio::net::TcpListener, Token)>,
    state: Arc<GrpcState>,
) -> std::io::Result<()> {
    match tcp {
        Some((listener, token)) => serve_tcp(listener, token, state).await,
        None => std::future::pending().await,
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
    tracing::info!(path = %socket.display(), "kopuzd listening");
    let tcp = match args.listen {
        Some(address) => Some(bind_tcp(address, args.token_path.clone()).await?),
        None => None,
    };
    let result = tokio::select! {
        served = serve(listener, state(&core)) => served.map_err(Into::into),
        served = serve_tcp_if(tcp, state(&core)) => served.map_err(Into::into),
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
    Some(directories::BaseDirs::new()?.cache_dir().join("kopuz/logs"))
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
pub fn block_on_run(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
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
        // Dropping the runtime waits for every blocking task, and the audio
        // engine owns threads that never finish, so a daemon that has decided
        // to exit would hang in teardown instead of exiting.
        runtime.shutdown_background();
        result
    }
}
