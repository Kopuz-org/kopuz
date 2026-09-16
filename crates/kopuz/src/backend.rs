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

/// Build the core and start serving it. Blocks until the library is open and
/// the config is known, because the tracing subscriber and the window are
/// built from that config before the UI exists.
pub fn start() -> Result<&'static Core, String> {
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
                let core = match daemon::boot::assemble(&CoreArgs::default()).await {
                    Ok(core) => core,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = CORE.set(core);
                let _ = ready_tx.send(Ok(()));
                let Some(core) = CORE.get() else {
                    return;
                };
                serve_socket(core).await;
            });
            // The audio engine owns threads that never finish, so dropping the
            // runtime here would block forever on teardown.
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
        Some(socket) => {
            if let Err(error) = kopuzd::listen(core, &socket).await {
                tracing::warn!(%error, "the daemon socket is unavailable");
            }
        }
        None => tracing::warn!("no address for the daemon socket"),
    }
    std::future::pending::<()>().await;
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

/// Flush what the core owns. The socket is released with the listener that
/// bound it, so this only has to make the library durable.
///
/// The volume goes first, because the core debounces it: a close moments
/// after a slider drag would otherwise persist the value from before the
/// drag, whatever the window handed over in its config snapshot.
pub fn shutdown() {
    let Some(core) = CORE.get() else {
        return;
    };
    let session = core.session.clone();
    let config = core.config_service.clone();
    // Dioxus forbids block_on inside its runtime, so the flush gets a thread.
    let flush = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            daemon::boot::flush_volume(&session, &config).await;
            session.persist_now().await;
        });
    });
    let _ = flush.join();
}
