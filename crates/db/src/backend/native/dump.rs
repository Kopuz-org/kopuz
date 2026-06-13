//! Reconstruct the in-memory playlist store/queue from the DB (issue #347). The
//! legacy `PersistedQueueState::load` can't parse the new `Track` shape, so the
//! runtime loads these from the DB (the converted source of truth) instead of
//! re-reading the old JSON.

use std::collections::HashMap;
use std::path::PathBuf;

use reader::PlaylistStore;
use reader::models::{JellyfinPlaylist, Playlist, PlaylistFolder};
use sqlx::SqlitePool;

use crate::{DbError, QueueSnapshot};

/// The active server id from the config blob (`None` ⇒ local).
pub(super) async fn active_server_id(pool: &SqlitePool) -> Option<String> {
    let blob: Option<String> = sqlx::query_scalar!("SELECT json FROM app_config WHERE id = 1")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    blob.as_deref()
        .and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok())
        .and_then(|v| {
            v.get("active_server_id")
                .and_then(|x| x.as_str())
                .map(String::from)
        })
}

type ArtistImages = (
    HashMap<String, String>,
    HashMap<String, PathBuf>,
    HashMap<String, PathBuf>,
);

pub async fn artist_images(pool: &SqlitePool) -> Result<ArtistImages, DbError> {
    let rows = sqlx::query!("SELECT artist_norm, kind, image_ref FROM artist_images")
        .fetch_all(pool)
        .await?;
    let mut server = HashMap::new();
    let mut local = HashMap::new();
    let mut custom = HashMap::new();
    for r in rows {
        match r.kind.as_str() {
            "local" => {
                local.insert(r.artist_norm, PathBuf::from(r.image_ref));
            }
            "custom" => {
                custom.insert(r.artist_norm, PathBuf::from(r.image_ref));
            }
            _ => {
                server.insert(r.artist_norm, r.image_ref);
            }
        }
    }
    Ok((server, local, custom))
}

pub async fn load_playlists(
    pool: &SqlitePool,
    active_server: Option<&str>,
) -> Result<PlaylistStore, DbError> {
    // Scoped to local + the ACTIVE server: the in-memory store only ever
    // represents the active server, and the write side is scoped the same
    // way — so other servers' rows are invisible to this load. The caller
    // passes the IN-MEMORY active id — the persisted blob lags a server
    // switch by the debounced config save, which would briefly show the
    // previous server's playlists under the new identity.
    let active = match active_server {
        Some(s) => s.to_owned(),
        None => active_server_id(pool).await.unwrap_or_default(),
    };
    let rows = sqlx::query!(
        "SELECT rowid_pk, source, source_pl_id, name, cover_path, image_tag \
         FROM playlists WHERE source = 'local' OR source = ?1 ORDER BY position",
        active
    )
    .fetch_all(pool)
    .await?;

    let mut playlists = Vec::new();
    let mut jellyfin_playlists = Vec::new();
    for r in rows {
        let tracks: Vec<String> = sqlx::query_scalar!(
            "SELECT track_ref FROM playlist_tracks WHERE playlist_pk = ?1 ORDER BY position",
            r.rowid_pk
        )
        .fetch_all(pool)
        .await?;
        if r.source == "local" {
            playlists.push(Playlist {
                id: r.source_pl_id,
                name: r.name,
                tracks: tracks.into_iter().map(PathBuf::from).collect(),
                cover_path: r.cover_path.map(PathBuf::from),
            });
        } else {
            jellyfin_playlists.push(JellyfinPlaylist {
                id: r.source_pl_id,
                name: r.name,
                tracks,
                image_tag: r.image_tag,
                cover_path: r.cover_path.map(PathBuf::from),
            });
        }
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

    Ok(PlaylistStore {
        playlists,
        jellyfin_playlists,
        folders,
    })
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
