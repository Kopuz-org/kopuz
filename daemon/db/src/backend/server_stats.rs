//! What a source's server reports per track: plays and star ratings. The
//! [`PLAY_COUNTS`] merges the plays with kopuz's own `listen_counts`.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::{DbError, ServerTrackStats, Source};

/// Plays as kopuz shows them, a table expression to read in place of `listen_counts`: its
/// own count or the server's, whichever is higher, never their sum. Not a view, so a
/// migration that rebuilds `listen_counts` needs no knowledge of it.
pub(crate) const PLAY_COUNTS: &str = "(SELECT source, track_key, MAX(count) AS count, \
       MAX(last_played_at) AS last_played_at \
  FROM (SELECT source, track_key, count, last_played_at FROM listen_counts \
        UNION ALL \
        SELECT source, track_key, COALESCE(play_count, 0), last_played_at FROM server_track_stats \
         WHERE play_count IS NOT NULL OR last_played_at IS NOT NULL) \
 GROUP BY source, track_key)";

/// A sync's report replaces the last one whole, so a track the server stopped rating loses its stars.
pub async fn replace(
    pool: &SqlitePool,
    source: &Source,
    stats: &[ServerTrackStats],
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM server_track_stats WHERE source = ?1")
        .bind(source.as_str())
        .execute(&mut *tx)
        .await?;

    for stat in stats {
        let empty =
            stat.play_count.is_none() && stat.last_played_at.is_none() && stat.rating.is_none();
        if empty {
            continue;
        }
        sqlx::query(
            "INSERT OR REPLACE INTO server_track_stats \
             (source, track_key, play_count, last_played_at, rating) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(source.as_str())
        .bind(&stat.track_key)
        .bind(stat.play_count)
        .bind(stat.last_played_at)
        .bind(stat.rating)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn set_rating(
    pool: &SqlitePool,
    source: &Source,
    track_key: &str,
    rating: Option<u8>,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO server_track_stats (source, track_key, rating) VALUES (?1, ?2, ?3) \
         ON CONFLICT(source, track_key) DO UPDATE SET rating = excluded.rating",
    )
    .bind(source.as_str())
    .bind(track_key)
    .bind(rating)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn ratings(pool: &SqlitePool, source: &Source) -> Result<HashMap<String, u8>, DbError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT track_key, rating FROM server_track_stats \
          WHERE source = ?1 AND rating BETWEEN 1 AND 5",
    )
    .bind(source.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(key, rating)| (key, rating as u8))
        .collect())
}
