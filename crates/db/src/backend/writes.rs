//! Batch upserts + scan reconcile (issue #347, step 7). Each call commits as one
//! transaction so a streaming scan/sync batch lands atomically — a mid-scan quit
//! keeps everything written so far (no torn whole-file write).

use reader::models::{Album, Track};
use sqlx::SqlitePool;

use crate::{DbError, QueueSnapshot, Source};

pub(crate) fn service_str(s: config::MusicService) -> &'static str {
    match s {
        config::MusicService::Jellyfin => "Jellyfin",
        config::MusicService::Subsonic => "Subsonic",
        config::MusicService::Custom => "Custom",
        config::MusicService::YtMusic => "YtMusic",
        config::MusicService::SoundCloud => "SoundCloud",
        config::MusicService::AppleMusic => "AppleMusic",
        config::MusicService::Spotify => "Spotify",
        config::MusicService::Nextcloud => "Nextcloud",
    }
}

/// Insert or refresh tracks.
#[tracing::instrument(skip_all, fields(count = tracks.len(), source = %source.as_str()))]
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
        // A remote row is dated when first seen; a local one takes its file's time from the scan.
        let pk = sqlx::query_scalar!(
            "INSERT INTO tracks \
               (source, track_key, path, service, source_album_id, title, artist, album, duration, \
                khz, bitrate, track_number, disc_number, cover_path, added_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                     CASE WHEN ?4 IS NULL THEN 0 ELSE unixepoch() END) \
             ON CONFLICT(source, track_key) DO UPDATE SET \
               path=?3, service=?4, \
               source_album_id=CASE WHEN ?5 != '' THEN ?5 ELSE tracks.source_album_id END, \
               title=?6, artist=?7, album=?8, duration=?9, \
               khz=?10, bitrate=?11, track_number=?12, disc_number=?13, cover_path=?14 \
             RETURNING rowid_pk AS \"pk!: i64\"",
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
            t.cover
        )
        .fetch_one(&mut *tx)
        .await?;
        write_track_children(&mut tx, pk, t).await?;
        ensure_album(&mut tx, src, t).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The album a track names gets a row, never overwriting one a library sync or scan wrote.
