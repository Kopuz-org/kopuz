//! Read queries backing the UI's query hooks (issue #347, step 6).
//!
//! Track listings are sorted + filtered + windowed in SQL (only the visible
//! slice is materialized), so a 20k-row library scrolls without ever holding
//! the whole list in memory. The track query is built at runtime (dynamic
//! `ORDER BY`/`WHERE` from the filter) rather than via the `query!` macro;
//! sort/search clauses are fixed strings, values are always bound.

use reader::models::{Album, Track};
use sqlx::SqlitePool;

use super::rows::{AlbumRow, TrackRow};
use crate::{DbError, Page, Source, TrackFilter, TrackSort};

pub(super) const TRACK_COLUMNS: &str = "source, track_key, service, cover_path, source_album_id, title, \
    artist, album, duration, khz, bitrate, track_number, disc_number, mb_release_id, \
    mb_recording_id, mb_track_id, playlist_item_id, artists_json";

fn order_by(sort: TrackSort) -> &'static str {
    match sort {
        TrackSort::ArtistAlbum => {
            "artist COLLATE NOCASE, album COLLATE NOCASE, disc_number, track_number, title COLLATE NOCASE"
        }
        TrackSort::Title => "title COLLATE NOCASE",
        TrackSort::Artist => "artist COLLATE NOCASE, album COLLATE NOCASE, track_number",
        TrackSort::Album => "album COLLATE NOCASE, disc_number, track_number",
        TrackSort::DateAdded => "rowid_pk DESC",
    }
}

/// WHERE clause + ordered bind values for a filter (after the `source = ?1` bind).
fn filter_clauses(filter: &TrackFilter) -> (String, Vec<String>) {
    let mut sql = String::new();
    let mut binds = Vec::new();
    if !filter.search.trim().is_empty() {
        let n = binds.len() + 2;
        sql.push_str(&format!(
            " AND (title LIKE ?{n} OR artist LIKE ?{n} OR album LIKE ?{n})"
        ));
        binds.push(format!("%{}%", filter.search.trim()));
    }
    if let Some(artist) = &filter.artist {
        let n = binds.len() + 2;
        sql.push_str(&format!(" AND artist = ?{n}"));
        binds.push(artist.clone());
    }
    if let Some(album_id) = &filter.album_id {
        let n = binds.len() + 2;
        sql.push_str(&format!(" AND source_album_id = ?{n}"));
        binds.push(album_id.clone());
    }
    (sql, binds)
}

pub async fn tracks_page(
    pool: &SqlitePool,
    filter: &TrackFilter,
    page: Page,
) -> Result<Vec<Track>, DbError> {
    let (clauses, binds) = filter_clauses(filter);
    let limit_n = binds.len() + 2;
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks WHERE source = ?1{clauses} ORDER BY {} LIMIT ?{limit_n} OFFSET ?{}",
        order_by(filter.sort),
        limit_n + 1,
    );
    let mut q = sqlx::query_as::<_, TrackRow>(&sql).bind(filter.source.as_str());
    for b in &binds {
        q = q.bind(b);
    }
    let rows = q
        .bind(page.limit as i64)
        .bind(page.offset as i64)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn tracks_all(pool: &SqlitePool, filter: &TrackFilter) -> Result<Vec<Track>, DbError> {
    let (clauses, binds) = filter_clauses(filter);
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks WHERE source = ?1{clauses} ORDER BY {}",
        order_by(filter.sort),
    );
    let mut q = sqlx::query_as::<_, TrackRow>(&sql).bind(filter.source.as_str());
    for b in &binds {
        q = q.bind(b);
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn tracks_count(pool: &SqlitePool, filter: &TrackFilter) -> Result<u32, DbError> {
    let (clauses, binds) = filter_clauses(filter);
    let sql = format!("SELECT COUNT(*) FROM tracks WHERE source = ?1{clauses}");
    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(filter.source.as_str());
    for b in &binds {
        q = q.bind(b);
    }
    Ok(q.fetch_one(pool).await?.max(0) as u32)
}

pub async fn tracks_by_keys(
    pool: &SqlitePool,
    source: &Source,
    keys: &[String],
) -> Result<Vec<Track>, DbError> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let keys_json = serde_json::to_string(keys)?;
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks WHERE source = ?1 \
         AND track_key IN (SELECT value FROM json_each(?2))"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source.as_str())
        .bind(keys_json)
        .fetch_all(pool)
        .await?;
    let mut by_key: std::collections::HashMap<String, Track> = rows
        .into_iter()
        .map(Into::into)
        .map(|t: Track| (t.id.key().into_owned(), t))
        .collect();
    Ok(keys.iter().filter_map(|k| by_key.remove(k)).collect())
}

pub async fn artists(pool: &SqlitePool, source: &Source) -> Result<Vec<(String, u32)>, DbError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT artist, COUNT(*) FROM tracks WHERE source = ?1 AND artist != '' \
         GROUP BY artist ORDER BY artist COLLATE NOCASE",
    )
    .bind(source.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(a, n)| (a, n.max(0) as u32))
        .collect())
}

pub async fn genres(pool: &SqlitePool, source: &Source) -> Result<Vec<String>, DbError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT genre FROM albums WHERE source = ?1 AND genre != '' \
         ORDER BY genre COLLATE NOCASE",
    )
    .bind(source.as_str())
    .fetch_all(pool)
    .await?)
}

pub async fn album(
    pool: &SqlitePool,
    source: &Source,
    album_id: &str,
) -> Result<Option<Album>, DbError> {
    let row = sqlx::query_as::<_, AlbumRow>(
        "SELECT source_album_id, title, artist, genre, year, cover_path, manual_cover \
         FROM albums WHERE source = ?1 AND source_album_id = ?2",
    )
    .bind(source.as_str())
    .bind(album_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn albums(pool: &SqlitePool, source: &Source) -> Result<Vec<Album>, DbError> {
    let rows = sqlx::query_as::<_, AlbumRow>(
        "SELECT source_album_id, title, artist, genre, year, cover_path, manual_cover \
         FROM albums WHERE source = ?1 ORDER BY artist COLLATE NOCASE, title COLLATE NOCASE",
    )
    .bind(source.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn favorites(pool: &SqlitePool, server_id: &str) -> Result<Vec<String>, DbError> {
    Ok(sqlx::query_scalar!(
        "SELECT ref FROM favorites WHERE server_id = ?1 AND dirty != 2",
        server_id
    )
    .fetch_all(pool)
    .await?)
}

pub async fn is_favorite(
    pool: &SqlitePool,
    server_id: &str,
    ref_: &str,
) -> Result<bool, DbError> {
    let n: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM favorites WHERE server_id = ?1 AND ref = ?2 AND dirty != 2",
        server_id,
        ref_
    )
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}
