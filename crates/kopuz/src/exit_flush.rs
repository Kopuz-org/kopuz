//! Last-known persistable config for exit paths that cannot read UI signals.
//!
//! The wry close handler peeks signals directly, but the SIGINT handler runs
//! on the ctrlc thread where no signal access exists. The `App` effects stash
//! the config here as it changes; the SIGINT path persists whatever was last
//! stashed, so a Ctrl+C no longer loses up to a debounce window of it.
//!
//! The queue is not here: the core owns its store and flushes on the way out.

use std::sync::Mutex;

static STASHED: Mutex<Option<config::AppConfig>> = Mutex::new(None);

/// Stash an eligibility-checked config snapshot (guards: `initial_load_done
/// && config_loaded_ok`).
pub fn stash_config(config: config::AppConfig) {
    if let Ok(mut stashed) = STASHED.lock() {
        *stashed = Some(config);
    }
}

/// Persist the given config on a fresh OS thread with its own runtime and join
/// it. A fresh thread is required from both exit paths: the main thread sits
/// inside dioxus's tokio context where `block_on` panics, and the ctrlc thread
/// should not host a runtime of unknown stack depth.
pub fn persist_on_fresh_thread(db: db::Db, config: Option<config::AppConfig>) {
    let Some(config) = config else {
        return;
    };
    let _ = std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        rt.block_on(async move {
            if let Err(e) = db.save_config(&config).await {
                tracing::warn!(error = %e, "config flush on exit failed");
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
    if let Some(core) = crate::backend::core() {
        persist_on_fresh_thread(core.db.clone(), config);
    }
    crate::backend::shutdown();
}
