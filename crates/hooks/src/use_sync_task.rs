//! Background favorites-sync coordinator (issue #347, step 9). Installed once
//! in `App`; runs a reconcile cycle on activation, on a per-service interval
//! (~5 min YT / ~10 min others), and shortly after a favorite toggle (via
//! [`nudge`], debounced). Single-flight by construction (one loop), with
//! exponential backoff on unreachable servers so an offline session doesn't
//! hammer the network.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use db::Db;
use dioxus::prelude::*;
use server::sync::{SyncError, SyncReason, reconcile_favorites};

use crate::db_reactivity::{Table, use_generations};

static NUDGE: OnceLock<tokio::sync::Notify> = OnceLock::new();
static MUTATION_NUDGE: AtomicBool = AtomicBool::new(false);

fn nudge_handle() -> &'static tokio::sync::Notify {
    NUDGE.get_or_init(tokio::sync::Notify::new)
}

/// Ask the coordinator to reconcile soon (debounced). Called after a favorite
/// toggle so a pending like reaches the server within seconds, not minutes.
pub fn nudge() {
    MUTATION_NUDGE.store(true, Ordering::Relaxed);
    nudge_handle().notify_one();
}

/// Startup/activation kick: same debounced cycle, but the reconcile is
/// recorded as `Activate` so a trace doesn't claim a mutation happened.
pub fn nudge_activate() {
    nudge_handle().notify_one();
}

pub fn use_sync_task(config: Signal<config::AppConfig>, db: Db) {
    let gens = use_generations();

    use_future(move || {
        let db = db.clone();
        async move {
            {
                let mut consecutive_failures: u32 = 0;
                loop {
                    let interval = {
                        let base: u64 = match config.peek().active_service() {
                            Some(config::MusicService::YtMusic) => 5 * 60,
                            Some(_) => 10 * 60,
                            None => 10 * 60,
                        };
                        // Exponential backoff while unreachable, capped at 30 min.
                        let backoff = base.saturating_mul(1 << consecutive_failures.min(3));
                        std::time::Duration::from_secs(backoff.min(30 * 60))
                    };

                    let nudged = tokio::select! {
                        _ = nudge_handle().notified() => true,
                        _ = utils::sleep(interval) => false,
                    };
                    if nudged {
                        // Debounce: coalesce a burst of toggles into one cycle.
                        utils::sleep(std::time::Duration::from_secs(2)).await;
                    }

                    let source = {
                        let cfg = config.peek();
                        let Some(source) = server::source::configured_server(db.clone(), &cfg)
                        else {
                            continue;
                        };
                        source
                    };
                    let server_id = source.source().as_str().to_string();

                    let reason = if nudged {
                        if MUTATION_NUDGE.swap(false, Ordering::Relaxed) {
                            SyncReason::AfterMutation
                        } else {
                            SyncReason::Activate
                        }
                    } else {
                        SyncReason::Interval
                    };
                    let outcome = utils::offload(async move {
                        reconcile_favorites(source.as_ref(), reason).await
                    })
                    .await;
                    match outcome {
                        Ok(report) => {
                            consecutive_failures = 0;
                            // did_pull, not pulled: a pull that only removed
                            // rows reports pulled == 0 but still changed the DB.
                            if report.pushed_likes + report.pushed_unlikes > 0 || report.did_pull {
                                gens.bump(Table::Favorites);
                            }
                        }
                        Err(SyncError::Expired) => {
                            consecutive_failures = consecutive_failures.saturating_add(1);
                            tracing::warn!(
                                server = %server_id,
                                "favorites sync: credentials expired — sign in again from settings"
                            );
                        }
                        Err(SyncError::Unreachable(e)) => {
                            consecutive_failures = consecutive_failures.saturating_add(1);
                            tracing::debug!(error = %e, "favorites sync: server unreachable, backing off");
                        }
                    }
                }
            }
        }
    });
}
