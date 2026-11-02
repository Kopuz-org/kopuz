//! An upgrade leaves the old kopuzd running on this app's own socket; the new app stops it and starts its own.

use std::path::Path;
use std::time::{Duration, Instant};

use gpui_kit::AsyncApp;

/// How long an asked-to-stop kopuzd gets to let go of the socket before it is killed.
const STOP_WAIT: Duration = Duration::from_secs(10);
const STOP_POLL: Duration = Duration::from_millis(250);

/// Whether a daemon at `daemon` is older than this build, which only a newer release than it could be.
pub(crate) fn is_older(daemon: &str) -> bool {
    older_than(daemon, utils::build_info::VERSION)
}

fn older_than(daemon: &str, app: &str) -> bool {
    match (semver::Version::parse(daemon), semver::Version::parse(app)) {
        (Ok(daemon), Ok(app)) => daemon < app,
        _ => false,
    }
}

/// Stop the kopuzd holding `socket`; false when what holds it is not a kopuzd, or it would not go.
pub(crate) async fn stop_daemon(socket: &Path, cx: &AsyncApp) -> bool {
    let Some(pid) = os::holder(socket) else {
        tracing::warn!("could not tell which process holds the daemon socket");
        return false;
    };
    if !os::is_kopuzd(pid) {
        tracing::warn!(
            pid,
            "the daemon socket is held by something other than kopuzd"
        );
        return false;
    }
    tracing::info!(pid, "stopping the older kopuzd");
    for force in [false, true] {
        os::stop(pid, force);
        let asked = Instant::now();
        while asked.elapsed() < STOP_WAIT {
            cx.background_executor().timer(STOP_POLL).await;
            if !crate::socket_taken(socket) {
                return true;
            }
        }
    }
    false
}

#[cfg(target_os = "linux")]
mod os {
    use std::os::fd::AsRawFd as _;
    use std::path::Path;

    pub(super) fn holder(socket: &Path) -> Option<u32> {
        let stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` and `len` describe a writable ucred, as SO_PEERCRED expects.
        let read = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&raw mut cred).cast(),
                &mut len,
            )
        };
        (read == 0 && cred.pid > 0).then_some(cred.pid as u32)
    }

    /// A replaced binary still reads as `kopuzd (deleted)`, and Nightly's as `kopuzd-nightly`.
    pub(super) fn is_kopuzd(pid: u32) -> bool {
        std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|exe| {
                exe.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .is_some_and(|name| name.starts_with("kopuzd"))
    }

    pub(super) fn stop(pid: u32, force: bool) {
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        // SAFETY: kill only sends a signal; the pid was just read off the socket's peer.
        unsafe { libc::kill(pid as libc::pid_t, signal) };
    }
}

#[cfg(windows)]
mod os {
    use std::os::windows::io::AsRawHandle as _;
    use std::path::Path;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        QueryFullProcessImageNameW, TerminateProcess,
    };
    use windows::core::PWSTR;

    pub(super) fn holder(pipe: &Path) -> Option<u32> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe)
            .ok()?;
        let mut pid = 0;
        // SAFETY: the handle is the open pipe client `file` owns.
        unsafe { GetNamedPipeServerProcessId(HANDLE(file.as_raw_handle()), &mut pid) }.ok()?;
        (pid != 0).then_some(pid)
    }

    pub(super) fn is_kopuzd(pid: u32) -> bool {
        // SAFETY: the process handle is closed before returning, and the buffer outlives the call.
        unsafe {
            let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut buffer = [0u16; 1024];
            let mut len = buffer.len() as u32;
            let named = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(process);
            named.is_ok()
                && Path::new(&String::from_utf16_lossy(&buffer[..len as usize]))
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("kopuzd"))
        }
    }

    /// A windowless process has no gentler way to be asked, so both tries terminate it.
    pub(super) fn stop(pid: u32, _force: bool) {
        // SAFETY: the process handle is closed before returning.
        unsafe {
            if let Ok(process) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(process, 0);
                let _ = CloseHandle(process);
            }
        }
    }
}

// macOS is left to say what is wrong, as before.
#[cfg(not(any(target_os = "linux", windows)))]
mod os {
    use std::path::Path;

    pub(super) fn holder(_: &Path) -> Option<u32> {
        None
    }

    pub(super) fn is_kopuzd(_: u32) -> bool {
        false
    }

    pub(super) fn stop(_: u32, _: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_strictly_older_release_is_replaced() {
        assert!(older_than("0.67.0", "0.68.0"));
        assert!(older_than(
            "0.68.0-nightly.20261027",
            "0.68.0-nightly.20261028"
        ));
        assert!(older_than("0.68.0-nightly.20261028", "0.68.0"));
        assert!(!older_than("0.68.0", "0.68.0"));
        assert!(!older_than("0.69.0", "0.68.0"));
        assert!(!older_than("garbage", "0.68.0"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_holder_of_a_socket_is_found_and_this_test_is_no_kopuzd() {
        let socket = std::env::temp_dir().join(format!("kopuz-holder-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        assert_eq!(os::holder(&socket), Some(std::process::id()));
        assert!(!os::is_kopuzd(std::process::id()));
        std::fs::remove_file(&socket).unwrap();
    }
}
