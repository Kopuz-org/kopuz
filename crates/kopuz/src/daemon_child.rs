//! Running the daemon as a child of this same executable.
//!
//! `cargo run -p kopuz` builds one binary, so the frontend re-executes
//! itself with `--run-daemon` rather than depending on a separate `kopuzd`
//! being built and on PATH. Same code, same build, two processes with two
//! lifetimes and two logs.
//!
//! Coupling is the socket, not process parentage. A child is reparented to
//! init when its parent dies, so a subprocess alone would leave an orphaned
//! daemon still playing; instead the daemon runs `--supervised` and exits
//! when the frontend's stream ends, and the frontend exits when a call comes
//! back `DaemonGone`. That works for a kill as well as a clean exit, because
//! the kernel closes the socket either way.

use std::path::PathBuf;

/// The hidden flag that turns this executable into the daemon.
pub const RUN_DAEMON: &str = "--run-daemon";

/// How the frontend was asked to reach a daemon.
pub enum Mode {
    /// Own one: spawn a supervised child and die with it.
    Spawn,
    /// Attach to a daemon someone else owns; leave it running on exit.
    Attach(PathBuf),
    /// No daemon; the app owns the database itself, as it always has.
    None,
}

pub fn mode_from_args() -> Mode {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg != "--daemon" {
            continue;
        }
        // Only a bare value is a socket; a following flag belongs to
        // whoever else parses it, not to --daemon.
        return match args.next().as_deref() {
            Some(path) if path != "spawn" && !path.starts_with('-') => {
                Mode::Attach(PathBuf::from(path.trim_start_matches("unix:")))
            }
            _ => Mode::Spawn,
        };
    }
    Mode::None
}

/// True when this process was re-executed to be the daemon.
pub fn is_daemon_process() -> bool {
    std::env::args().any(|arg| arg == RUN_DAEMON)
}

/// Run the daemon in this process, having been spawned with [`RUN_DAEMON`].
pub fn run_as_daemon() -> std::process::ExitCode {
    let _log_guard = daemon::boot::init_logging();
    let mut boot = daemon::boot::BootArgs {
        supervised: true,
        ..Default::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => boot.socket = args.next().map(PathBuf::from),
            "--db-path" => boot.db_path = args.next(),
            _ => {}
        }
    }
    let code = match daemon::boot::block_on_run(boot) {
        Ok(()) => 0,
        Err(error) => {
            tracing::error!(%error, "the daemon child exited with an error");
            1
        }
    };
    tracing::info!("daemon exiting");
    // Dropping the appender guard joins its worker, which does not finish
    // once the runtime is gone; give it a moment to drain instead.
    std::thread::sleep(std::time::Duration::from_millis(150));
    std::mem::forget(_log_guard);
    daemon::boot::exit_now(code)
}

/// Spawn the daemon as a child of this executable and wait for its socket.
///
/// The handle is dropped deliberately: dropping a Child neither kills nor
/// reaps it, and the daemon needs neither. `--supervised` makes the
/// frontend's disconnect its exit signal, which fires however this process
/// died.
pub fn spawn() -> Result<(std::process::Child, PathBuf), String> {
    let exe =
        std::env::current_exe().map_err(|error| format!("no path to this binary: {error}"))?;
    let socket = daemon::boot::default_socket_path()
        .ok_or_else(|| "no usable runtime directory for the daemon socket".to_string())?;

    let child = std::process::Command::new(&exe)
        .arg(RUN_DAEMON)
        .arg("--socket")
        .arg(&socket)
        .arg("--db-path")
        .arg(daemon_db_path())
        .spawn()
        .map_err(|error| format!("could not start the daemon: {error}"))?;

    wait_for_socket(&socket)?;
    Ok((child, socket))
}

/// Exit when the daemon does.
///
/// A blocking wait on the child is the reliable signal here: the kernel
/// reports its death exactly once, however it died. The event stream cannot
/// stand in for this -- an idle Subscribe does not surface the peer going
/// away -- so the socket covers the frontend's death and this covers the
/// daemon's.
fn follow_child(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let status = child.wait();
        tracing::error!(
            ?status,
            "the daemon exited; shutting the frontend down with it"
        );
        std::process::exit(1);
    });
}

/// A database of the daemon's own, beside the app's.
///
/// Temporary: the app still opens its library directly, and SQLite takes one
/// writer, so a daemon on the same file would fight it -- in release that
/// file is the real library. Nothing reads through the daemon yet, so it
/// loses nothing by starting empty. Drop this once the app reads through the
/// API and the daemon owns the library outright.
fn daemon_db_path() -> PathBuf {
    let app = db::default_db_path();
    match app.file_name().and_then(|name| name.to_str()) {
        Some(name) => app.with_file_name(format!("daemon-{name}")),
        None => app,
    }
}

/// The daemon binds before it serves, so the socket appearing and accepting
/// is the readiness signal. Nothing to poll over gRPC.
fn wait_for_socket(path: &std::path::Path) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Err(format!(
        "the daemon did not start serving {} in time",
        path.display()
    ))
}

/// Reach a daemon and bind this process's lifetime to it: when the daemon
/// goes, the frontend follows.
pub fn attach(mode: Mode) -> Result<(), String> {
    let socket = match mode {
        Mode::Spawn => {
            let (child, socket) = spawn()?;
            tracing::info!(path = %socket.display(), "spawned a supervised daemon");
            follow_child(child);
            socket
        }
        Mode::Attach(socket) => {
            std::os::unix::net::UnixStream::connect(&socket)
                .map_err(|error| format!("no daemon at {}: {error}", socket.display()))?;
            tracing::info!(path = %socket.display(), "attached to a running daemon");
            socket
        }
        Mode::None => unreachable!("attach is only called for a daemon mode"),
    };
    hold_attachment(&socket)?;
    Ok(())
}

/// Hold an event subscription for the life of the process.
///
/// This is what makes the daemon count a frontend as attached: supervision
/// keys off the Subscribe stream, so a connection that only makes unary
/// calls leaves a `--supervised` daemon believing it was never adopted, and
/// it outlives the frontend it was spawned for. The first `player_state`
/// also doubles as readiness -- a socket that accepts but does not serve
/// would otherwise only surface later as a puzzling empty UI.
fn hold_attachment(socket: &std::path::Path) -> Result<(), String> {
    use api::KopuzApi;
    use futures_util::StreamExt;

    let socket = socket.to_path_buf();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = ready_tx.send(Err(format!("no runtime to reach the daemon: {error}")));
                return;
            }
        };
        runtime.block_on(async move {
            let api = match client::GrpcApi::new(&socket) {
                Ok(api) => api,
                Err(error) => {
                    let _ = ready_tx.send(Err(error.to_string()));
                    return;
                }
            };
            if let Err(error) = api.player_state().await {
                let _ = ready_tx.send(Err(format!("the daemon is not answering: {error}")));
                return;
            }

            // Ready means attached, not merely answering: Subscribe greets
            // with one event, and only once the daemon has that stream does
            // it count this frontend as one to outlive.
            let mut events = api.events();
            if events.next().await.is_none() {
                let _ = ready_tx.send(Err("the daemon closed the event stream".to_string()));
                return;
            }
            tracing::info!("attached to the daemon");
            let _ = ready_tx.send(Ok(()));

            while events.next().await.is_some() {}
            tracing::error!("the daemon exited; shutting the frontend down with it");
            std::process::exit(1);
        });
    });

    match ready_rx.recv_timeout(std::time::Duration::from_secs(15)) {
        Ok(result) => result,
        Err(_) => Err("the daemon did not answer in time".to_string()),
    }
}
