//! Last-known persistable state for exit paths that cannot read UI signals.
//!
//! The wry close handler peeks signals directly, but the SIGINT handler runs
//! on the ctrlc thread where no signal access exists. The `App` effects stash
//! eligible snapshots here as they change; the SIGINT path persists whatever
//! was last stashed, so a Ctrl+C no longer loses up to a debounce window of
//! queue and config state.

use std::sync::Mutex;

struct Stashed {
    queue: Option<db::QueueSnapshot>,
    config: Option<config::AppConfig>,
}

static STASHED: Mutex<Stashed> = Mutex::new(Stashed {
    queue: None,
    config: None,
});

/// Stash an eligibility-checked queue snapshot. Callers must apply the same
/// guards as the debounced saver (`initial_load_done && queue_loaded_ok`);
/// an empty-but-eligible queue is stashed as the default snapshot so a
/// cleared queue persists as empty instead of resurrecting.
pub fn stash_queue(snapshot: db::QueueSnapshot) {
    if let Ok(mut stashed) = STASHED.lock() {
        stashed.queue = Some(snapshot);
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
pub fn persist_on_fresh_thread(
    db: db::Db,
    queue: Option<db::QueueSnapshot>,
    config: Option<config::AppConfig>,
) {
    if queue.is_none() && config.is_none() {
        tracing::info!("daemon shutdown has no pending state to flush");
        return;
    }
    let queue_pending = queue.is_some();
    let config_pending = config.is_some();
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
            if let Some(snap) = queue
                && let Err(e) = db.save_queue(&snap).await
            {
                tracing::warn!(error = %e, "queue flush on exit failed");
            }
            if let Some(cfg) = config
                && let Err(e) = db.save_config(&cfg).await
            {
                tracing::warn!(error = %e, "config flush on exit failed");
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
    let (queue, config) = match STASHED.lock() {
        Ok(stashed) => (stashed.queue.clone(), stashed.config.clone()),
        Err(_) => return,
    };
    let Some(db) = crate::app_db::DB_HANDLE.get() else {
        return;
    };
    persist_on_fresh_thread(db.clone(), queue, config);
}
