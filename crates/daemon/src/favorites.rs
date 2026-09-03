//! FavoritesService: the optimistic toggle and the background reconciler,
//! ported from `hooks/src/favorites.rs` and `hooks/src/use_sync_task.rs`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use api::{ApiError, ApiEvent, ErrorCode, FavoritesView, JobKind, JobRef, Table};
use server::sync::{SyncError, SyncReason, reconcile_favorites};
use tokio::sync::Notify;

use crate::error::source as source_error;
use crate::jobs::{JobCtx, JobRunner};
use crate::session::SessionHandle;

const NUDGE_DEBOUNCE: Duration = Duration::from_secs(2);
const BACKOFF_CAP_SECS: u64 = 30 * 60;

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
            .map_err(crate::error::source)?;
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
        // Live search results are not in the DB yet; the materializer resolves
        // them from the library's transient cache so hearting them still works
        // (record_favorite then upserts the track like the old direct path).
        let track = match self
            .db
            .tracks_by_keys(&config.active_source, &[key.to_string()])
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?
            .into_iter()
            .next()
        {
            Some(track) => track,
            None => self.session.materialize_track(key.to_string()).await?,
        };

        if source.is_favorite(key).await == favorite {
            return Ok(());
        }
        source
            .record_favorite(&track, favorite)
            .await
            .map_err(crate::error::source)?;
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

    /// Ask the reconciler to push soon (debounced): called after an
    /// in-process favorite mutation by the embedded frontend.
    pub fn nudge_after_mutation(&self) {
        self.mutation_nudge.store(true, Ordering::Relaxed);
        self.nudge.notify_one();
    }

    /// Ask the reconciler to run soon without the after-mutation marker
    /// (the app window regained focus).
    pub fn nudge_activate(&self) {
        self.nudge.notify_one();
    }

    pub fn spawn_sync(self: &Arc<Self>, runner: &JobRunner) -> Result<JobRef, ApiError> {
        let service = self.clone();
        runner.start(JobKind::FavoritesSync, move |ctx| async move {
            ctx.progress("reconciling", None, None, None);
            service.manual_sync(&ctx).await
        })
    }

    async fn manual_sync(&self, ctx: &JobCtx) -> Result<(), ApiError> {
        let config = self.session.config_watch().borrow().clone();
        let Some(source) = server::source::configured_server(self.db.clone(), &config) else {
            return Err(ApiError::invalid_input("no server configured"));
        };
        if source.capabilities().favorites_sync == server::source::FavoritesSync::Paginated {
            return self.sync_paginated(ctx, source.as_ref()).await;
        }
        self.reconcile(SyncReason::Manual).await
    }

    async fn sync_paginated(
        &self,
        ctx: &JobCtx,
        source: &dyn server::source::MediaSource,
    ) -> Result<(), ApiError> {
        match source.validate().await {
            server::source::AuthOutcome::Valid => {}
            server::source::AuthOutcome::Expired => {
                return Err(ApiError::new(
                    ErrorCode::SourceAuthExpired,
                    "credentials expired; sign in again",
                ));
            }
            server::source::AuthOutcome::Unreachable => {
                return Err(ApiError::new(
                    ErrorCode::SourceUnreachable,
                    "server unreachable",
                ));
            }
        }

        let source_id = source.source().clone();
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or_default();
        let mut cursor = None;
        let mut seen = HashSet::new();
        let mut keys = Vec::new();
        let mut keep_albums = Vec::new();
        loop {
            if ctx.cancelled() {
                return Ok(());
            }
            let page = source
                .fetch_favorites_page(cursor)
                .await
                .map_err(source_error)?;
            let next = page.next;
            let tracks: Vec<reader::Track> = page
                .tracks
                .into_iter()
                .filter(|track| {
                    let key = track.id.key().into_owned();
                    !key.is_empty() && seen.insert(key)
                })
                .collect();
            if !tracks.is_empty() {
                let page_keys: Vec<String> = tracks
                    .iter()
                    .map(|track| track.id.key().into_owned())
                    .collect();
                let start = keys.len() as i64;
                keys.extend(page_keys.iter().cloned());
                keep_albums.extend(tracks.iter().map(|track| track.album_id.clone()));
                self.db
                    .upsert_tracks(&source_id, &tracks)
                    .await
                    .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
                self.db
                    .upsert_albums(&source_id, &synthesize_albums(&tracks))
                    .await
                    .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
                self.db
                    .upsert_favorites_page(source_id.as_str(), &page_keys, start, epoch)
                    .await
                    .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
                ctx.progress("fetching favorites", Some(keys.len() as u64), None, None);
                self.bump(Table::Tracks);
                self.bump(Table::Favorites);
            }
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        keep_albums.sort();
        keep_albums.dedup();
        self.db
            .prune_source(&source_id, &keys, &keep_albums)
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
        self.db
            .sweep_favorites(source_id.as_str(), epoch)
            .await
            .map_err(|error| ApiError::internal(format!("database error: {error}")))?;
        self.bump(Table::Tracks);
        self.bump(Table::Albums);
        self.bump(Table::Favorites);
        Ok(())
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

fn synthesize_albums(tracks: &[reader::Track]) -> Vec<reader::Album> {
    let mut by_album: HashMap<String, &reader::Track> = HashMap::new();
    for track in tracks {
        if !track.album_id.is_empty() {
            by_album.entry(track.album_id.clone()).or_insert(track);
        }
    }
    by_album
        .into_iter()
        .map(|(id, track)| reader::Album {
            id,
            title: if track.album.is_empty() {
                "Singles".to_string()
            } else {
                track.album.clone()
            },
            artist: track.artist.clone(),
            genre: String::new(),
            year: 0,
            cover_path: track.cover.as_deref().map(PathBuf::from),
            manual_cover: false,
        })
        .collect()
}
