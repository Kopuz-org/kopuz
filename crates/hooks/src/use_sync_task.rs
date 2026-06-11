//! Background favorites-sync coordinator (issue #347, step 9). Installed once
//! in `App`; runs a reconcile cycle on activation, on a per-service interval
//! (~5 min YT / ~10 min others), and shortly after a favorite toggle (via
//! [`nudge`], debounced). Single-flight by construction (one loop), with
//! exponential backoff on unreachable servers so an offline session doesn't
//! hammer the network.

use std::sync::OnceLock;

use db::Db;
use dioxus::prelude::*;
use server::server_ops::ServerConn;
use server::sync::{SyncError, SyncReason, reconcile_favorites};

use crate::db_reactivity::{Table, use_generations};

static NUDGE: OnceLock<tokio::sync::Notify> = OnceLock::new();

fn nudge_handle() -> &'static tokio::sync::Notify {
    NUDGE.get_or_init(tokio::sync::Notify::new)
}

/// Ask the coordinator to reconcile soon (debounced). Called after a favorite
/// toggle so a pending like reaches the server within seconds, not minutes.
pub fn nudge() {
    nudge_handle().notify_one();
}

pub fn use_sync_task(config: Signal<config::AppConfig>) {
    let db = use_context::<Db>();
    let gens = use_generations();

    use_future(move || {
        let db = db.clone();
        async move {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (&db, &gens);
            }
            #[cfg(not(target_arch = "wasm32"))]
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

                    let (conn, server_id) = {
                        let cfg = config.peek();
                        let Some(conn) = ServerConn::resolve(&cfg) else {
                            continue;
                        };
                        let Some(id) = cfg
                            .active_server_id
                            .clone()
                            .or_else(|| cfg.server.as_ref().and_then(|s| s.id.clone()))
                        else {
                            continue;
                        };
                        (conn, id)
                    };

                    let reason = if nudged {
                        SyncReason::AfterMutation
                    } else {
                        SyncReason::Interval
                    };
                    match reconcile_favorites(&db, &conn, &server_id, reason).await {
                        Ok(report) => {
                            consecutive_failures = 0;
                            if report.pushed_likes + report.pushed_unlikes + report.pulled > 0 {
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
