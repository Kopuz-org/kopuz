//! Links go to the window already open: a launch that only carries one hands it to the daemon and leaves.

use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::App;

/// Long enough for a source switch and the songs' lookups, which the daemon finishes before it answers.
const HAND_OFF_PATIENCE: Duration = Duration::from_secs(30);

/// Held for the app's lifetime by the process that shows the window.
pub struct Instance(#[allow(dead_code)] File);

/// Take the lock that says this process shows the window, unless another app process holds it.
pub fn claim(address: &link::Address) -> Result<Instance, Elsewhere> {
    let Some(path) = lock_path(address) else {
        return Err(Elsewhere::Unknown);
    };
    // The daemon makes this directory, and this window may be what starts it.
    if let Some(dir) = path.parent()
        && let Err(error) = std::fs::create_dir_all(dir)
    {
        tracing::warn!(%error, dir = %dir.display(), "could not make the app lock's directory");
    }
    let file = match File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "could not open the app lock");
            return Err(Elsewhere::Unknown);
        }
    };
    match file.try_lock() {
        Ok(()) => Ok(Instance(file)),
        Err(TryLockError::WouldBlock) => Err(Elsewhere::Running),
        Err(TryLockError::Error(error)) => {
            tracing::warn!(%error, "could not take the app lock");
            Err(Elsewhere::Unknown)
        }
    }
}

/// Why this process does not hold the lock.
pub enum Elsewhere {
    Running,
    Unknown,
}

/// Beside the daemon's socket, so each flavour and each dev socket has its own; a named pipe has no directory, so Windows uses the temp dir.
fn lock_path(address: &link::Address) -> Option<PathBuf> {
    let link::Address::Socket(socket) = address else {
        return None;
    };
    if cfg!(windows) {
        let name = config::flavor::Flavor::current().name();
        return Some(std::env::temp_dir().join(format!("{name}-app.lock")));
    }
    Some(socket.with_extension("app.lock"))
}

/// Give `url` to the daemon the open window is attached to.
pub fn hand_off(address: &link::Address, url: String) {
    match link::call_once(address, HAND_OFF_PATIENCE, |api| async move {
        api.open_listen_link(url).await
    }) {
        Ok(()) => tracing::info!("handed the link to the open window's daemon"),
        Err(error) => tracing::warn!(%error, "could not hand the link over"),
    }
}

/// Give `url` to the daemon once this window's link to it is up.
pub fn open(url: String, cx: &mut App) {
    let connection = link::Link::connection(cx);
    if connection.read(cx).is_connected() {
        return send(url, cx);
    }
    let mut pending = Some(url);
    cx.observe(&connection, move |connection, cx| {
        if connection.read(cx).is_connected()
            && let Some(url) = pending.take()
        {
            send(url, cx);
        }
    })
    .detach();
}

/// The daemon announces how it went, so only a call that never reached it is worth a line in the log.
fn send(url: String, cx: &mut App) {
    let call =
        link::Link::global(cx).call(move |api| async move { api.open_listen_link(url).await });
    cx.spawn(async move |_| {
        if let Err(error) = call.await {
            tracing::warn!(%error, "the daemon did not play the link");
        }
    })
    .detach();
}
