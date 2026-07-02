//! Reconstruct the in-memory playlist store/queue from the DB (issue #347). The
//! legacy `PersistedQueueState::load` can't parse the new `Track` shape, so the
//! runtime loads these from the DB (the converted source of truth) instead of
//! re-reading the old JSON.

use std::collections::HashMap;
use std::path::PathBuf;

use reader::PlaylistStore;
use reader::models::{ArtistImageRef, Playlist, PlaylistFolder};
use sqlx::SqlitePool;

use crate::{ArtistImages, DbError, QueueSnapshot, Source};

pub async fn artist_images(pool: &SqlitePool) -> Result<ArtistImages, DbError> {
    let rows = sqlx::query!("SELECT artist_norm, kind, image_ref FROM artist_images")
        .fetch_all(pool)
        .await?;
    let mut overrides = HashMap::new();
    let mut photos: HashMap<String, ArtistImageRef> = HashMap::new();
    for r in rows {
        match r.kind.as_str() {
            "custom" => {
                overrides.insert(r.artist_norm, PathBuf::from(r.image_ref));
            }
            // Server photo wins over a local one for the same artist: `insert`
            // always overwrites, `or_insert` for local never clobbers a server.
            "server" => {
                photos.insert(r.artist_norm, ArtistImageRef::Remote(r.image_ref));
            }
            _ => {
                photos
                    .entry(r.artist_norm)
                    .or_insert_with(|| ArtistImageRef::Local(PathBuf::from(r.image_ref)));
            }
        }
    }
    Ok((overrides, photos))
}

pub async fn load_playlists(pool: &SqlitePool, source: &Source) -> Result<PlaylistStore, DbError> {
    // Scoped to the ACTIVE source only: the app is in local OR one server mode
    // at a time, so the in-memory store represents exactly one source — a local
    // and a server playlist that share an id never collide here. The caller
    // passes the IN-MEMORY active source (the persisted blob lags a switch).
    let src = source.as_str();
    // 'LM' (YT Liked Music) is NOT surfaced as a playlist — likes are the
    // favorites view's domain — so it never appears in the playlists grid even
    // if an older sync materialized a row for it.
    let rows = sqlx::query!(
        "SELECT rowid_pk, source_pl_id, name, cover_path, image_tag \
         FROM playlists WHERE source = ?1 AND source_pl_id != 'LM' ORDER BY position",
        src
    )
    .fetch_all(pool)
    .await?;

    let mut playlists = Vec::new();
    for r in rows {
        let tracks: Vec<String> = sqlx::query_scalar!(
            "SELECT track_ref FROM playlist_tracks WHERE playlist_pk = ?1 ORDER BY position",
            r.rowid_pk
        )
        .fetch_all(pool)
        .await?;
        playlists.push(Playlist {
            id: r.source_pl_id,
            name: r.name,
            tracks,
            image_tag: r.image_tag,
            cover_path: r.cover_path.map(PathBuf::from),
        });
    }

    let folder_rows = sqlx::query!("SELECT id, name FROM folders")
        .fetch_all(pool)
        .await?;
    let mut folders = Vec::new();
    for f in folder_rows {
        let playlist_ids: Vec<String> = sqlx::query_scalar!(
            "SELECT playlist_ref FROM folder_playlists WHERE folder_id = ?1 ORDER BY position",
            f.id
        )
        .fetch_all(pool)
        .await?;
        folders.push(PlaylistFolder {
            id: f.id,
            name: f.name,
            playlist_ids,
        });
    }

    Ok(PlaylistStore { playlists, folders })
}

pub async fn load_queue(pool: &SqlitePool) -> Result<QueueSnapshot, DbError> {
    let row = sqlx::query!(
        "SELECT version, queue_json, current_queue_index, progress_secs, \
                shuffle_order_json, shuffle_enabled \
         FROM queue_state WHERE id = 1"
    )
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(QueueSnapshot::default());
    };
    Ok(QueueSnapshot {
        version: row.version.clamp(0, u8::MAX as i64) as u8,
        queue: serde_json::from_str(&row.queue_json).unwrap_or_default(),
        current_queue_index: row.current_queue_index.max(0) as usize,
        progress_secs: row.progress_secs.max(0) as u64,
        shuffle_order: serde_json::from_str(&row.shuffle_order_json).unwrap_or_default(),
        shuffle_enabled: row.shuffle_enabled != 0,
    })
}
