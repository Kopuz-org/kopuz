//! Batch upserts + scan reconcile (issue #347, step 7). Each call commits as one
//! transaction so a streaming scan/sync batch lands atomically — a mid-scan quit
//! keeps everything written so far (no torn whole-file write).

use reader::models::{Album, Track};
use sqlx::SqlitePool;

use crate::{DbError, QueueSnapshot, Source};

fn service_str(s: config::MusicService) -> &'static str {
    match s {
        config::MusicService::Jellyfin => "Jellyfin",
        config::MusicService::Subsonic => "Subsonic",
        config::MusicService::Custom => "Custom",
        config::MusicService::YtMusic => "YtMusic",
    }
}

pub async fn upsert_tracks(
    pool: &SqlitePool,
    source: &Source,
    tracks: &[Track],
) -> Result<(), DbError> {
    let src = source.as_str();
    let mut tx = pool.begin().await?;
    for t in tracks {
        let track_key = t.id.key().into_owned();
        let path = t.id.local_path().map(|p| p.to_string_lossy().into_owned());
        let service = t.id.service().map(|s| service_str(s).to_string());
        let duration = t.duration as i64;
        let khz = t.khz as i64;
        let bitrate = t.bitrate as i64;
        let track_number = t.track_number.map(|n| n as i64);
        let disc_number = t.disc_number.map(|n| n as i64);
        let artists_json = serde_json::to_string(&t.artists)?;
        sqlx::query!(
            "INSERT INTO tracks \
               (source, track_key, path, service, source_album_id, title, artist, album, duration, \
                khz, bitrate, track_number, disc_number, mb_release_id, mb_recording_id, mb_track_id, \
                playlist_item_id, artists_json, cover_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19) \
             ON CONFLICT(source, track_key) DO UPDATE SET \
               path=?3, service=?4, source_album_id=?5, title=?6, artist=?7, album=?8, duration=?9, \
               khz=?10, bitrate=?11, track_number=?12, disc_number=?13, mb_release_id=?14, \
               mb_recording_id=?15, mb_track_id=?16, playlist_item_id=?17, artists_json=?18, cover_path=?19",
            src,
            track_key,
            path,
            service,
            t.album_id,
            t.title,
            t.artist,
            t.album,
            duration,
            khz,
            bitrate,
            track_number,
            disc_number,
            t.musicbrainz_release_id,
            t.musicbrainz_recording_id,
            t.musicbrainz_track_id,
            t.playlist_item_id,
            artists_json,
            t.cover
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn upsert_albums(
    pool: &SqlitePool,
    source: &Source,
    albums: &[Album],
) -> Result<(), DbError> {
    let src = source.as_str();
    let mut tx = pool.begin().await?;
    for a in albums {
        let year = a.year as i64;
        let manual = a.manual_cover as i64;
        let cover = a.cover_path.as_ref().map(|p| p.to_string_lossy().into_owned());
        sqlx::query!(
            "INSERT INTO albums (source, source_album_id, title, artist, genre, year, cover_path, manual_cover) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(source, source_album_id) DO UPDATE SET \
               title=?3, artist=?4, genre=?5, year=?6, \
               cover_path=COALESCE(?7, albums.cover_path), \
               manual_cover=MAX(?8, albums.manual_cover)",
            src,
            a.id,
            a.title,
            a.artist,
            a.genre,
            year,
            cover,
            manual
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn set_favorite(
    pool: &SqlitePool,
    server_id: &str,
    ref_: &str,
    on: bool,
) -> Result<(), DbError> {
    if on {
        // A re-like of a pending-unlike tombstone resurrects it as pending-like.
        sqlx::query!(
            "INSERT INTO favorites (server_id, ref, dirty) VALUES (?1, ?2, 1) \
             ON CONFLICT(server_id, ref) DO UPDATE SET dirty = 1",
            server_id,
            ref_
        )
        .execute(pool)
        .await?;
    } else {
        let mut tx = pool.begin().await?;
        // A never-pushed like just disappears; a synced (clean) row becomes a
        // pending-unlike tombstone so the removal survives until pushed.
        sqlx::query!(
            "DELETE FROM favorites WHERE server_id = ?1 AND ref = ?2 AND dirty = 1",
            server_id,
            ref_
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE favorites SET dirty = 2 WHERE server_id = ?1 AND ref = ?2 AND dirty = 0",
            server_id,
            ref_
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

pub async fn dirty_favorites(pool: &SqlitePool, server_id: &str) -> Result<Vec<String>, DbError> {
    Ok(sqlx::query_scalar!(
        "SELECT ref FROM favorites WHERE server_id = ?1 AND dirty = 1",
        server_id
    )
    .fetch_all(pool)
    .await?)
}

pub async fn dirty_unlikes(pool: &SqlitePool, server_id: &str) -> Result<Vec<String>, DbError> {
    Ok(sqlx::query_scalar!(
        "SELECT ref FROM favorites WHERE server_id = ?1 AND dirty = 2",
        server_id
    )
    .fetch_all(pool)
    .await?)
}

pub async fn clear_favorite_dirty(
    pool: &SqlitePool,
    server_id: &str,
    ref_: &str,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM favorites WHERE server_id = ?1 AND ref = ?2 AND dirty = 2",
        server_id,
        ref_
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE favorites SET dirty = 0 WHERE server_id = ?1 AND ref = ?2 AND dirty = 1",
        server_id,
        ref_
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn replace_favorites_clean(
    pool: &SqlitePool,
    server_id: &str,
    refs: &[String],
) -> Result<(), DbError> {
    let keep_json = serde_json::to_string(refs)?;
    let mut tx = pool.begin().await?;
    // Drop clean rows the remote no longer has (dirty rows survive — not pushed yet).
    sqlx::query(
        "DELETE FROM favorites WHERE server_id = ?1 AND dirty = 0 \
         AND ref NOT IN (SELECT value FROM json_each(?2))",
    )
    .bind(server_id)
    .bind(&keep_json)
    .execute(&mut *tx)
    .await?;
    // Add the remote set as clean rows (leave a dirty row's flag intact).
    for r in refs {
        sqlx::query!(
            "INSERT INTO favorites (server_id, ref, dirty) VALUES (?1, ?2, 0) \
             ON CONFLICT(server_id, ref) DO NOTHING",
            server_id,
            r
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn prune_local_tracks(
    pool: &SqlitePool,
    root: &str,
    keep: &[String],
) -> Result<u64, DbError> {
    let keep_json = serde_json::to_string(keep)?;
    let res = sqlx::query(
        "DELETE FROM tracks WHERE source = 'local' AND path LIKE ?1 \
         AND track_key NOT IN (SELECT value FROM json_each(?2))",
    )
    .bind(format!("{root}%"))
    .bind(keep_json)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

async fn prune_full(pool: &SqlitePool, table_key: &str, source: &str, keep: &[String]) -> Result<(), DbError> {
    let keep_json = serde_json::to_string(keep)?;
    let col = if table_key == "albums" { "source_album_id" } else { "track_key" };
    let table = if table_key == "albums" { "albums" } else { "tracks" };
    let sql = format!(
        "DELETE FROM {table} WHERE source = ?1 AND {col} NOT IN (SELECT value FROM json_each(?2))"
    );
    sqlx::query(&sql)
        .bind(source)
        .bind(keep_json)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_tracks(
    pool: &SqlitePool,
    source: &Source,
    keys: &[String],
) -> Result<u64, DbError> {
    if keys.is_empty() {
        return Ok(0);
    }
    let keys_json = serde_json::to_string(keys)?;
    let res = sqlx::query(
        "DELETE FROM tracks WHERE source = ?1 \
         AND track_key IN (SELECT value FROM json_each(?2))",
    )
    .bind(source.as_str())
    .bind(keys_json)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Drop a source's tracks/albums not present in the keep-sets (post-sync
/// reconcile — the replacement for clear-and-repopulate).
pub async fn prune_source(
    pool: &SqlitePool,
    source: &Source,
    keep_track_keys: &[String],
    keep_album_ids: &[String],
) -> Result<(), DbError> {
    prune_full(pool, "tracks", source.as_str(), keep_track_keys).await?;
    prune_full(pool, "albums", source.as_str(), keep_album_ids).await?;
    Ok(())
}

pub async fn delete_album(
    pool: &SqlitePool,
    source: &Source,
    album_id: &str,
) -> Result<(), DbError> {
    let src = source.as_str();
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM tracks WHERE source = ?1 AND source_album_id = ?2",
        src,
        album_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM albums WHERE source = ?1 AND source_album_id = ?2",
        src,
        album_id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_artist_image(
    pool: &SqlitePool,
    artist_norm: &str,
    kind: &str,
    image_ref: Option<&str>,
) -> Result<(), DbError> {
    match image_ref {
        Some(r) => {
            sqlx::query!(
                "INSERT INTO artist_images (artist_norm, kind, image_ref) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(artist_norm, kind) DO UPDATE SET image_ref = ?3",
                artist_norm,
                kind,
                r
            )
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query!(
                "DELETE FROM artist_images WHERE artist_norm = ?1 AND kind = ?2",
                artist_norm,
                kind
            )
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

pub async fn update_album_cover(
    pool: &SqlitePool,
    source: &Source,
    album_id: &str,
    cover_path: Option<&str>,
    manual: bool,
) -> Result<(), DbError> {
    let src = source.as_str();
    let m = manual as i64;
    sqlx::query!(
        "UPDATE albums SET cover_path = ?3, manual_cover = ?4 \
         WHERE source = ?1 AND source_album_id = ?2",
        src,
        album_id,
        cover_path,
        m
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_playlist_meta(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
    name: &str,
    cover_path: Option<&str>,
    image_tag: Option<&str>,
) -> Result<(), DbError> {
    let src = source.as_str();
    sqlx::query!(
        "INSERT INTO playlists (source, source_pl_id, name, cover_path, image_tag) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(source, source_pl_id) DO UPDATE SET name=?3, cover_path=?4, image_tag=?5",
        src,
        pl_id,
        name,
        cover_path,
        image_tag
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_playlist(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
) -> Result<(), DbError> {
    let src = source.as_str();
    sqlx::query!(
        "DELETE FROM playlists WHERE source = ?1 AND source_pl_id = ?2",
        src,
        pl_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Replace ONE playlist's membership (creating the playlist row if absent) —
/// playlist-scoped, never the whole store.
pub async fn set_playlist_tracks(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
    refs: &[String],
) -> Result<(), DbError> {
    let src = source.as_str();
    let mut tx = pool.begin().await?;
    let existing: Option<i64> = sqlx::query_scalar!(
        "SELECT rowid_pk FROM playlists WHERE source = ?1 AND source_pl_id = ?2",
        src,
        pl_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    let pk: i64 = match existing {
        Some(pk) => pk,
        None => {
            let res = sqlx::query!(
                "INSERT INTO playlists (source, source_pl_id, name) VALUES (?1, ?2, ?2)",
                src,
                pl_id
            )
            .execute(&mut *tx)
            .await?;
            res.last_insert_rowid()
        }
    };
    sqlx::query!("DELETE FROM playlist_tracks WHERE playlist_pk = ?1", pk)
        .execute(&mut *tx)
        .await?;
    for (pos, r) in refs.iter().enumerate() {
        let pos = pos as i64;
        sqlx::query!(
            "INSERT INTO playlist_tracks (playlist_pk, position, track_ref) VALUES (?1, ?2, ?3)",
            pk,
            pos,
            r
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn set_folders(
    pool: &SqlitePool,
    folders: &[reader::models::PlaylistFolder],
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query!("DELETE FROM folders").execute(&mut *tx).await?;
    for f in folders {
        sqlx::query!(
            "INSERT INTO folders (id, source, name) VALUES (?1, 'local', ?2)",
            f.id,
            f.name
        )
        .execute(&mut *tx)
        .await?;
        for (pos, pid) in f.playlist_ids.iter().enumerate() {
            let pos = pos as i64;
            sqlx::query!(
                "INSERT OR IGNORE INTO folder_playlists (folder_id, playlist_ref, position) \
                 VALUES (?1, ?2, ?3)",
                f.id,
                pid,
                pos
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// One `json_set`/`json_remove` on the config blob — the downloads hot path
/// must not rewrite the whole config per finished song.
pub async fn set_offline_track(
    pool: &SqlitePool,
    id: &str,
    path: Option<&str>,
) -> Result<(), DbError> {
    let key = format!("$.offline_tracks.\"{}\"", id.replace('"', ""));
    match path {
        Some(p) => {
            sqlx::query("UPDATE app_config SET json = json_set(json, ?1, ?2) WHERE id = 1")
                .bind(key)
                .bind(p)
                .execute(pool)
                .await?;
        }
        None => {
            sqlx::query("UPDATE app_config SET json = json_remove(json, ?1) WHERE id = 1")
                .bind(key)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

pub async fn meta_get(
    pool: &SqlitePool,
    cache_key: &str,
    kind: &str,
) -> Result<Option<String>, DbError> {
    Ok(sqlx::query_scalar!(
        "SELECT payload FROM metadata_cache WHERE cache_key = ?1 AND kind = ?2",
        cache_key,
        kind
    )
    .fetch_optional(pool)
    .await?
    .flatten())
}

pub async fn meta_put(
    pool: &SqlitePool,
    cache_key: &str,
    kind: &str,
    payload: &str,
) -> Result<(), DbError> {
    sqlx::query!(
        "INSERT INTO metadata_cache (cache_key, kind, payload) VALUES (?1, ?2, ?3) \
         ON CONFLICT(cache_key, kind) DO UPDATE SET payload = ?3, fetched_at = unixepoch()",
        cache_key,
        kind,
        payload
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn save_queue(pool: &SqlitePool, snap: &QueueSnapshot) -> Result<(), DbError> {
    let queue_json = serde_json::to_string(&snap.queue)?;
    let shuffle_json = serde_json::to_string(&snap.shuffle_order)?;
    let version = snap.version as i64;
    let cqi = snap.current_queue_index as i64;
    let prog = snap.progress_secs as i64;
    let shuffle_on = snap.shuffle_enabled as i64;
    sqlx::query!(
        "INSERT INTO queue_state \
           (id, version, queue_json, current_queue_index, progress_secs, shuffle_order_json, shuffle_enabled) \
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(id) DO UPDATE SET version=?1, queue_json=?2, current_queue_index=?3, \
           progress_secs=?4, shuffle_order_json=?5, shuffle_enabled=?6",
        version,
        queue_json,
        cqi,
        prog,
        shuffle_json,
        shuffle_on
    )
    .execute(pool)
    .await?;
    Ok(())
}
