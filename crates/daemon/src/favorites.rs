//! FavoritesService: the optimistic toggle and the background reconciler,
//! ported from `hooks/src/favorites.rs` and `hooks/src/use_sync_task.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use api::{ApiError, ApiEvent, ErrorCode, FavoritesView, JobKind, JobRef, Table};
use server::sync::{SyncError, SyncReason, reconcile_favorites};
use tokio::sync::Notify;

use crate::jobs::JobRunner;
use crate::session::SessionHandle;

const NUDGE_DEBOUNCE: Duration = Duration::from_secs(2);
const BACKOFF_CAP_SECS: u64 = 30 * 60;

fn source_error(error: server::source::SourceError) -> ApiError {
    use server::source::SourceError;
    match &error {
        SourceError::Unsupported(what) => ApiError::unsupported(*what),
        SourceError::Auth => ApiError::new(ErrorCode::SourceAuthExpired, error.to_string()),
        SourceError::Connectivity => ApiError::new(ErrorCode::SourceUnreachable, error.to_string()),
        SourceError::InvalidInput(message) => ApiError::invalid_input(message.clone()),
        SourceError::Backend(message) => ApiError::internal(message.clone()),
    }
}

pub struct FavoritesService {
    db: db::Db,
    session: SessionHandle,
    generation: AtomicU64,
    nudge: Notify,
    mutation_nudge: AtomicBool,
}

impl FavoritesService {
    pub fn new(db: db::Db, session: SessionHandle) -> Arc<Self> {
        Arc::new(Self {
            db,
            session,
            generation: AtomicU64::new(0),
            nudge: Notify::new(),
            mutation_nudge: AtomicBool::new(false),
        })
    }

    fn active_source(&self) -> server::source::ActiveSource {
        let config = self.session.config_watch().borrow().clone();
        Arc::from(server::source::active(self.db.clone(), &config))
    }

    fn bump(&self, table: Table) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.session
            .emit_event(ApiEvent::LibraryInvalidated { table, generation });
    }

    pub async fn list(&self) -> Result<FavoritesView, ApiError> {
        let refs = self
            .active_source()
            .favorites()
            .await
            .map_err(source_error)?;
        Ok(FavoritesView {
            refs,
            generation: self.generation.load(Ordering::Relaxed),
        })
    }

    /// Optimistic set, matching the hooks toggle: local write reflected
    /// immediately, remote push follows; a rejected push reverts the local
    /// state, emits a notice, and surfaces the error to the caller.
    pub async fn set(&self, key: &str, favorite: bool) -> Result<(), ApiError> {
        if key.trim().is_empty() {
            return Err(ApiError::invalid_input("empty favorite key"));
        }
        let source = self.active_source();
        let config = self.session.config_watch().borrow().clone();
        let track = self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("unknown track key"))?;

        if source.is_favorite(key).await == favorite {
            return Ok(());
        }
        source
            .record_favorite(&track, favorite)
            .await
            .map_err(source_error)?;
        self.bump(Table::Favorites);
        self.bump(Table::Tracks);

        if let Err(error) = source.push_favorite(key, favorite).await {
            tracing::warn!(%error, track = %track.id.uid(), "favorite push rejected; reverting");
            let _ = source.record_favorite(&track, !favorite).await;
            self.bump(Table::Favorites);
            self.bump(Table::Tracks);
            self.session.emit_event(ApiEvent::Notice {
                level: api::NoticeLevel::Error,
                code: "favorite_push_rejected".to_string(),
                message: Some(error.to_string()),
            });
            return Err(source_error(error));
        }
        self.mutation_nudge.store(true, Ordering::Relaxed);
        self.nudge.notify_one();
        Ok(())
    }

    pub fn spawn_sync(self: &Arc<Self>, runner: &JobRunner) -> Result<JobRef, ApiError> {
        let service = self.clone();
        runner.start(JobKind::FavoritesSync, move |ctx| async move {
            ctx.progress("reconciling", None, None, None);
            service.reconcile(SyncReason::Manual).await
        })
    }

    async fn reconcile(&self, reason: SyncReason) -> Result<(), ApiError> {
        let config = self.session.config_watch().borrow().clone();
        let Some(source) = server::source::configured_server(self.db.clone(), &config) else {
            return Err(ApiError::invalid_input("no server configured"));
        };
        match reconcile_favorites(source.as_ref(), reason).await {
            Ok(report) => {
                if report.pushed_likes + report.pushed_unlikes > 0 || report.did_pull {
                    self.bump(Table::Favorites);
                }
                Ok(())
            }
            Err(SyncError::Expired) => Err(ApiError::new(
                ErrorCode::SourceAuthExpired,
                "credentials expired; sign in again",
            )),
            Err(SyncError::Unreachable(error)) => {
                Err(ApiError::new(ErrorCode::SourceUnreachable, error))
            }
        }
    }

    /// The background reconcile loop: interval per service (5 min YT, 10 min
    /// others) with exponential backoff while unreachable, debounced nudges
    /// after toggles. Fully parked while no server is configured, per the
    /// resource budget.
    pub fn spawn_reconciler(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut config_rx = service.session.config_watch();
            let mut consecutive_failures: u32 = 0;
            loop {
                let (has_server, base_secs) = {
                    let config = config_rx.borrow();
                    let base: u64 = match config.active_service() {
                        Some(config::MusicService::YtMusic) => 5 * 60,
                        _ => 10 * 60,
                    };
                    (config.server.is_some(), base)
                };

                if !has_server {
                    tokio::select! {
                        _ = service.nudge.notified() => {}
                        changed = config_rx.changed() => {
                            if changed.is_err() {
                                return;
                            }
                        }
                    }
                    continue;
                }

                let backoff = base_secs.saturating_mul(1 << consecutive_failures.min(3));
                let interval = Duration::from_secs(backoff.min(BACKOFF_CAP_SECS));
                let nudged = tokio::select! {
                    _ = service.nudge.notified() => true,
                    _ = tokio::time::sleep(interval) => false,
                };
                if nudged {
                    tokio::time::sleep(NUDGE_DEBOUNCE).await;
                }
                let reason = if nudged {
                    if service.mutation_nudge.swap(false, Ordering::Relaxed) {
                        SyncReason::AfterMutation
                    } else {
                        SyncReason::Activate
                    }
                } else {
                    SyncReason::Interval
                };
                match service.reconcile(reason).await {
                    Ok(()) => consecutive_failures = 0,
                    Err(error) if error.code == ErrorCode::SourceAuthExpired => {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        tracing::warn!("favorites sync: credentials expired");
                    }
                    Err(error) => {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        tracing::debug!(%error, "favorites sync backing off");
                    }
                }
            }
        })
    }
}
