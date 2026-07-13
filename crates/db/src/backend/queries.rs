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

/// Track columns for a `TrackRow`, `t.`-aliased and read via [`TRACKS_FROM`] so a
/// local track's `cover_path` (NULL on the row — the cover is owned by the album)
/// falls back to its album's cover. The track self-resolves its cover with no
/// caller-side album lookup.
///
/// The album fallback is gated to local tracks: their `a.cover_path` is a
/// filesystem path the cover resolver uses directly. A server row's `a.cover_path`
/// is instead a service-encoded ref (e.g. `jellyfin:{albumId}:{tag}`) that the
/// resolver would misread as the *track's* own image tag — so server rows keep
/// their own `t.cover_path` and fall back to the album via `album_id` at resolve
/// time (`server::cover::track`), where the encoding is understood.
const TRACK_COLUMNS: &str = "t.source, t.track_key, t.service, \
    COALESCE(t.cover_path, CASE WHEN t.source = 'local' THEN a.cover_path END) AS cover_path, \
    t.source_album_id, t.title, \
    t.artist, t.album, t.duration, t.khz, t.bitrate, t.track_number, t.disc_number, \
    t.mb_release_id, t.mb_recording_id, t.mb_track_id, t.playlist_item_id, t.artists_json";

/// `FROM tracks t` + the album join that backs the `COALESCE` in [`TRACK_COLUMNS`].
/// LEFT so a track whose album row is missing still returns (cover → NULL → default).
/// `albums` shares column names with `tracks` (`artist`/`title`/`cover_path`/…), so
/// every query using this must `t.`-qualify its WHERE/ORDER BY columns.
const TRACKS_FROM: &str = "FROM tracks t LEFT JOIN albums a \
    ON a.source = t.source AND a.source_album_id = t.source_album_id";

/// SQL for a track row's listen-count key: the local path, or the lowercase
/// legacy `service:item_id` uid the `listen_counts` table is keyed by.
const UID_EXPR: &str = "(CASE WHEN t.source = 'local' THEN t.track_key ELSE \
    (CASE t.service WHEN 'YtMusic' THEN 'ytmusic' WHEN 'Subsonic' THEN 'subsonic' \
     WHEN 'Custom' THEN 'custom' ELSE 'jellyfin' END) || ':' || t.track_key END)";

fn order_by(sort: &TrackSort) -> String {
    match sort {
        TrackSort::ArtistAlbum => {
            "t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.disc_number, t.track_number, t.title COLLATE NOCASE".into()
        }
        TrackSort::Title => "t.title COLLATE NOCASE".into(),
        TrackSort::Artist => "t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.track_number".into(),
        TrackSort::Album => "t.album COLLATE NOCASE, t.disc_number, t.track_number".into(),
        TrackSort::DateAdded => "t.rowid_pk DESC".into(),
        TrackSort::PlayCount => "COALESCE(lc.count, 0) DESC, t.title COLLATE NOCASE".into(),
        TrackSort::Fields(criteria) => {
            if criteria.is_empty() {
                return order_by(&TrackSort::ArtistAlbum);
            }
            let mut cols: Vec<String> = criteria
                .iter()
                .map(|c| {
                    let dir = match c.direction {
                        config::SortDirection::Asc => "ASC",
                        config::SortDirection::Desc => "DESC",
                    };
                    let col = match c.field {
                        config::TrackSortField::Title => "t.title COLLATE NOCASE",
                        config::TrackSortField::Artist => "t.artist COLLATE NOCASE",
                        config::TrackSortField::Album => "t.album COLLATE NOCASE",
                        config::TrackSortField::Duration => "t.duration",
                        config::TrackSortField::DateAdded => "t.rowid_pk",
                    };
                    format!("{col} {dir}")
                })
                .collect();
            // Stable tail so rows equal on every criterion keep album order.
            cols.push("t.disc_number".into());
            cols.push("t.track_number".into());
            cols.push("t.title COLLATE NOCASE".into());
            cols.join(", ")
        }
    }
}

