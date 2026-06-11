//! Reconstruct the in-memory `Library`/queue from the DB (issue #347). The
//! legacy `Library::load`/`PersistedQueueState::load` can't parse the new `Track`
//! shape, so the runtime loads these from the DB (the converted source of truth)
//! instead of re-reading the old JSON. Transitional: still materializes the whole
//! library into the signal; the windowed hooks replace that in the Sweep.

use std::collections::HashMap;
use std::path::PathBuf;

use reader::models::{Album, JellyfinPlaylist, Library, Playlist, PlaylistFolder, Track};
use reader::FavoritesStore;
use reader::PlaylistStore;
use sqlx::SqlitePool;

use super::queries::TRACK_COLUMNS;
use super::rows::{AlbumRow, TrackRow};
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

async fn all_tracks(pool: &SqlitePool, source: &str) -> Result<Vec<Track>, DbError> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks WHERE source = ?1 \
         ORDER BY artist COLLATE NOCASE, album COLLATE NOCASE, disc_number, track_number"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

async fn all_albums(pool: &SqlitePool, source: &str) -> Result<Vec<Album>, DbError> {
    let rows = sqlx::query_as::<_, AlbumRow>(
        "SELECT source_album_id, title, artist, genre, year, cover_path, manual_cover \
         FROM albums WHERE source = ?1",
    )
    .bind(source)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn load_library(pool: &SqlitePool) -> Result<Library, DbError> {
    // active_server_id + sync timestamps + music dirs come from the config blob.
    let blob: Option<String> = sqlx::query_scalar!("SELECT json FROM app_config WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    let cfg: serde_json::Value = blob
        .as_deref()
        .and_then(|b| serde_json::from_str(b).ok())
        .unwrap_or(serde_json::Value::Null);

    let active_id = cfg.get("active_server_id").and_then(|v| v.as_str());
    // YT sync timestamps: the metadata cache is the durable home (save_library
    // writes there); fall back to blob keys for a DB imported before that
    // change (the importer stamped them into the blob).
    let stamps: Option<serde_json::Value> = super::writes::meta_get(pool, "yt_sync", "timestamps")
        .await
        .ok()
        .flatten()
        .and_then(|p| serde_json::from_str(&p).ok());
    let stamp = |key: &str| {
        stamps
            .as_ref()
            .and_then(|s| s.get(key).and_then(|v| v.as_u64()))
            .or_else(|| cfg.get(key).and_then(|v| v.as_u64()))
    };
    let last_yt_sync_at = stamp("last_yt_sync_at");
    let last_yt_playlists_sync_at = stamp("last_yt_playlists_sync_at");
    let root_paths: Vec<PathBuf> = cfg
        .get("music_directory")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).map(PathBuf::from).collect())
        .unwrap_or_default();

    let tracks = all_tracks(pool, "local").await?;
    let albums = all_albums(pool, "local").await?;
    let (jellyfin_tracks, jellyfin_albums) = match active_id {
        Some(id) => (all_tracks(pool, id).await?, all_albums(pool, id).await?),
        None => (Vec::new(), Vec::new()),
    };

    let (server_artist_images, local_artist_images, custom_artist_images) =
        artist_images(pool).await?;

    Ok(Library {
        root_paths,
        tracks,
        albums,
        jellyfin_tracks,
        jellyfin_albums,
        jellyfin_genres: Vec::new(),
        last_yt_sync_at,
        last_yt_playlists_sync_at,
        server_artist_images,
        local_artist_images,
        custom_artist_images,
    })
}

type ArtistImages = (
    HashMap<String, String>,
    HashMap<String, PathBuf>,
    HashMap<String, PathBuf>,
);

async fn artist_images(pool: &SqlitePool) -> Result<ArtistImages, DbError> {
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

pub async fn load_playlists(pool: &SqlitePool) -> Result<PlaylistStore, DbError> {
    // Scoped to local + the ACTIVE server, mirroring load_library /
    // load_favorites_store: the in-memory store only ever represents the
    // active server, and the save side is scoped the same way — so other
    // servers' rows are invisible to the whole-store round-trip.
    let active = active_server_id(pool).await.unwrap_or_default();
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

/// One server's full cached state — the server-SWITCH load. The in-memory
/// caches are replaced with this (instead of cleared), so switching reuses the
/// DB cache and never destroys another server's rows.
pub async fn load_server_cache(
    pool: &SqlitePool,
    server_id: &str,
) -> Result<
    (
        Vec<Track>,
        Vec<Album>,
        Vec<JellyfinPlaylist>,
        Vec<String>,
    ),
    DbError,
> {
    let tracks = all_tracks(pool, server_id).await?;
    let albums = all_albums(pool, server_id).await?;

    let rows = sqlx::query!(
        "SELECT rowid_pk, source_pl_id, name, cover_path, image_tag \
         FROM playlists WHERE source = ?1 ORDER BY position",
        server_id
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
        playlists.push(JellyfinPlaylist {
            id: r.source_pl_id,
            name: r.name,
            tracks,
            image_tag: r.image_tag,
            cover_path: r.cover_path.map(PathBuf::from),
        });
    }

    let favorites: Vec<String> = sqlx::query_scalar!(
        "SELECT ref FROM favorites WHERE server_id = ?1 AND dirty != 2",
        server_id
    )
    .fetch_all(pool)
    .await?;

    Ok((tracks, albums, playlists, favorites))
}

pub async fn load_favorites_store(pool: &SqlitePool) -> Result<FavoritesStore, DbError> {
    let local: Vec<String> = sqlx::query_scalar!(
        "SELECT ref FROM favorites WHERE server_id = 'local' AND dirty != 2"
    )
    .fetch_all(pool)
    .await?;
    let jellyfin_favorites = match active_server_id(pool).await {
        Some(id) => sqlx::query_scalar::<_, String>(
            "SELECT ref FROM favorites WHERE server_id = ?1 AND dirty != 2",
        )
        .bind(id)
        .fetch_all(pool)
        .await?,
        None => Vec::new(),
    };
    Ok(FavoritesStore {
        local_favorites: local.into_iter().map(PathBuf::from).collect(),
        jellyfin_favorites,
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
