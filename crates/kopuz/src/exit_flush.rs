//! Last-known persistable config for exit paths that cannot read UI signals.
//!
//! The wry close handler peeks signals directly, but the SIGINT handler runs
//! on the ctrlc thread where no signal access exists. The `App` effects stash
//! the config here as it changes; the SIGINT path persists whatever was last
//! stashed, so a Ctrl+C no longer loses up to a debounce window of it.
//!
//! The queue is not here: the core owns its store and flushes on the way out.

use std::sync::Mutex;
use std::time::Duration;

static STASHED: Mutex<Option<config::AppConfig>> = Mutex::new(None);

/// How long an exiting process will wait for the config write. Past this the
/// user is closing a window that will not close, which is worse than losing a
/// settings change made in the last moment.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Stash an eligibility-checked config snapshot (guards: `initial_load_done
/// && config_loaded_ok`).
pub fn stash_config(config: config::AppConfig) {
    if let Ok(mut stashed) = STASHED.lock() {
        *stashed = Some(config);
    }
}

/// Persist a config through the core on a fresh OS thread with its own runtime
/// and join it. A fresh thread is required from both exit paths: the main
/// thread sits inside dioxus's tokio context where `block_on` panics, and the
/// ctrlc thread should not host a runtime of unknown stack depth.
pub fn persist_on_fresh_thread(config: Option<config::AppConfig>) {
    let Some(config) = config else {
        return;
    };
    let api = crate::backend::api();
    let _ = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            let write = tokio::time::timeout(FLUSH_TIMEOUT, api.set_config(config));
            match write.await {
                Ok(Err(error)) => tracing::warn!(%error, "config flush on exit failed"),
                Err(_) => tracing::warn!("config flush on exit timed out"),
                Ok(Ok(_)) => {}
            }
        });
    })
    .join();
}

/// SIGINT path: persist whatever the app last stashed, then flush the core. A
/// no-op before the startup loads complete, so an early Ctrl+C cannot wipe
/// saved state.
pub fn flush_stashed_blocking() {
    let config = match STASHED.lock() {
        Ok(stashed) => stashed.clone(),
        Err(_) => return,
    };
    if crate::backend::core().is_some() {
        persist_on_fresh_thread(config);
    }
    crate::backend::shutdown();
}