async fn ensure_album(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    src: &str,
    t: &Track,
) -> Result<(), DbError> {
    if t.album_id.is_empty() {
        return Ok(());
    }
    let title = match t.album.is_empty() {
        true => "Singles",
        false => t.album.as_str(),
    };
    let artist_id = t
        .credits
        .iter()
        .find(|credit| credit.name.trim() == t.artist.trim())
        .or(t.credits.first())
        .and_then(|credit| credit.id.clone());
    sqlx::query!(
        "INSERT INTO albums (source, source_album_id, title, artist, cover_path, artist_id, derived) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1) ON CONFLICT(source, source_album_id) DO NOTHING",
        src,
        t.album_id,
        title,
        t.artist,
        t.cover,
        artist_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The credits a row stores: the source's own, else its names as unlinked credits.
fn stored_credits(t: &Track) -> Vec<reader::ArtistCredit> {
    if !t.credits.is_empty() {
        return t.credits.clone();
    }
    let names: Vec<&str> = match t.artists.is_empty() {
        true => vec![t.artist.as_str()],
        false => t.artists.iter().map(String::as_str).collect(),
    };
    names
        .into_iter()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(reader::ArtistCredit::unlinked)
        .collect()
}

/// Write a track's credits and MusicBrainz ids beside its row.
pub(crate) async fn write_track_children(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    pk: i64,
    t: &Track,
) -> Result<(), DbError> {
    // Some paths only name a row's artists, so bare names never replace a list that carries an id.
    let linked: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM track_credits WHERE track_pk = ?1 AND artist_id IS NOT NULL",
        pk
    )
    .fetch_one(&mut **tx)
    .await?;
    if !t.credits.is_empty() || linked == 0 {
        sqlx::query!("DELETE FROM track_credits WHERE track_pk = ?1", pk)
            .execute(&mut **tx)
            .await?;
        for (position, credit) in stored_credits(t).iter().enumerate() {
            let position = position as i64;
            let name = credit.name.trim();
            sqlx::query!(
                "INSERT INTO track_credits (track_pk, position, name, artist_id) \
                 VALUES (?1, ?2, ?3, ?4)",
                pk,
                position,
                name,
                credit.id
            )
            .execute(&mut **tx)
            .await?;
        }
    }

    let (release, recording, track) = (
        &t.musicbrainz_release_id,
        &t.musicbrainz_recording_id,
        &t.musicbrainz_track_id,
    );
    if release.is_none() && recording.is_none() && track.is_none() {
        sqlx::query!("DELETE FROM track_musicbrainz WHERE track_pk = ?1", pk)
            .execute(&mut **tx)
            .await?;
    } else {
        sqlx::query!(
            "INSERT INTO track_musicbrainz (track_pk, release_id, recording_id, track_id) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(track_pk) DO UPDATE SET \
               release_id = ?2, recording_id = ?3, track_id = ?4",
            pk,
            release,
            recording,
            track
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Stamp each `(track_key, unix_secs)` as the track's date added. Only rows that
/// have never been stamped are touched, so the value survives everything that
/// rewrites a file afterwards: a tag edit bumping the mtime must not make a
/// track look freshly added.
#[tracing::instrument(skip_all, fields(count = stamps.len(), source = %source.as_str()))]
pub async fn stamp_added_at(
    pool: &SqlitePool,
    source: &Source,
    stamps: &[(String, i64)],
) -> Result<(), DbError> {
    if stamps.is_empty() {
        return Ok(());
    }
    let src = source.as_str();
    let mut tx = pool.begin().await?;
    for (track_key, added_at) in stamps {
        sqlx::query!(
            "UPDATE tracks SET added_at = ?3 \
             WHERE source = ?1 AND track_key = ?2 AND added_at = 0",
            src,
            track_key,
            added_at
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[tracing::instrument(skip_all, fields(count = albums.len(), source = %source.as_str()))]
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
        let cover = a
            .cover_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        sqlx::query!(
            "INSERT INTO albums (source, source_album_id, title, artist, genre, year, cover_path, manual_cover, artist_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(source, source_album_id) DO UPDATE SET \
               title=?3, artist=?4, genre=?5, year=?6, \
               cover_path=COALESCE(?7, albums.cover_path), \
               manual_cover=MAX(?8, albums.manual_cover), \
               artist_id=COALESCE(?9, albums.artist_id), derived=0",
            src,
            a.id,
            a.title,
            a.artist,
            a.genre,
            year,
            cover,
            manual,
            a.artist_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[tracing::instrument(skip_all, fields(server_id = %server_id, on))]
pub async fn set_favorite(
    pool: &SqlitePool,
    server_id: &str,
    ref_: &str,
    on: bool,
) -> Result<(), DbError> {
    if on {
        // A fresh like sorts to the top (rank below the current minimum); a
        // re-like of a pending-unlike tombstone just resurrects it as a
        // pending-like and keeps its existing rank/position.
        sqlx::query!(
            "INSERT INTO favorites (server_id, ref, dirty, rank) \
             SELECT ?1, ?2, 1, COALESCE(MIN(rank), 0) - 1 FROM favorites WHERE server_id = ?1 \
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

#[tracing::instrument(skip_all, fields(server_id = %server_id))]
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

#[tracing::instrument(skip_all, fields(server_id = %server_id, count = refs.len()))]
pub async fn replace_favorites_clean(
    pool: &SqlitePool,
    server_id: &str,
    refs: &[String],
) -> Result<(), DbError> {
    let keep_json = serde_json::to_string(refs)?;
    let mut tx = pool.begin().await?;
    // Drop clean rows the remote no longer has (dirty rows survive — not pushed yet).
    sqlx::query!(
        "DELETE FROM favorites WHERE server_id = ?1 AND dirty = 0 \
         AND ref NOT IN (SELECT value FROM json_each(?2))",
        server_id,
        keep_json
    )
    .execute(&mut *tx)
    .await?;
    // Add the remote set in one statement. `json_each.key` is the array index,
    // which is also the remote newest-first rank. Updating only rank preserves a
    // dirty row's pending local toggle.
    sqlx::query(
        "INSERT INTO favorites (server_id, ref, dirty, rank) \
         SELECT ?1, CAST(value AS TEXT), 0, CAST(key AS INTEGER) FROM json_each(?2) WHERE true \
         ON CONFLICT(server_id, ref) DO UPDATE SET rank = excluded.rank",
    )
    .bind(server_id)
    .bind(&keep_json)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Upsert one page of a streaming favorites sync: each ref becomes a clean row
/// at `start_rank + offset` (preserving the remote's newest-first order) and is
/// stamped with `epoch` so the end-of-stream sweep can tell what's still live.
/// Existing rows are updated in place (rank + epoch); a dirty row keeps its flag
/// (pending local toggle), it just gets re-stamped so the sweep won't drop it.
#[tracing::instrument(skip_all, fields(server_id = %server_id, count = refs.len()))]
pub async fn upsert_favorites_page(
    pool: &SqlitePool,
    server_id: &str,
    refs: &[String],
    start_rank: i64,
    epoch: i64,
) -> Result<(), DbError> {
    let refs_json = serde_json::to_string(refs)?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO favorites (server_id, ref, dirty, rank, epoch) \
         SELECT ?1, CAST(value AS TEXT), 0, ?3 + CAST(key AS INTEGER), ?4 \
         FROM json_each(?2) WHERE true \
         ON CONFLICT(server_id, ref) DO UPDATE SET rank = excluded.rank, epoch = excluded.epoch",
    )
    .bind(server_id)
    .bind(refs_json)
    .bind(start_rank)
    .bind(epoch)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// End-of-stream sweep: drop clean rows NOT re-stamped with the current sync's
/// `epoch` — i.e. liked items the remote no longer has. Dirty rows (pending
/// local likes/unlikes) survive, exactly as in `replace_favorites_clean`.
#[tracing::instrument(skip_all, fields(server_id = %server_id))]
pub async fn sweep_favorites(
    pool: &SqlitePool,
    server_id: &str,
    epoch: i64,
) -> Result<(), DbError> {
    sqlx::query!(
        "DELETE FROM favorites WHERE server_id = ?1 AND dirty = 0 AND epoch != ?2",
        server_id,
        epoch
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[tracing::instrument(skip_all, fields(count = keys.len(), source = %source.as_str()))]
pub async fn delete_tracks(
    pool: &SqlitePool,
    source: &Source,
    keys: &[String],
) -> Result<u64, DbError> {
    if keys.is_empty() {
        return Ok(0);
    }
    let keys_json = serde_json::to_string(keys)?;
    let src = source.as_str();
    let res = sqlx::query!(
        "DELETE FROM tracks WHERE source = ?1 \
         AND track_key IN (SELECT value FROM json_each(?2))",
        src,
        keys_json
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Drop a source's tracks/albums not present in the keep-sets (post-sync
/// reconcile — the replacement for clear-and-repopulate). One transaction, so
/// a failure can't leave tracks pruned but their albums behind.
#[tracing::instrument(skip_all, fields(source = %source.as_str()))]
pub async fn prune_source(
    pool: &SqlitePool,
    source: &Source,
    keep_track_keys: &[String],
    keep_album_ids: &[String],
) -> Result<(), DbError> {
    let src = source.as_str();
    let keep_tracks = serde_json::to_string(keep_track_keys)?;
    let keep_albums = serde_json::to_string(keep_album_ids)?;
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM tracks WHERE source = ?1 \
         AND track_key NOT IN (SELECT value FROM json_each(?2)) \
         AND track_key NOT IN (SELECT pt.track_ref FROM playlist_tracks pt \
             JOIN playlists p ON p.rowid_pk = pt.playlist_pk WHERE p.source = ?1) \
         AND track_key NOT IN (SELECT ref FROM favorites WHERE server_id = ?1)",
        src,
        keep_tracks
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "DELETE FROM albums WHERE source = ?1 \
         AND source_album_id NOT IN (SELECT value FROM json_each(?2)) \
         AND source_album_id NOT IN (SELECT source_album_id FROM tracks WHERE source = ?1)",
        src,
        keep_albums
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[tracing::instrument(skip_all, fields(album_id = %album_id, source = %source.as_str()))]
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

#[tracing::instrument(skip_all, fields(artist_norm = %artist_norm, kind = %kind))]
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

#[tracing::instrument(skip_all, fields(album_id = %album_id, source = %source.as_str()))]
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

#[tracing::instrument(skip_all, fields(album_id = %album_id, source = %source.as_str()))]
pub async fn update_album_cover_if_not_manual(
    pool: &SqlitePool,
    source: &Source,
    album_id: &str,
    cover_path: &str,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE albums SET cover_path = ?3 \
         WHERE source = ?1 AND source_album_id = ?2 AND manual_cover = 0",
    )
    .bind(source.as_str())
    .bind(album_id)
    .bind(cover_path)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() != 0)
}

#[tracing::instrument(skip_all, fields(pl_id = %pl_id, source = %source.as_str()))]
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

#[tracing::instrument(skip_all, fields(pl_id = %pl_id, source = %source.as_str()))]
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

/// Resolve a playlist's `rowid_pk`, creating the playlist row (name = id) if it
/// doesn't exist yet. Shared by the membership writers below.
async fn resolve_or_create_pk(
    conn: &mut sqlx::SqliteConnection,
    src: &str,
    pl_id: &str,
) -> Result<i64, DbError> {
    let existing: Option<i64> = sqlx::query_scalar!(
        "SELECT rowid_pk FROM playlists WHERE source = ?1 AND source_pl_id = ?2",
        src,
        pl_id
    )
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    if let Some(pk) = existing {
        return Ok(pk);
    }
    let res = sqlx::query!(
        "INSERT INTO playlists (source, source_pl_id, name) VALUES (?1, ?2, ?2)",
        src,
        pl_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(res.last_insert_rowid())
}

/// Replace ONE playlist's membership (creating the playlist row if absent) —
/// playlist-scoped, never the whole store. For reorders and full rebuilds.
#[tracing::instrument(skip_all, fields(count = entries.len(), source = %source.as_str(), pl_id))]
pub async fn set_playlist_tracks(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
    entries: &[reader::PlaylistEntry],
) -> Result<(), DbError> {
    let src = source.as_str();
    let mut tx = pool.begin().await?;
    let pk = resolve_or_create_pk(&mut tx, src, pl_id).await?;
    sqlx::query!("DELETE FROM playlist_tracks WHERE playlist_pk = ?1", pk)
        .execute(&mut *tx)
        .await?;
    for (pos, entry) in entries.iter().enumerate() {
        let pos = pos as i64;
        sqlx::query!(
            "INSERT INTO playlist_tracks (playlist_pk, position, track_ref, item_id) \
             VALUES (?1, ?2, ?3, ?4)",
            pk,
            pos,
            entry.key,
            entry.item_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// One playlist's entries in order, each with the id the source gave it.
pub async fn playlist_entries(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
) -> Result<Vec<reader::PlaylistEntry>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query!(
        "SELECT pt.track_ref, pt.item_id FROM playlist_tracks pt \
         JOIN playlists p ON p.rowid_pk = pt.playlist_pk \
         WHERE p.source = ?1 AND p.source_pl_id = ?2 ORDER BY pt.position",
        src,
        pl_id
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| reader::PlaylistEntry {
            key: row.track_ref,
            item_id: row.item_id,
        })
        .collect())
}

/// Remove the entry at `index` in play order, so a track listed twice loses only that copy.
#[tracing::instrument(skip(pool), fields(source = %source.as_str(), pl_id, index))]
pub async fn remove_playlist_entry(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
    index: usize,
) -> Result<(), DbError> {
    let src = source.as_str();
    let index = index as i64;
    sqlx::query!(
        "DELETE FROM playlist_tracks WHERE rowid = ( \
           SELECT pt.rowid FROM playlist_tracks pt \
           JOIN playlists p ON p.rowid_pk = pt.playlist_pk \
           WHERE p.source = ?1 AND p.source_pl_id = ?2 \
           ORDER BY pt.position LIMIT 1 OFFSET ?3)",
        src,
        pl_id,
        index
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Append `refs` to one playlist (creating it if absent), skipping any ref
/// already present so a track is never duplicated. Existing rows are untouched.
#[tracing::instrument(skip_all, fields(count = refs.len(), source = %source.as_str(), pl_id))]
pub async fn add_playlist_tracks(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
    refs: &[String],
) -> Result<(), DbError> {
    let src = source.as_str();
    let mut tx = super::begin_immediate(pool).await?;
    let pk = resolve_or_create_pk(&mut tx, src, pl_id).await?;
    let mut present: std::collections::HashSet<String> = sqlx::query_scalar!(
        "SELECT track_ref FROM playlist_tracks WHERE playlist_pk = ?1",
        pk
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let max_pos: Option<i64> = sqlx::query_scalar!(
        "SELECT MAX(position) AS \"m?: i64\" FROM playlist_tracks WHERE playlist_pk = ?1",
        pk
    )
    .fetch_one(&mut *tx)
    .await?;
    let mut next = max_pos.map_or(0, |m| m + 1);
    for r in refs {
        if present.insert(r.clone()) {
            sqlx::query!(
                "INSERT INTO playlist_tracks (playlist_pk, position, track_ref) VALUES (?1, ?2, ?3)",
                pk,
                next,
                r
            )
            .execute(&mut *tx)
            .await?;
            next += 1;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Remove every occurrence of each ref from one playlist. No-op if the playlist
/// or a ref is absent. Leaves gaps in `position` — reads are `ORDER BY position`,
/// so the surviving order is unaffected.
#[tracing::instrument(skip_all, fields(count = refs.len(), source = %source.as_str(), pl_id))]
pub async fn remove_playlist_tracks(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
    refs: &[String],
) -> Result<(), DbError> {
    if refs.is_empty() {
        return Ok(());
    }
    let src = source.as_str();
    let mut tx = super::begin_immediate(pool).await?;
    let pk: Option<i64> = sqlx::query_scalar!(
        "SELECT rowid_pk FROM playlists WHERE source = ?1 AND source_pl_id = ?2",
        src,
        pl_id
    )
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    let Some(pk) = pk else {
        return Ok(());
    };
    for r in refs {
        sqlx::query!(
            "DELETE FROM playlist_tracks WHERE playlist_pk = ?1 AND track_ref = ?2",
            pk,
            r
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[tracing::instrument(skip(pool, entries), fields(pl_id = %pl_id, count = entries.len(), start = start_position))]
pub async fn upsert_playlist_tracks_page(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
    entries: &[reader::PlaylistEntry],
    start_position: i64,
    epoch: i64,
) -> Result<(), DbError> {
    let src = source.as_str();
    let mut tx = pool.begin().await?;
    let pk = resolve_or_create_pk(&mut tx, src, pl_id).await?;
    for (i, entry) in entries.iter().enumerate() {
        let pos = start_position + i as i64;
        // Overwrite by position: re-walking in order re-stamps positions 0..N with
        // the current epoch; a now-shorter playlist leaves its tail at the old
        // epoch for the sweep. Position is the entry order, so this also applies
        // reorders in place.
        sqlx::query!(
            "INSERT INTO playlist_tracks (playlist_pk, position, track_ref, epoch, item_id) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(playlist_pk, position) DO UPDATE SET track_ref = excluded.track_ref, \
               epoch = excluded.epoch, item_id = excluded.item_id",
            pk,
            pos,
            entry.key,
            epoch,
            entry.item_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[tracing::instrument(skip(pool), fields(pl_id = %pl_id))]
pub async fn sweep_playlist_tracks(
    pool: &SqlitePool,
    source: &Source,
    pl_id: &str,
    epoch: i64,
) -> Result<(), DbError> {
    let src = source.as_str();
    let pk: Option<i64> = sqlx::query_scalar!(
        "SELECT rowid_pk FROM playlists WHERE source = ?1 AND source_pl_id = ?2",
        src,
        pl_id
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    let Some(pk) = pk else {
        return Ok(());
    };
    sqlx::query!(
        "DELETE FROM playlist_tracks WHERE playlist_pk = ?1 AND epoch != ?2",
        pk,
        epoch
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[tracing::instrument(skip(pool), fields(id = %id))]
pub async fn create_folder(pool: &SqlitePool, id: &str, name: &str) -> Result<(), DbError> {
    sqlx::query!(
        "INSERT INTO folders (id, source, name) VALUES (?1, 'local', ?2) \
         ON CONFLICT(id) DO UPDATE SET name = excluded.name",
        id,
        name
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[tracing::instrument(skip(pool), fields(id = %id))]
pub async fn rename_folder(pool: &SqlitePool, id: &str, name: &str) -> Result<(), DbError> {
    sqlx::query!("UPDATE folders SET name = ?2 WHERE id = ?1", id, name)
        .execute(pool)
        .await?;
    Ok(())
}

#[tracing::instrument(skip(pool), fields(id = %id))]
pub async fn delete_folder(pool: &SqlitePool, id: &str) -> Result<(), DbError> {
    sqlx::query!("DELETE FROM folders WHERE id = ?1", id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Move one playlist into `folder_id`, or out of every folder when `None`.
/// Membership is single-folder, so the playlist's existing rows are cleared
/// first, then one is appended at the end of the target folder.
#[tracing::instrument(skip(pool), fields(playlist_ref = %playlist_ref))]
pub async fn set_playlist_folder(
    pool: &SqlitePool,
    playlist_ref: &str,
    folder_id: Option<&str>,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM folder_playlists WHERE playlist_ref = ?1",
        playlist_ref
    )
    .execute(&mut *tx)
    .await?;
    if let Some(fid) = folder_id {
        sqlx::query!(
            "INSERT OR IGNORE INTO folder_playlists (folder_id, playlist_ref, position) \
             SELECT ?1, ?2, COALESCE(MAX(position) + 1, 0) \
             FROM folder_playlists WHERE folder_id = ?1",
            fid,
            playlist_ref
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// One `json_set`/`json_remove` on the config blob — the downloads hot path
/// must not rewrite the whole config per finished song.
#[tracing::instrument(skip_all, fields(id = %id))]
pub async fn set_offline_track(
    pool: &SqlitePool,
    id: &str,
    path: Option<&str>,
) -> Result<(), DbError> {
    let key = format!("$.offline_tracks.\"{}\"", id.replace('"', ""));
    match path {
        Some(p) => {
            // Upsert so a download finishing before the first config save
            // (fresh DB, no row 1 yet) isn't silently dropped.
            sqlx::query!(
                "INSERT INTO app_config (id, json) VALUES (1, json_set('{}', ?1, ?2)) \
                 ON CONFLICT(id) DO UPDATE SET json = json_set(json, ?1, ?2)",
                key,
                p
            )
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query!(
                "UPDATE app_config SET json = json_remove(json, ?1) WHERE id = 1",
                key
            )
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
        "SELECT value FROM kv WHERE name = ?1 AND kind = ?2",
        cache_key,
        kind
    )
    .fetch_optional(pool)
    .await?)
}

/// Metadata-cache keys of `kind` written within the last `max_age_secs` — e.g.
/// the fresh artist-photo negative results the fetch loop must not re-search.
pub async fn meta_keys_since(
    pool: &SqlitePool,
    kind: &str,
    max_age_secs: i64,
) -> Result<Vec<String>, DbError> {
    Ok(sqlx::query_scalar!(
        "SELECT name FROM kv WHERE kind = ?1 AND updated_at >= (unixepoch() - ?2)",
        kind,
        max_age_secs
    )
    .fetch_all(pool)
    .await?)
}

#[tracing::instrument(skip_all, fields(cache_key = %cache_key, kind = %kind))]
pub async fn meta_put(
    pool: &SqlitePool,
    cache_key: &str,
    kind: &str,
    payload: &str,
) -> Result<(), DbError> {
    sqlx::query!(
        "INSERT INTO kv (name, kind, value) VALUES (?1, ?2, ?3) \
         ON CONFLICT(kind, name) DO UPDATE SET value = ?3, updated_at = unixepoch()",
        cache_key,
        kind,
        payload
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[tracing::instrument(name = "queue.save", skip_all)]
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
