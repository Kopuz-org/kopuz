use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The app's flag for running the kopuzd packaged beside it instead of itself.
pub const DAEMON_FLAG: &str = "--daemon";

/// Start `kopuzd` detached on `socket`, told to exit once no app is left on it.
pub(crate) fn spawn_daemon(socket: &Path) -> std::io::Result<Child> {
    let (program, through_app) = daemon_program();
    let mut command = Command::new(program);
    if through_app {
        command.arg(DAEMON_FLAG);
    }
    command
        .arg("--socket")
        .arg(socket)
        .arg("--exit-with-app")
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
    command.spawn()
}

/// `KOPUZD`, else the AppImage run with [`DAEMON_FLAG`], else this flavour's kopuzd beside this binary, else `PATH`; true when it goes through the app.
fn daemon_program() -> (OsString, bool) {
    if let Some(program) = std::env::var_os("KOPUZD") {
        return (program, false);
    }
    // The AppImage's mount goes when the app exits, so a daemon outliving it must run from a mount of its own.
    if let Some(image) = std::env::var_os("APPIMAGE").filter(|image| Path::new(image).is_file()) {
        return (image, true);
    }
    (bundled_daemon(), false)
}

/// This flavour's kopuzd beside this binary (an .app or install dir is off `PATH`), else `PATH`.
fn bundled_daemon() -> OsString {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(sibling_daemon))
        .map(PathBuf::into_os_string)
        .unwrap_or_else(|| daemon_name().into())
}

/// `kopuzd`, or `kopuzd-nightly` for Kopuz Nightly, so the two apps never start each other's daemon.
fn daemon_name() -> String {
    config::flavor::Flavor::current().binary("kopuzd")
}

/// Run as kopuzd when the app is started with [`DAEMON_FLAG`], answering its exit code; `None` when it was not.
pub fn run_daemon_flag(args: &[OsString]) -> Option<i32> {
    let (flag, rest) = args.split_first()?;
    if flag != DAEMON_FLAG {
        return None;
    }
    let mut command = Command::new(bundled_daemon());
    command.args(rest);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // exec only returns on failure; the daemon's own pid is what a service manager tracks.
        let _ = command.exec();
        Some(1)
    }
    #[cfg(not(unix))]
    Some(
        command
            .status()
            .ok()
            .and_then(|status| status.code())
            .unwrap_or(1),
    )
}

fn sibling_daemon(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(format!("{}{}", daemon_name(), std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_leading_daemon_flag_turns_the_app_into_kopuzd() {
        assert_eq!(run_daemon_flag(&[]), None);
        assert_eq!(
            run_daemon_flag(&["--socket".into(), DAEMON_FLAG.into()]),
            None
        );
    }

    #[test]
    fn finds_kopuzd_only_when_it_sits_beside_the_app() {
        let dir = std::env::temp_dir().join(format!("kopuz-sibling-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(sibling_daemon(&dir), None);
        let daemon = dir.join(format!("{}{}", daemon_name(), std::env::consts::EXE_SUFFIX));
        std::fs::write(&daemon, b"").unwrap();
        assert_eq!(sibling_daemon(&dir), Some(daemon));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
