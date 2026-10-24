use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Start `kopuzd` detached; the supervisor attaches on its next retry.
pub fn spawn_daemon(socket: Option<PathBuf>) -> std::io::Result<()> {
    let mut command = Command::new(daemon_program());
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
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map(drop)
}

/// `KOPUZD`, else the kopuzd packaged beside this binary (an .app or install dir is off `PATH`), else `PATH`.
fn daemon_program() -> OsString {
    if let Some(program) = std::env::var_os("KOPUZD") {
        return program;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(sibling_daemon))
        .map(PathBuf::into_os_string)
        .unwrap_or_else(|| "kopuzd".into())
}

fn sibling_daemon(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(format!("kopuzd{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_kopuzd_only_when_it_sits_beside_the_app() {
        let dir = std::env::temp_dir().join(format!("kopuz-sibling-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(sibling_daemon(&dir), None);
        let daemon = dir.join(format!("kopuzd{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&daemon, b"").unwrap();
        assert_eq!(sibling_daemon(&dir), Some(daemon));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
