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

pub async fn tracks_page(
    pool: &SqlitePool,
    filter: &TrackFilter,
    page: Page,
) -> Result<Vec<Track>, DbError> {
    let has_search = !filter.search.trim().is_empty();
    let mut sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE source = ?1");
    if has_search {
        sql.push_str(" AND (title LIKE ?4 OR artist LIKE ?4 OR album LIKE ?4)");
    }
    sql.push_str(&format!(
        " ORDER BY {} LIMIT ?2 OFFSET ?3",
        order_by(filter.sort)
    ));

    let mut q = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(filter.source.as_str())
        .bind(page.limit as i64)
        .bind(page.offset as i64);
    if has_search {
        q = q.bind(format!("%{}%", filter.search.trim()));
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn tracks_count(pool: &SqlitePool, filter: &TrackFilter) -> Result<u32, DbError> {
    let has_search = !filter.search.trim().is_empty();
    let mut sql = "SELECT COUNT(*) FROM tracks WHERE source = ?1".to_string();
    if has_search {
        sql.push_str(" AND (title LIKE ?2 OR artist LIKE ?2 OR album LIKE ?2)");
    }
    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(filter.source.as_str());
    if has_search {
        q = q.bind(format!("%{}%", filter.search.trim()));
    }
    Ok(q.fetch_one(pool).await?.max(0) as u32)
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
    Ok(
        sqlx::query_scalar!("SELECT ref FROM favorites WHERE server_id = ?1", server_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn is_favorite(
    pool: &SqlitePool,
    server_id: &str,
    ref_: &str,
) -> Result<bool, DbError> {
    let n: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM favorites WHERE server_id = ?1 AND ref = ?2",
        server_id,
        ref_
    )
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}
