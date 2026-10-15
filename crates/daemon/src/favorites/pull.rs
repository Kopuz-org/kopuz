//! Importing the favorites a server already holds.
//!
//! Distinct from the reconcile in the parent module, which only pushes local
//! likes and unlikes: this walks what the remote has and materializes it, so
//! a fresh sign-in shows the user's library rather than an empty page.
//!
//! Which shape the walk takes is the source's capability, not its identity.
//! `Instant` sources answer with the whole set at once; `Paginated` ones (YT
//! Music) hand back a cursor at a time, repeat tracks across page boundaries,
//! and need an epoch sweep to notice what was unliked while we were away.

use std::collections::HashSet;

use api::{ApiError, JobKind, Table};
use reader::models::Track;
use server::source::{ActiveSource, FavoritesSync};

use crate::jobs::JobCtx;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

impl super::FavoritesService {
    /// Whether a pull would tell us anything we do not already know. Skipping
    /// is per-shape: a paginated import runs once, an instant one re-runs
    /// after fifteen minutes.
    async fn pull_is_stale(&self, source: &ActiveSource, server_id: &str) -> bool {
        match source.capabilities().favorites_sync {
            FavoritesSync::Paginated => {
                let stamped = crate::auto_sync::last_synced(
                    &self.db,
                    JobKind::FavoritesSync,
                    source.source(),
                )
                .await
                .is_some();

                let held = self.db.favorites(server_id).await.unwrap_or_default().len();
                let dirty = self
                    .db
                    .dirty_favorites(server_id)
                    .await
                    .unwrap_or_default()
                    .len();
                !(stamped || held > dirty)
            }
            FavoritesSync::Instant => {
                let last: u64 = self
                    .db
                    .meta_get("fav_pull", server_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|raw| raw.parse().ok())
                    .unwrap_or(0);
                let now = unix_now();
                !(last <= now && now - last < 15 * 60)
            }
        }
    }

    /// The gated import the reconcile loop runs: no job, no progress, and a
    /// no-op unless something is actually missing.
    pub(super) async fn pull_unattended(&self) -> Result<(), ApiError> {
        self.pull(None, false).await
    }

    /// Import the remote favorites. `force` runs even when a previous import
    /// would have made this one redundant; without a `ctx` the walk is
    /// unattended, reporting no progress and answering to no cancel.
    pub(super) async fn pull(&self, ctx: Option<&JobCtx>, force: bool) -> Result<(), ApiError> {
        let config = self.session.config_watch().borrow().clone();
        let source: ActiveSource =
            std::sync::Arc::from(server::source::active(self.db.clone(), &config));
        if !source.capabilities().sync {
            return Ok(());
        }
        let server_id = source.source().as_str().to_string();
        if !force && !self.pull_is_stale(&source, &server_id).await {
            return Ok(());
        }

        match source.capabilities().favorites_sync {
            FavoritesSync::Instant => {
                if let Some(ctx) = ctx {
                    ctx.progress("importing favorites", None, None, None);
                }
                let ids = source
                    .fetch_favorites()
                    .await
                    .map_err(super::source_error)?;

                source
                    .replace_favorites_clean(&ids)
                    .await
                    .map_err(super::source_error)?;
                source
                    .set_meta("fav_pull", &server_id, &unix_now().to_string())
                    .await
                    .map_err(super::source_error)?;
                self.bump(Table::Favorites);
            }
            FavoritesSync::Paginated => self.pull_paginated(ctx, &source).await?,
        }
        Ok(())
    }

    async fn pull_paginated(
        &self,
        ctx: Option<&JobCtx>,
        source: &ActiveSource,
    ) -> Result<(), ApiError> {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or_default();
        let mut seen: HashSet<String> = HashSet::new();
        let mut ids: Vec<String> = Vec::new();
        let mut keep_albums: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut cursors = HashSet::new();

        loop {
            if ctx.is_some_and(|ctx| ctx.cancelled()) {
                return Ok(());
            }
            let page = source
                .fetch_favorites_page(cursor.clone())
                .await
                .map_err(super::source_error)?;
            let next = page.next.clone();

            let fresh: Vec<Track> = page
                .tracks
                .into_iter()
                .filter(|track| {
                    let key = track.id.key().to_string();
                    !key.is_empty() && seen.insert(key)
                })
                .collect();
            let page_refs: Vec<String> = fresh
                .iter()
                .map(|track| track.id.key().to_string())
                .filter(|key| !key.is_empty())
                .collect();
            let start_rank = ids.len() as i64;
            ids.extend(page_refs.iter().cloned());
            keep_albums.extend(fresh.iter().map(|track| track.album_id.clone()));

            for chunk in fresh.chunks(100) {
                source
                    .upsert_tracks(chunk)
                    .await
                    .map_err(super::source_error)?;
            }
            source
                .upsert_favorites_page(&page_refs, start_rank, epoch)
                .await
                .map_err(super::source_error)?;
            if let Some(ctx) = ctx {
                ctx.progress("importing favorites", Some(ids.len() as u64), None, None);
            }
            self.bump(Table::Tracks);
            self.bump(Table::Favorites);

            match next {
                Some(next) => {
                    if !cursors.insert(next.clone()) {
                        return Err(ApiError::internal("favorites pagination repeated a cursor"));
                    }
                    cursor = Some(next);
                }
                None => break,
            }
        }

        if ctx.is_some_and(|ctx| ctx.cancelled()) {
            return Ok(());
        }
        keep_albums.sort();
        keep_albums.dedup();
        source
            .prune(&ids, &keep_albums)
            .await
            .map_err(super::source_error)?;
        source
            .sweep_favorites(epoch)
            .await
            .map_err(super::source_error)?;
        self.bump(Table::Favorites);
        self.bump(Table::Tracks);
        self.bump(Table::Albums);
        Ok(())
    }
}
