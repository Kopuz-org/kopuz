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
    let rows = sqlx::query!("SELECT source, artist_key, kind, image_ref FROM artist_images")
        .fetch_all(pool)
        .await?;
    let mut overrides = HashMap::new();
    let mut photos: HashMap<(String, String), ArtistImageRef> = HashMap::new();
    for r in rows {
        let identity = (r.source, r.artist_key);
        match r.kind.as_str() {
            "custom" => {
                overrides.insert(identity, PathBuf::from(r.image_ref));
            }
            // Server photo wins over a local one for the same artist: `insert`
            // always overwrites, `or_insert` for local never clobbers a server.
            "server" => {
                photos.insert(identity, ArtistImageRef::Remote(r.image_ref));
            }
            _ => {
                photos
                    .entry(identity)
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
    let rows = sqlx::query!(
        "SELECT rowid_pk as \"rowid_pk!\", source_pl_id, name, cover_path, image_tag \
         FROM playlists WHERE source = ?1 ORDER BY position",
        src
    )
    .fetch_all(pool)
    .await?;

    // One query for every playlist's tracks (not one per playlist), grouped
    // by playlist afterwards. Order by playlist first so each group's tracks
    // arrive contiguous and position-sorted.
    let track_rows = sqlx::query!(
        "SELECT pt.playlist_pk, pt.track_ref \
         FROM playlist_tracks pt \
         JOIN playlists p ON p.rowid_pk = pt.playlist_pk \
         WHERE p.source = ?1 \
         ORDER BY pt.playlist_pk, pt.position",
        src
    )
    .fetch_all(pool)
    .await?;
    let mut tracks_by_pk: HashMap<i64, Vec<String>> = HashMap::new();
    for t in track_rows {
        tracks_by_pk
            .entry(t.playlist_pk)
            .or_default()
            .push(t.track_ref);
    }

    let playlists = rows
        .into_iter()
        .map(|r| Playlist {
            id: r.source_pl_id,
            name: r.name,
            tracks: tracks_by_pk.remove(&r.rowid_pk).unwrap_or_default(),
            image_tag: r.image_tag,
            cover_path: r.cover_path.map(PathBuf::from),
        })
        .collect();

    let folder_rows = sqlx::query!("SELECT id, name FROM folders")
        .fetch_all(pool)
        .await?;
    let member_rows = sqlx::query!(
        "SELECT folder_id, playlist_ref FROM folder_playlists ORDER BY folder_id, position"
    )
    .fetch_all(pool)
    .await?;
    let mut members_by_folder: HashMap<String, Vec<String>> = HashMap::new();
    for m in member_rows {
        members_by_folder
            .entry(m.folder_id)
            .or_default()
            .push(m.playlist_ref);
    }
    let folders = folder_rows
        .into_iter()
        .map(|f| PlaylistFolder {
            playlist_ids: members_by_folder.remove(&f.id).unwrap_or_default(),
            id: f.id,
            name: f.name,
        })
        .collect();

    Ok(PlaylistStore { playlists, folders })
}

/// The queue `source` was left with; empty when it never had one.
pub async fn load_queue(
    pool: &SqlitePool,
    source: &crate::Source,
) -> Result<QueueSnapshot, DbError> {
    let src = source.as_str();
    // One read snapshot, so a save landing between the reads cannot pair old rows with a new index.
    let mut tx = pool.begin().await?;
    let row = sqlx::query!(
        "SELECT version, current_queue_index, progress_secs, shuffle_enabled, radio_cursor \
         FROM queue_state WHERE source = ?1",
        src
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        return Ok(QueueSnapshot::default());
    };
    let rows = sqlx::query_as!(
        super::rows::QueueTrackRow,
        "SELECT position, track_key, service, source_album_id, title, artist, album, duration, \
           khz, bitrate, track_number, disc_number, cover_path, mb_release_id, mb_recording_id, \
           mb_track_id, playlist_item_id, rg_track_gain, rg_track_peak, rg_album_gain, rg_album_peak \
         FROM queue_tracks WHERE source = ?1 ORDER BY position",
        src
    )
    .fetch_all(&mut *tx)
    .await?;
    let credits = sqlx::query_as!(
        super::rows::QueueCreditRow,
        "SELECT queue_position, name, source_artist_id, source, artist_key \
         FROM queue_credits WHERE queue_source = ?1 ORDER BY queue_position, position",
        src
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut by_row: std::collections::HashMap<i64, Vec<reader::ArtistCredit>> =
        std::collections::HashMap::new();
    for credit in credits {
        by_row
            .entry(credit.queue_position)
            .or_default()
            .push(credit.into());
    }
    let saved: Vec<reader::Track> = rows
        .into_iter()
        .map(|row| {
            let credits = by_row.remove(&row.position).unwrap_or_default();
            row.into_track(credits)
        })
        .collect();
    let shuffle_order = sqlx::query_scalar!(
        "SELECT position FROM queue_shuffle WHERE source = ?1 ORDER BY step",
        src
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(QueueSnapshot {
        version: row.version.clamp(0, u8::MAX as i64) as u8,
        queue: super::queries::refresh_from_library(pool, source, saved).await?,
        current_queue_index: row.current_queue_index.max(0) as usize,
        progress_secs: row.progress_secs.max(0) as u64,
        shuffle_order: shuffle_order
            .into_iter()
            .map(|at| at.max(0) as usize)
            .collect(),
        shuffle_enabled: row.shuffle_enabled != 0,
        radio_cursor: row.radio_cursor,
    })
}

/// What the lyrics cache holds for `cache_key`, if anything.
pub async fn cached_lyrics(
    pool: &SqlitePool,
    cache_key: &str,
) -> Result<Option<crate::CachedLyrics>, DbError> {
    use utils::lyrics::{LyricChunk, LyricLine, Lyrics};

    let Some(stored) = sqlx::query!(
        "SELECT kind, plain_text, fetched_at FROM lyrics WHERE cache_key = ?1",
        cache_key
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let cached = match (stored.kind.as_str(), stored.plain_text) {
        ("plain", Some(text)) => crate::CachedLyrics::Found(Lyrics::Plain(text)),
        ("synced", _) => {
            let lines = sqlx::query!(
                "SELECT position, start_time, end_time, text, parent_line, background, opposite_turn \
                   FROM lyric_lines WHERE cache_key = ?1 ORDER BY position",
                cache_key
            )
            .fetch_all(pool)
            .await?;
            let chunks = sqlx::query!(
                "SELECT line, start_time, text FROM lyric_chunks WHERE cache_key = ?1 \
                  ORDER BY line, position",
                cache_key
            )
            .fetch_all(pool)
            .await?;
            let mut by_line: std::collections::HashMap<i64, Vec<LyricChunk>> =
                std::collections::HashMap::new();
            for chunk in chunks {
                by_line.entry(chunk.line).or_default().push(LyricChunk {
                    start_time: chunk.start_time,
                    text: chunk.text,
                });
            }
            let lines = lines
                .into_iter()
                .map(|line| LyricLine {
                    start_time: line.start_time,
                    end_time: line.end_time,
                    text: line.text,
                    chunks: by_line.remove(&line.position).unwrap_or_default(),
                    parent_line_index: line.parent_line.map(|index| index.max(0) as usize),
                    background: line.background != 0,
                    opposite_turn: line.opposite_turn != 0,
                })
                .collect();
            crate::CachedLyrics::Found(Lyrics::Synced(lines))
        }
        _ => crate::CachedLyrics::Missing {
            at: stored.fetched_at,
        },
    };
    Ok(Some(cached))
}
