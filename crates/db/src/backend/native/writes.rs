//! Batch upserts + scan reconcile (issue #347, step 7). Each call commits as one
//! transaction so a streaming scan/sync batch lands atomically — a mid-scan quit
//! keeps everything written so far (no torn whole-file write).

use reader::models::{Album, Library, Track};
use reader::{FavoritesStore, PlaylistStore};
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

/// Full sync of the in-memory `Library` to the DB (the persistence side of the
/// reactive save effect — replaces the legacy whole-file `Library::save`).
/// Prunes are scoped to `'local'` + the passed active server; other servers'
/// rows are never touched.
pub async fn save_library(
    pool: &SqlitePool,
    lib: &Library,
    active: Option<&str>,
) -> Result<(), DbError> {
    upsert_tracks(pool, &Source::Local, &lib.tracks).await?;
    upsert_albums(pool, &Source::Local, &lib.albums).await?;
    let local_track_keys: Vec<String> = lib.tracks.iter().map(|t| t.id.key().into_owned()).collect();
    let local_album_keys: Vec<String> = lib.albums.iter().map(|a| a.id.clone()).collect();
    prune_full(pool, "tracks", "local", &local_track_keys).await?;
    prune_full(pool, "albums", "local", &local_album_keys).await?;

    if let Some(id) = active {
        let src = Source::Server(id.to_string());
        upsert_tracks(pool, &src, &lib.jellyfin_tracks).await?;
        upsert_albums(pool, &src, &lib.jellyfin_albums).await?;
        let server_track_keys: Vec<String> = lib
            .jellyfin_tracks
            .iter()
            .map(|t| t.id.key().into_owned())
            .collect();
        let server_album_keys: Vec<String> =
            lib.jellyfin_albums.iter().map(|a| a.id.clone()).collect();
        prune_full(pool, "tracks", id, &server_track_keys).await?;
        prune_full(pool, "albums", id, &server_album_keys).await?;
    }

    let mut tx = pool.begin().await?;
    sqlx::query!("DELETE FROM artist_images").execute(&mut *tx).await?;
    for (artist, img) in &lib.server_artist_images {
        sqlx::query!(
            "INSERT OR IGNORE INTO artist_images (artist_norm, kind, image_ref) VALUES (?1, 'server', ?2)",
            artist,
            img
        )
        .execute(&mut *tx)
        .await?;
    }
    for (artist, img) in &lib.local_artist_images {
        let p = img.to_string_lossy().into_owned();
        sqlx::query!(
            "INSERT OR IGNORE INTO artist_images (artist_norm, kind, image_ref) VALUES (?1, 'local', ?2)",
            artist,
            p
        )
        .execute(&mut *tx)
        .await?;
    }
    for (artist, img) in &lib.custom_artist_images {
        let p = img.to_string_lossy().into_owned();
        sqlx::query!(
            "INSERT OR IGNORE INTO artist_images (artist_norm, kind, image_ref) VALUES (?1, 'custom', ?2)",
            artist,
            p
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    // The YT sync timestamps live in the metadata cache, NOT the config blob —
    // save_config rewrites the blob from AppConfig (which has no such fields),
    // so blob-resident timestamps would be erased by any config save.
    let stamps = serde_json::json!({
        "last_yt_sync_at": lib.last_yt_sync_at,
        "last_yt_playlists_sync_at": lib.last_yt_playlists_sync_at,
    })
    .to_string();
    meta_put(pool, "yt_sync", "timestamps", &stamps).await?;
    Ok(())
}

/// Replace the persisted playlists/folders for `'local'` + the active server
/// with the in-memory store. SCOPED: other servers' playlist rows are never
/// deleted or re-stamped — the store only ever holds the active server's
/// playlists, so a full DELETE would destroy every other server's cache on
/// each save (and a server switch would wipe them all).
pub async fn save_playlists(
    pool: &SqlitePool,
    store: &PlaylistStore,
    active: Option<&str>,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query!("DELETE FROM playlists WHERE source = 'local'")
        .execute(&mut *tx)
        .await?;
    if let Some(id) = active {
        sqlx::query!("DELETE FROM playlists WHERE source = ?1", id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query!("DELETE FROM folders").execute(&mut *tx).await?;

    for (i, p) in store.playlists.iter().enumerate() {
        let pos = i as i64;
        let cover = p.cover_path.as_ref().map(|c| c.to_string_lossy().into_owned());
        let rec = sqlx::query!(
            "INSERT INTO playlists (source, source_pl_id, name, cover_path, position) \
             VALUES ('local', ?1, ?2, ?3, ?4) RETURNING rowid_pk",
            p.id,
            p.name,
            cover,
            pos
        )
        .fetch_one(&mut *tx)
        .await?;
        for (j, tref) in p.tracks.iter().enumerate() {
            let jp = j as i64;
            let s = tref.to_string_lossy().into_owned();
            sqlx::query!(
                "INSERT INTO playlist_tracks (playlist_pk, position, track_ref) VALUES (?1, ?2, ?3)",
                rec.rowid_pk,
                jp,
                s
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    // Server playlists are written only when an active server id exists to
    // attribute them to; otherwise they'd be stamped with a bogus source.
    if let Some(server_src) = active {
        for (i, p) in store.jellyfin_playlists.iter().enumerate() {
            let pos = i as i64;
            let cover = p.cover_path.as_ref().map(|c| c.to_string_lossy().into_owned());
            let rec = sqlx::query!(
                "INSERT INTO playlists (source, source_pl_id, name, cover_path, image_tag, position) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING rowid_pk",
                server_src,
                p.id,
                p.name,
                cover,
                p.image_tag,
                pos
            )
            .fetch_one(&mut *tx)
            .await?;
            for (j, tref) in p.tracks.iter().enumerate() {
                let jp = j as i64;
                sqlx::query!(
                    "INSERT INTO playlist_tracks (playlist_pk, position, track_ref) VALUES (?1, ?2, ?3)",
                    rec.rowid_pk,
                    jp,
                    tref
                )
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    for f in &store.folders {
        sqlx::query!(
            "INSERT OR IGNORE INTO folders (id, source, name) VALUES (?1, 'local', ?2)",
            f.id,
            f.name
        )
        .execute(&mut *tx)
        .await?;
        for (k, pid) in f.playlist_ids.iter().enumerate() {
            let kp = k as i64;
            sqlx::query!(
                "INSERT OR IGNORE INTO folder_playlists (folder_id, playlist_ref, position) \
                 VALUES (?1, ?2, ?3)",
                f.id,
                pid,
                kp
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;
    Ok(())
}

pub async fn save_favorites_store(
    pool: &SqlitePool,
    store: &FavoritesStore,
    active: Option<&str>,
) -> Result<(), DbError> {
    let local: Vec<String> = store
        .local_favorites
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    replace_favorites_clean(pool, "local", &local).await?;
    if let Some(id) = active {
        replace_favorites_clean(pool, id, &store.jellyfin_favorites).await?;
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
