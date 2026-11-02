//! The daemon core the app talks to.
//!
//! One process: the app hosts the same core `kopuzd` runs and reaches it
//! through [`api::KopuzApi`], and serves it on the socket so other frontends,
//! `kopuzctl` and `grpcurl` can attach to the running app. Nothing is spawned
//! and nothing is supervised -- there is only one player, one library, and one
//! registration with the OS media keys.
//!
//! The core runs on its own runtime, on its own thread, for the life of the
//! process: Dioxus owns the main thread, and every service the core spawns
//! (the session actor, the reconciler, the integrations) has to outlive any
//! single UI future.

use std::sync::{OnceLock, mpsc};

use daemon::boot::{Core, CoreArgs};

static CORE: OnceLock<Core> = OnceLock::new();

/// What the app was launched with that the core it hosts takes: `--offline` starts it in offline mode.
pub fn core_args(argv: impl IntoIterator<Item = String>) -> CoreArgs {
    CoreArgs {
        offline: argv.into_iter().any(|arg| arg == "--offline"),
        ..CoreArgs::default()
    }
}

/// Build the core and start serving it. Blocks until the library is open and
/// the config is known, because the tracing subscriber and the window are
/// built from that config before the UI exists.
pub fn start(args: CoreArgs) -> Result<&'static Core, String> {
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kopuz-core".into())
        .spawn(move || {
            daemon::boot::prepare_thread();
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("core runtime: {error}")));
                    return;
                }
            };
            runtime.block_on(async move {
                let core = match daemon::boot::assemble(&args).await {
                    Ok(core) => core,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                #[cfg(target_os = "android")]
                {
                    let endpoint = match crate::artwork_http::start(core.api.clone()).await {
                        Ok(endpoint) => endpoint,
                        Err(error) => {
                            let _ = ready_tx.send(Err(format!("artwork server: {error}")));
                            return;
                        }
                    };
                    if let Err(error) = utils::set_artwork_endpoint(endpoint) {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                }
                #[cfg(target_os = "linux")]
                {
                    let endpoint = match crate::video_http::start().await {
                        Ok(endpoint) => endpoint,
                        Err(error) => {
                            let _ = ready_tx.send(Err(format!("video server: {error}")));
                            return;
                        }
                    };
                    if let Err(error) = utils::set_video_endpoint(endpoint) {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                }
                let _ = CORE.set(core);
                let _ = ready_tx.send(Ok(()));
                let Some(core) = CORE.get() else {
                    return;
                };
                serve_socket(core).await;
            });

            runtime.shutdown_background();
        })
        .map_err(|error| format!("could not start the core thread: {error}"))?;

    match ready_rx.recv() {
        Ok(Ok(())) => CORE.get().ok_or_else(|| "core vanished".to_string()),
        Ok(Err(error)) => Err(error),
        Err(_) => Err("the core thread stopped before it was ready".into()),
    }
}

/// Serve the core to other frontends for the life of the process. Our own
/// UI is already attached in-process, so a socket that cannot be bound or
/// stops serving costs external clients, not playback: the core stays up on
/// this runtime either way.
#[cfg(not(target_os = "android"))]
async fn serve_socket(core: &Core) {
    match kopuzd::default_socket_path() {
        Some(socket) => match kopuzd::listen(core, &socket) {
            Ok(serving) => {
                *bound_socket() = Some(socket);
                let served = serving.await;
                // The listener unlinked the path as it dropped, and a later instance may have bound it since.
                bound_socket().take();
                if let Err(error) = served {
                    tracing::warn!(%error, "the daemon socket stopped serving");
                }
            }
            Err(error) => tracing::warn!(%error, "the daemon socket is unavailable"),
        },
        None => tracing::warn!("no address for the daemon socket"),
    }
    std::future::pending::<()>().await;
}

/// The socket this process bound, while it still serves it.
#[cfg(not(target_os = "android"))]
fn bound_socket() -> std::sync::MutexGuard<'static, Option<std::path::PathBuf>> {
    static BOUND: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);
    BOUND
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The process exits without unwinding the core thread, so the listener never drops and its path has to be unlinked here; a named pipe leaves no file.
#[cfg(not(target_os = "android"))]
fn release_socket() {
    if let Some(path) = bound_socket().take()
        && cfg!(unix)
        && let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(%error, path = %path.display(), "could not remove the daemon socket");
    }
}

/// Android hosts the core without serving it: `kopuzd` is a desktop-only
/// dependency, and nothing on the device attaches to the app's core the way
/// `kopuzctl` does. The thread still parks here, because the runtime it owns
/// has to outlive every service the core spawned on it.
#[cfg(target_os = "android")]
async fn serve_socket(_core: &Core) {
    std::future::pending::<()>().await;
}

pub fn core() -> Option<&'static Core> {
    CORE.get()
}

/// The handle every read and every mutation goes through. Panics before the
/// core is up, which cannot happen: main builds it before the window exists.
pub fn api() -> std::sync::Arc<dyn api::KopuzApi> {
    CORE.get()
        .map(|core| core.api.clone() as std::sync::Arc<dyn api::KopuzApi>)
        .expect("the core is started before anything asks it for something")
}

/// Flush what the core owns; the volume and mute go first, since a close right after a slider drag lands inside their debounce.
pub fn shutdown() {
    let Some(core) = CORE.get() else {
        return;
    };
    let session = core.session.clone();
    let database = core.db.clone();
    // Dioxus forbids block_on inside its runtime, so the flush gets a thread.
    let flush = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            daemon::boot::flush_playback_state(&session, &database).await;
            session.persist_now().await;
        });
    });
    let _ = flush.join();
    #[cfg(not(target_os = "android"))]
    release_socket();
}

#[cfg(all(test, unix))]
mod tests {
    use super::{bound_socket, release_socket};

    #[test]
    fn a_clean_exit_unlinks_only_the_socket_this_process_bound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ours = dir.path().join("kopuzd.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&ours).expect("bind");
        *bound_socket() = Some(ours.clone());
        release_socket();
        assert!(!ours.exists(), "the bound socket is gone");

        let theirs = dir.path().join("other.sock");
        let _other = std::os::unix::net::UnixListener::bind(&theirs).expect("bind");
        release_socket();
        assert!(theirs.exists(), "a second release touches nothing");
    }
}

#[cfg(test)]
mod args_tests {
    use super::core_args;

    fn args(argv: &[&str]) -> daemon::boot::CoreArgs {
        core_args(argv.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn offline_starts_the_hosted_core_offline() {
        assert!(!args(&[]).offline);
        assert!(args(&["--offline"]).offline);
        assert!(args(&["--something", "--offline"]).offline);
        assert_eq!(args(&["--offline"]).db_path, None);
    }
}
