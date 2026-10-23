use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Start `kopuzd` detached (`KOPUZD` overrides the binary); the supervisor attaches on its next retry.
pub fn spawn_daemon(socket: Option<PathBuf>) -> std::io::Result<()> {
    let program = std::env::var_os("KOPUZD").unwrap_or_else(|| "kopuzd".into());
    let mut command = Command::new(program);
    if let Some(socket) = socket {
        command.arg("--socket").arg(socket);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    command.spawn().map(drop)
}
