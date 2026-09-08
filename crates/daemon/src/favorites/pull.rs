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

use api::{ApiError, Table};
use reader::models::{Album, Track};
use server::source::{ActiveSource, FavoritesSync};

use crate::jobs::JobCtx;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// A liked track may belong to an album the library has never seen, so the
/// album row is built from the track itself. The first track's thumbnail is
/// the cover: a YT track's `cover` is already a self-contained ref that
/// `CoverRef::parse` reads as-is, so there is no wrapper to add.
fn synthesize_albums(tracks: &[Track]) -> Vec<Album> {
    let mut by_album: std::collections::HashMap<String, &Track> = std::collections::HashMap::new();
    for track in tracks {
        if track.album_id.is_empty() {
            continue;
        }
        by_album.entry(track.album_id.clone()).or_insert(track);
    }
    by_album
        .into_iter()
        .map(|(album_id, track)| Album {
            id: album_id,
            title: if track.album.is_empty() {
                "Singles".to_string()
            } else {
                track.album.clone()
            },
            artist: track.artist.clone(),
            genre: String::new(),
            year: 0,
            cover_path: track.cover.as_deref().map(std::path::PathBuf::from),
            manual_cover: false,
        })
        .collect()
}

impl super::FavoritesService {
    /// Whether a pull would tell us anything we do not already know. Skipping
    /// is per-shape: a paginated import runs once, an instant one re-runs
    /// after fifteen minutes.
    async fn pull_is_stale(&self, source: &ActiveSource, server_id: &str) -> bool {
        match source.capabilities().favorites_sync {
            FavoritesSync::Paginated => {
                let stamped = self
                    .db
                    .meta_get("yt_sync", "timestamps")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                    .and_then(|stamps| stamps.get("last_yt_sync_at").and_then(|at| at.as_u64()))
                    .is_some();
                // Dirty rows do not count as "already imported": a like made
                // locally and never pushed must not suppress the first import.
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
                // A diff in place rather than a clear and re-add, so the list
                // never blinks empty; local dirty rows survive it.
                source
                    .replace_favorites_clean(&ids)
                    .await
                    .map_err(super::source_error)?;
                let _ = source
                    .set_meta("fav_pull", &server_id, &unix_now().to_string())
                    .await;
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
        // One epoch for the whole walk: every page stamps its rows with it and
        // the closing sweep drops whatever was not re-stamped, which is how a
        // remote unlike is noticed.
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or_default();
        let mut seen: HashSet<String> = HashSet::new();
        let mut ids: Vec<String> = Vec::new();
        let mut keep_albums: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut completed = true;

        loop {
            if ctx.is_some_and(|ctx| ctx.cancelled()) {
                completed = false;
                break;
            }
            let page = match source.fetch_favorites_page(cursor.clone()).await {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, "favorites page fetch failed");
                    completed = false;
                    break;
                }
            };
            let next = page.next.clone();
            // YT repeats tracks across page boundaries, so the dedup is ours.
            let fresh: Vec<Track> = page
                .tracks
                .into_iter()
                .filter(|track| {
                    let key = track.id.key().to_string();
                    !key.is_empty() && seen.insert(key)
                })
                .collect();
            // Nothing new after the dedup means the walk is exhausted; going
            // round again would hammer the same continuation forever.
            if fresh.is_empty() {
                break;
            }
            let page_refs: Vec<String> = fresh
                .iter()
                .map(|track| track.id.key().to_string())
                .filter(|key| !key.is_empty())
                .collect();
            let start_rank = ids.len() as i64;
            ids.extend(page_refs.iter().cloned());
            keep_albums.extend(fresh.iter().map(|track| track.album_id.clone()));

            for chunk in fresh.chunks(100) {
                let _ = source.upsert_tracks(chunk).await;
            }
            let _ = source.upsert_albums(&synthesize_albums(&fresh)).await;
            let _ = source
                .upsert_favorites_page(&page_refs, start_rank, epoch)
                .await;
            if let Some(ctx) = ctx {
                ctx.progress("importing favorites", Some(ids.len() as u64), None, None);
            }
            self.bump(Table::Tracks);
            self.bump(Table::Favorites);

            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        if !completed {
            return Ok(());
        }
        keep_albums.sort();
        keep_albums.dedup();
        let _ = source.prune(&ids, &keep_albums).await;
        if source.sweep_favorites(epoch).await.is_ok() {
            self.bump(Table::Favorites);
        }
        let mut stamps: serde_json::Value = self
            .db
            .meta_get("yt_sync", "timestamps")
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        stamps["last_yt_sync_at"] = serde_json::json!(unix_now());
        let _ = source
            .set_meta("yt_sync", "timestamps", &stamps.to_string())
            .await;
        self.bump(Table::Tracks);
        self.bump(Table::Albums);
        Ok(())
    }
}
