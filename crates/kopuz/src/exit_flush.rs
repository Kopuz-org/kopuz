//! What the app has to write on exit, stashed for the SIGINT path, which runs on the ctrlc thread where no signal can be read.

use std::sync::Mutex;
use std::time::Duration;

static STASHED: Mutex<Option<config::AppConfig>> = Mutex::new(None);
static STASHED_PREFS: Mutex<Option<Vec<api::PrefEntry>>> = Mutex::new(None);

/// How long an exiting process waits for each write; a window that will not close is worse than a lost last-moment change.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Stash an eligibility-checked config snapshot (guards: `initial_load_done
/// && config_loaded_ok`).
pub fn stash_config(config: config::AppConfig) {
    if let Ok(mut stashed) = STASHED.lock() {
        *stashed = Some(config);
    }
}

/// Stash the UI preferences not yet sent, for the same exit write.
pub fn stash_prefs(entries: Vec<api::PrefEntry>) {
    if let Ok(mut stashed) = STASHED_PREFS.lock() {
        *stashed = Some(entries);
    }
}

/// Persist the config and unsent preferences on a fresh thread: `block_on` panics inside dioxus's tokio context.
pub fn persist_on_fresh_thread(
    config: Option<config::AppConfig>,
    prefs: Option<Vec<api::PrefEntry>>,
) {
    let prefs = prefs.filter(|prefs| !prefs.is_empty());
    if config.is_none() && prefs.is_none() {
        return;
    }
    let api = crate::backend::api();
    let _ = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            if let Some(config) = config {
                let write = tokio::time::timeout(FLUSH_TIMEOUT, api.set_config(config));
                match write.await {
                    Ok(Err(error)) => tracing::warn!(%error, "config flush on exit failed"),
                    Err(_) => tracing::warn!("config flush on exit timed out"),
                    Ok(Ok(_)) => {}
                }
            }
            if let Some(prefs) = prefs {
                let frontend = hooks::ui_prefs::FRONTEND.to_string();
                let write =
                    tokio::time::timeout(FLUSH_TIMEOUT, api.set_frontend_prefs(frontend, prefs));
                match write.await {
                    Ok(Err(error)) => tracing::warn!(%error, "UI preferences flush on exit failed"),
                    Err(_) => tracing::warn!("UI preferences flush on exit timed out"),
                    Ok(Ok(())) => {}
                }
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
    let prefs = match STASHED_PREFS.lock() {
        Ok(stashed) => stashed.clone(),
        Err(_) => None,
    };
    if crate::backend::core().is_some() {
        persist_on_fresh_thread(config, prefs);
    }
    crate::backend::shutdown();
}
