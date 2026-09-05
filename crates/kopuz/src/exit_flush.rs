//! Exit flushing sourced from the daemon rather than the UI mirror.

use std::sync::{Arc, Mutex};

struct Stashed {
    flush_queue: bool,
    config: Option<config::AppConfig>,
}

static STASHED: Mutex<Stashed> = Mutex::new(Stashed {
    flush_queue: false,
    config: None,
});
static FRONTEND_API: Mutex<Option<Arc<dyn api::KopuzApi>>> = Mutex::new(None);
static QUEUE_PERSISTENCE: Mutex<Option<(daemon::SessionHandle, Arc<dyn daemon::QueueStore>)>> =
    Mutex::new(None);

pub fn install_frontend_api(api: Arc<dyn api::KopuzApi>) {
    if let Ok(mut installed) = FRONTEND_API.lock() {
        *installed = Some(api);
    }
}

pub fn install_queue_persistence(
    session: daemon::SessionHandle,
    store: Arc<dyn daemon::QueueStore>,
) {
    if let Ok(mut installed) = QUEUE_PERSISTENCE.lock() {
        *installed = Some((session, store));
    }
}

pub fn enable_queue_flush() {
    if let Ok(mut stashed) = STASHED.lock() {
        stashed.flush_queue = true;
    }
}

/// Stash an eligibility-checked config snapshot (guards: `initial_load_done
/// && config_loaded_ok`, live volume already injected).
pub fn stash_config(config: config::AppConfig) {
    if let Ok(mut stashed) = STASHED.lock() {
        stashed.config = Some(config);
    }
}

/// Persist the given snapshots on a fresh OS thread with its own runtime and
/// join it. A fresh thread is required from both exit paths: the main thread
/// sits inside dioxus's tokio context where `block_on` panics, and the ctrlc
/// thread should not host a runtime of unknown stack depth.
pub fn persist_on_fresh_thread(flush_queue: bool, config: Option<config::AppConfig>) {
    if !flush_queue && config.is_none() {
        tracing::info!("daemon shutdown has no pending state to flush");
        return;
    }
    let queue_pending = flush_queue;
    let config_pending = config.is_some();
    let frontend_api = FRONTEND_API.lock().ok().and_then(|api| api.clone());
    let queue_persistence = QUEUE_PERSISTENCE
        .lock()
        .ok()
        .and_then(|persistence| persistence.clone());
    tracing::info!(
        queue_pending,
        config_pending,
        "daemon shutdown flush started"
    );
    let worker = std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(error) => {
                tracing::error!(%error, "daemon shutdown flush runtime could not start");
                return;
            }
        };
        rt.block_on(async move {
            if flush_queue {
                match queue_persistence {
                    Some((session, store)) => store.save(session.queue_snapshot()).await,
                    None => tracing::warn!("queue flush on exit has no owned daemon session"),
                }
            }
            if let Some(api) = frontend_api {
                if let Some(cfg) = config {
                    match api.config().await {
                        Ok(current) => {
                            let patch = super::frontend_config_patch(&cfg, &current.config);
                            if patch.as_object().is_some_and(|patch| !patch.is_empty())
                                && let Err(error) = api.patch_config(patch).await
                            {
                                tracing::warn!(%error, "config flush on exit failed");
                            }
                        }
                        Err(error) => tracing::warn!(%error, "config read on exit failed"),
                    }
                }
            } else {
                tracing::warn!("shutdown flush has no daemon API");
            }
        });
    });
    match worker.join() {
        Ok(()) => tracing::info!(
            queue_pending,
            config_pending,
            "daemon shutdown flush finished"
        ),
        Err(_) => tracing::error!("daemon shutdown flush worker panicked"),
    }
}

/// SIGINT path: persist whatever the app last stashed. A no-op before the
/// startup loads complete, so an early Ctrl+C cannot wipe saved state.
pub fn flush_stashed_blocking() {
    let (flush_queue, config) = match STASHED.lock() {
        Ok(stashed) => (stashed.flush_queue, stashed.config.clone()),
        Err(_) => return,
    };
    persist_on_fresh_thread(flush_queue, config);
}