/// WHERE clause + ordered bind values for a filter (after the `source = ?1` bind).
fn filter_clauses(filter: &TrackFilter) -> (String, Vec<String>) {
    let mut sql = String::new();
    let mut binds = Vec::new();
    if !filter.search.trim().is_empty() {
        let n = binds.len() + 2;
        sql.push_str(&format!(
            " AND (t.title LIKE ?{n} ESCAPE '\\' OR t.artist LIKE ?{n} ESCAPE '\\' OR t.album LIKE ?{n} ESCAPE '\\')"
        ));
        binds.push(format!("%{}%", escape_like(filter.search.trim())));
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
    // PlayCount needs the listen_counts join; the other sorts stay join-free
    // so they read straight off the tracks indexes.
    let sql = if filter.sort == TrackSort::PlayCount {
        format!(
            "SELECT {TRACK_COLUMNS} {TRACKS_FROM} \
             LEFT JOIN listen_counts lc ON lc.track_key = {UID_EXPR} \
             WHERE t.source = ?1{clauses} ORDER BY {} LIMIT ?{limit_n} OFFSET ?{}",
            order_by(&filter.sort),
            limit_n + 1,
        )
    } else {
        format!(
            "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1{clauses} ORDER BY {} LIMIT ?{limit_n} OFFSET ?{}",
            order_by(&filter.sort),
            limit_n + 1,
        )
    };
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

pub async fn album_tracks(
    pool: &SqlitePool,
    source: &Source,
    album_id: &str,
) -> Result<Vec<Track>, DbError> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1 AND t.source_album_id = ?2 \
         ORDER BY t.disc_number, t.track_number, t.title COLLATE NOCASE"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source.as_str())
        .bind(album_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn artist_tracks(
    pool: &SqlitePool,
    source: &Source,
    artist: &str,
) -> Result<Vec<Track>, DbError> {
    // Match like the old in-memory derivation did: the primary artist column,
    // secondary credits (artists_json — featured artists get their own tiles),
    // and tracks on albums credited to the artist; all case-insensitively.
    let sql = format!(
        "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1 AND ( \
            t.artist = ?2 COLLATE NOCASE \
            OR EXISTS (SELECT 1 FROM json_each(t.artists_json) WHERE value = ?2 COLLATE NOCASE) \
            OR t.source_album_id IN \
               (SELECT source_album_id FROM albums WHERE source = ?1 AND artist = ?2 COLLATE NOCASE) \
         ) ORDER BY t.album COLLATE NOCASE, t.disc_number, t.track_number, t.title COLLATE NOCASE"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source.as_str())
        .bind(artist)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn genre_tracks(
    pool: &SqlitePool,
    source: &Source,
    genre: &str,
) -> Result<Vec<Track>, DbError> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks t \
         JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id \
         WHERE t.source = ?1 AND a.genre = ?2 \
         ORDER BY t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.disc_number, t.track_number"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source.as_str())
        .bind(genre)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub async fn folder_tracks(pool: &SqlitePool, prefix: &str) -> Result<Vec<Track>, DbError> {
    // Local track_key IS the path. Escape LIKE metachars so a folder named
    // "100%" doesn't widen the match.
    let escaped = escape_like(prefix);
    let sql = format!(
        "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = 'local' \
         AND t.track_key LIKE ?1 ESCAPE '\\' ORDER BY t.track_key"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(format!("{escaped}%"))
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn artist_sample_tracks(
    pool: &SqlitePool,
    source: &Source,
    limit: u32,
) -> Result<Vec<Track>, DbError> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.rowid_pk IN \
           (SELECT MIN(rowid_pk) FROM tracks WHERE source = ?1 GROUP BY artist) \
         ORDER BY t.artist COLLATE NOCASE LIMIT ?2"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source.as_str())
        .bind(limit as i64)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn top_genre(pool: &SqlitePool, source: &Source) -> Result<Option<String>, DbError> {
    let sql = format!(
        "SELECT a.genre FROM tracks t \
         JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id \
         JOIN listen_counts lc ON lc.track_key = {UID_EXPR} \
         WHERE t.source = ?1 AND TRIM(a.genre) != '' \
         GROUP BY a.genre ORDER BY SUM(lc.count) DESC LIMIT 1"
    );
    Ok(sqlx::query_scalar::<_, String>(&sql)
        .bind(source.as_str())
        .fetch_optional(pool)
        .await?)
}

/// The one whole-source read: full-text search needs the corpus because its
/// Unicode-aware matching can't be SQLite `LIKE` (ASCII-only case folding).
/// Runs only when a query is typed — never on page mount.
pub async fn search_corpus(pool: &SqlitePool, source: &Source) -> Result<Vec<Track>, DbError> {
    let sql = format!(
        "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1 \
         ORDER BY t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.disc_number, t.track_number"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source.as_str())
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn tracks_count(pool: &SqlitePool, filter: &TrackFilter) -> Result<u32, DbError> {
    let (clauses, binds) = filter_clauses(filter);
    let sql = format!("SELECT COUNT(*) FROM tracks t WHERE t.source = ?1{clauses}");
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
        "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1 \
         AND t.track_key IN (SELECT value FROM json_each(?2))"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source.as_str())
        .bind(keys_json)
        .fetch_all(pool)
        .await?;
    let by_key: std::collections::HashMap<String, Track> = rows
        .into_iter()
        .map(Into::into)
        .map(|t: Track| (t.id.key().into_owned(), t))
        .collect();
    // get(), not remove(): a playlist can hold the same track twice.
    Ok(keys.iter().filter_map(|k| by_key.get(k).cloned()).collect())
}

pub async fn artists(pool: &SqlitePool, source: &Source) -> Result<Vec<(String, u32)>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query!(
        r#"SELECT artist, COUNT(*) AS "cnt!: i64" FROM tracks WHERE source = ?1 AND artist != ''
         GROUP BY artist ORDER BY artist COLLATE NOCASE"#,
        src
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.artist, r.cnt.max(0) as u32))
        .collect())
}

pub async fn genres(pool: &SqlitePool, source: &Source) -> Result<Vec<String>, DbError> {
    let src = source.as_str();
    Ok(sqlx::query_scalar!(
        "SELECT DISTINCT genre FROM albums WHERE source = ?1 AND genre != '' \
         ORDER BY genre COLLATE NOCASE",
        src
    )
    .fetch_all(pool)
    .await?)
}

pub async fn album(
    pool: &SqlitePool,
    source: &Source,
    album_id: &str,
) -> Result<Option<Album>, DbError> {
    let src = source.as_str();
    let row = sqlx::query_as!(
        AlbumRow,
        "SELECT source_album_id, title, artist, genre, year, cover_path, manual_cover \
         FROM albums WHERE source = ?1 AND source_album_id = ?2",
        src,
        album_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn albums(pool: &SqlitePool, source: &Source) -> Result<Vec<Album>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query_as!(
        AlbumRow,
        "SELECT source_album_id, title, artist, genre, year, cover_path, manual_cover \
         FROM albums WHERE source = ?1 ORDER BY artist COLLATE NOCASE, title COLLATE NOCASE",
        src
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn favorites(pool: &SqlitePool, server_id: &str) -> Result<Vec<String>, DbError> {
    Ok(sqlx::query_scalar!(
        "SELECT ref FROM favorites WHERE server_id = ?1 AND dirty != 2 \
         ORDER BY rank, rowid",
        server_id
    )
    .fetch_all(pool)
    .await?)
}

pub async fn is_favorite(pool: &SqlitePool, server_id: &str, ref_: &str) -> Result<bool, DbError> {
    let n: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM favorites WHERE server_id = ?1 AND ref = ?2 AND dirty != 2",
        server_id,
        ref_
    )
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}
