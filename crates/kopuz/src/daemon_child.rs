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
        return match args.next().as_deref() {
            None | Some("spawn") => Mode::Spawn,
            Some(path) => Mode::Attach(PathBuf::from(path.trim_start_matches("unix:"))),
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
    match daemon::boot::block_on_run(boot) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the daemon child exited with an error");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Spawn the daemon as a child of this executable and wait for its socket.
///
/// The handle is dropped deliberately: dropping a Child neither kills nor
/// reaps it, and the daemon needs neither. `--supervised` makes the
/// frontend's disconnect its exit signal, which fires however this process
/// died.
pub fn spawn() -> Result<PathBuf, String> {
    let exe =
        std::env::current_exe().map_err(|error| format!("no path to this binary: {error}"))?;
    let socket = daemon::boot::default_socket_path()
        .ok_or_else(|| "no usable runtime directory for the daemon socket".to_string())?;

    std::process::Command::new(&exe)
        .arg(RUN_DAEMON)
        .arg("--socket")
        .arg(&socket)
        .spawn()
        .map_err(|error| format!("could not start the daemon: {error}"))?;

    wait_for_socket(&socket)?;
    Ok(socket)
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
            let socket = spawn()?;
            tracing::info!(path = %socket.display(), "spawned a supervised daemon");
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
    probe(&socket)?;
    watch_for_exit(&socket);
    Ok(())
}

/// One real RPC before the window opens. A socket that accepts but does not
/// serve would otherwise only surface later, as a puzzling empty UI.
fn probe(socket: &std::path::Path) -> Result<(), String> {
    use api::KopuzApi;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("no runtime to reach the daemon: {error}"))?;
    let api = client::GrpcApi::new(socket).map_err(|error| error.to_string())?;
    let state = runtime
        .block_on(api.player_state())
        .map_err(|error| format!("the daemon is not answering: {error}"))?;
    tracing::info!(phase = ?state.phase, "daemon ready");
    Ok(())
}

/// Exit when the daemon does. The socket closing is the signal, so this
/// covers a clean shutdown, a panic, and a kill alike -- the frontend has no
/// backend to draw once it is gone.
fn watch_for_exit(socket: &std::path::Path) {
    let socket = socket.to_path_buf();
    std::thread::spawn(move || {
        use std::io::Read;
        let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&socket) else {
            return;
        };
        // Never written to by the daemon, so this blocks until the peer
        // closes and then returns 0 bytes.
        let mut byte = [0u8; 1];
        let _ = stream.read(&mut byte);
        tracing::error!("the daemon exited; shutting the frontend down with it");
        std::process::exit(1);
    });
}
