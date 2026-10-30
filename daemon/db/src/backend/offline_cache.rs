//! The offline playback cache's index: runtime queries, so the table needs no prepared-query cache.

use sqlx::SqlitePool;

use crate::{CachedTrackRow, DbError};

type Row = (String, String, String, i64, String, i64, i64);

pub async fn all(pool: &SqlitePool) -> Result<Vec<CachedTrackRow>, DbError> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT track_key, source, path, size, content_type, cached_at, last_played_at \
           FROM offline_cache ORDER BY last_played_at, cached_at, track_key",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(track_key, source, path, size, content_type, cached_at, last_played_at)| {
                CachedTrackRow {
                    track_key,
                    source,
                    path,
                    size: size.max(0) as u64,
                    content_type,
                    cached_at,
                    last_played_at,
                }
            },
        )
        .collect())
}

pub async fn put(pool: &SqlitePool, row: &CachedTrackRow) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO offline_cache (track_key, source, path, size, content_type, cached_at, last_played_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(track_key) DO UPDATE SET source = ?2, path = ?3, size = ?4, content_type = ?5, \
           cached_at = ?6, last_played_at = ?7",
    )
    .bind(&row.track_key)
    .bind(&row.source)
    .bind(&row.path)
    .bind(i64::try_from(row.size).unwrap_or(i64::MAX))
    .bind(&row.content_type)
    .bind(row.cached_at)
    .bind(row.last_played_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn touch(pool: &SqlitePool, track_key: &str, at: i64) -> Result<(), DbError> {
    sqlx::query("UPDATE offline_cache SET last_played_at = ?2 WHERE track_key = ?1")
        .bind(track_key)
        .bind(at)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn remove(pool: &SqlitePool, track_keys: &[String]) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    for key in track_keys {
        sqlx::query("DELETE FROM offline_cache WHERE track_key = ?1")
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
