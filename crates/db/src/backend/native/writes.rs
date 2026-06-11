//! Batch upserts + scan reconcile (issue #347, step 7). Each call commits as one
//! transaction so a streaming scan/sync batch lands atomically — a mid-scan quit
//! keeps everything written so far (no torn whole-file write).

use reader::models::{Album, Track};
use sqlx::SqlitePool;

use crate::{DbError, Source};

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
        sqlx::query!(
            "INSERT INTO favorites (server_id, ref, dirty) VALUES (?1, ?2, 1) \
             ON CONFLICT(server_id, ref) DO UPDATE SET dirty = 1",
            server_id,
            ref_
        )
        .execute(pool)
        .await?;
    } else {
        sqlx::query!(
            "DELETE FROM favorites WHERE server_id = ?1 AND ref = ?2",
            server_id,
            ref_
        )
        .execute(pool)
        .await?;
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

pub async fn clear_favorite_dirty(
    pool: &SqlitePool,
    server_id: &str,
    ref_: &str,
) -> Result<(), DbError> {
    sqlx::query!(
        "UPDATE favorites SET dirty = 0 WHERE server_id = ?1 AND ref = ?2",
        server_id,
        ref_
    )
    .execute(pool)
    .await?;
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
