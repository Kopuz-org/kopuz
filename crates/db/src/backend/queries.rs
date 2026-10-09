//! Read queries backing the UI's query hooks (issue #347, step 6).
//!
//! Track listings are sorted + filtered + windowed in SQL (only the visible
//! slice is materialized), so a 20k-row library scrolls without ever holding
//! the whole list in memory. The track query is built at runtime (dynamic
//! `ORDER BY`/`WHERE` from the filter) rather than via the `query!` macro;
//! sort/search clauses are fixed strings, values are always bound.

use reader::models::{Album, Track};
use sqlx::SqlitePool;

use super::rows::{AlbumRow, CreditRow, TrackRow};
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
const TRACK_COLUMNS: &str = "t.rowid_pk, t.track_key, t.service, \
    COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS cover_path, \
    t.source_album_id, t.title, \
    t.artist, t.album, t.duration, t.khz, t.bitrate, t.track_number, t.disc_number, \
    mb.release_id AS mb_release_id, mb.recording_id AS mb_recording_id, mb.track_id AS mb_track_id, \
    t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak";

/// The rows as tracks, their credits read in one query for the lot.
async fn with_credits(pool: &SqlitePool, rows: Vec<TrackRow>) -> Result<Vec<Track>, DbError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let pks: Vec<i64> = rows.iter().map(|row| row.rowid_pk).collect();
    let pks = serde_json::to_string(&pks)?;
    let credits = sqlx::query_as!(
        CreditRow,
        r#"SELECT c.track_pk, c.name, ar.key AS artist_key, ar.source, ar.source_artist_id
             FROM track_credits c JOIN artists ar ON ar.id = c.artist_pk
            WHERE c.track_pk IN (SELECT value FROM json_each(?1))
            ORDER BY c.track_pk, c.position"#,
        pks
    )
    .fetch_all(pool)
    .await?;
    let mut by_track: std::collections::HashMap<i64, Vec<reader::ArtistCredit>> =
        std::collections::HashMap::new();
    for credit in credits {
        by_track
            .entry(credit.track_pk)
            .or_default()
            .push(credit.into());
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let credits = by_track.remove(&row.rowid_pk).unwrap_or_default();
            row.into_track(credits)
        })
        .collect())
}

/// `FROM tracks t` + the album join that backs the `COALESCE` in [`TRACK_COLUMNS`].
/// LEFT so a track whose album row is missing still returns (cover → NULL → default).
/// `albums` shares column names with `tracks` (`artist`/`title`/`cover_path`/…), so
/// every query using this must `t.`-qualify its WHERE/ORDER BY columns.
const TRACKS_FROM: &str = "FROM tracks t LEFT JOIN albums a \
    ON a.source = t.source AND a.source_album_id = t.source_album_id \
    LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk";

fn order_by(sort: &TrackSort) -> String {
    match sort {
        TrackSort::ArtistAlbum => {
            "t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.disc_number, t.track_number, t.title COLLATE NOCASE".into()
        }
        TrackSort::Title => "t.title COLLATE NOCASE".into(),
        TrackSort::Artist => "t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.track_number".into(),
        TrackSort::Album => "t.album COLLATE NOCASE, t.disc_number, t.track_number".into(),
        TrackSort::DateAdded => "t.added_at DESC, t.rowid_pk DESC".into(),
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
                    // A field may span more than one column (date added falls
                    // back to insertion order), and each needs its own direction.
                    let fields: &[&str] = match c.field {
                        config::TrackSortField::Title => &["t.title COLLATE NOCASE"],
                        config::TrackSortField::Artist => &["t.artist COLLATE NOCASE"],
                        config::TrackSortField::Album => &["t.album COLLATE NOCASE"],
                        config::TrackSortField::Duration => &["t.duration"],
                        config::TrackSortField::DateAdded => &["t.added_at", "t.rowid_pk"],
                    };
                    fields
                        .iter()
                        .map(|col| format!("{col} {dir}"))
                        .collect::<Vec<_>>()
                        .join(", ")
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
    if let Some(favorite) = filter.favorite {
        // favorites.server_id holds the same string as tracks.source, and
        // favorites.ref holds the track_key, so this needs no extra bind.
        let exists = if favorite { "EXISTS" } else { "NOT EXISTS" };
        sql.push_str(&format!(
            " AND {exists} (SELECT 1 FROM favorites f \
              WHERE f.server_id = t.source AND f.ref = t.track_key)"
        ));
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
             LEFT JOIN listen_counts lc ON lc.source = t.source AND lc.track_key = t.track_key \
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
    with_credits(pool, rows).await
}

pub async fn album_tracks(
    pool: &SqlitePool,
    source: &Source,
    album_id: &str,
) -> Result<Vec<Track>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query_as!(
        TrackRow,
        r#"SELECT t.rowid_pk AS "rowid_pk!", t.track_key, t.service,
                  COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS "cover_path?: String",
                  t.source_album_id, t.title, t.artist, t.album, t.duration, t.khz, t.bitrate,
                  t.track_number, t.disc_number,
                  mb.release_id AS "mb_release_id?", mb.recording_id AS "mb_recording_id?", mb.track_id AS "mb_track_id?",
                  t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak
             FROM tracks t
             LEFT JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk
            WHERE t.source = ?1 AND t.source_album_id = ?2
            ORDER BY t.disc_number, t.track_number, t.title COLLATE NOCASE"#,
        src,
        album_id
    )
    .fetch_all(pool)
    .await?;
    with_credits(pool, rows).await
}

/// One artist's tracks: those credited to it, and those on an album a real listing bills to it; `limit` caps them.
pub async fn artist_tracks(
    pool: &SqlitePool,
    source: &Source,
    artist: &str,
    limit: Option<u32>,
) -> Result<Vec<Track>, DbError> {
    let src = source.as_str();
    // SQLite reads a negative LIMIT as none.
    let limit = limit.map_or(-1, i64::from);
    let rows = sqlx::query_as!(
        TrackRow,
        r#"SELECT t.rowid_pk AS "rowid_pk!", t.track_key, t.service,
                  COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS "cover_path?: String",
                  t.source_album_id, t.title, t.artist, t.album, t.duration, t.khz, t.bitrate,
                  t.track_number, t.disc_number,
                  mb.release_id AS "mb_release_id?", mb.recording_id AS "mb_recording_id?", mb.track_id AS "mb_track_id?",
                  t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak
             FROM tracks t
             LEFT JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk
            WHERE t.source = ?1 AND t.rowid_pk IN (
                  SELECT track_pk FROM artist_credit_rows
                   WHERE source = ?1 AND artist_pk = (SELECT id FROM artists WHERE source = ?1 AND key = ?2))
            ORDER BY t.album COLLATE NOCASE, t.disc_number, t.track_number, t.title COLLATE NOCASE
            LIMIT ?3"#,
        src,
        artist,
        limit
    )
    .fetch_all(pool)
    .await?;
    with_credits(pool, rows).await
}

pub async fn artist_albums(
    pool: &SqlitePool,
    source: &Source,
    artist: &str,
) -> Result<Vec<Album>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query_as!(
        AlbumRow,
        r#"SELECT al.source_album_id, al.title, al.artist, al.genre, al.year, al.cover_path, al.manual_cover,
                  ar.key AS "artist_key?", ar.source_artist_id AS "artist_source_id?"
             FROM albums al LEFT JOIN artists ar ON ar.id = al.artist_pk
            WHERE al.source = ?1 AND al.artist_pk = (SELECT id FROM artists WHERE source = ?1 AND key = ?2)
            ORDER BY al.year DESC, al.title COLLATE NOCASE"#,
        src,
        artist
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn genre_tracks(
    pool: &SqlitePool,
    source: &Source,
    genre: &str,
) -> Result<Vec<Track>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query_as!(
        TrackRow,
        r#"SELECT t.rowid_pk AS "rowid_pk!", t.track_key, t.service,
                  COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS "cover_path?: String",
                  t.source_album_id, t.title, t.artist, t.album, t.duration, t.khz, t.bitrate,
                  t.track_number, t.disc_number,
                  mb.release_id AS "mb_release_id?", mb.recording_id AS "mb_recording_id?", mb.track_id AS "mb_track_id?",
                  t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak
             FROM tracks t
             JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk
            WHERE t.source = ?1 AND a.genre = ?2
            ORDER BY t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.disc_number, t.track_number"#,
        src,
        genre
    )
    .fetch_all(pool)
    .await?;
    with_credits(pool, rows).await
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub async fn folder_tracks(
    pool: &SqlitePool,
    source: &Source,
    prefix: &str,
) -> Result<Vec<Track>, DbError> {
    let src = source.as_str();
    // Local track_key IS the path. Escape LIKE metachars so a folder named "100%" doesn't widen the match.
    let pattern = format!("{}%", escape_like(prefix));
    let rows = sqlx::query_as!(
        TrackRow,
        r#"SELECT t.rowid_pk AS "rowid_pk!", t.track_key, t.service,
                  COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS "cover_path?: String",
                  t.source_album_id, t.title, t.artist, t.album, t.duration, t.khz, t.bitrate,
                  t.track_number, t.disc_number,
                  mb.release_id AS "mb_release_id?", mb.recording_id AS "mb_recording_id?", mb.track_id AS "mb_track_id?",
                  t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak
             FROM tracks t
             LEFT JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk
            WHERE t.source = ?1 AND t.track_key LIKE ?2 ESCAPE '\'
            ORDER BY t.track_key"#,
        src,
        pattern
    )
    .fetch_all(pool)
    .await?;
    with_credits(pool, rows).await
}

/// One track per billed artist, the credit the byline names else the first, as a tile bills it; the name match folds ASCII case only.
pub async fn artist_sample_tracks(
    pool: &SqlitePool,
    source: &Source,
    limit: u32,
) -> Result<Vec<Track>, DbError> {
    let src = source.as_str();
    let limit = i64::from(limit);
    let rows = sqlx::query_as!(
        TrackRow,
        r#"SELECT t.rowid_pk AS "rowid_pk!", t.track_key, t.service,
                  COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS "cover_path?: String",
                  t.source_album_id, t.title, t.artist, t.album, t.duration, t.khz, t.bitrate,
                  t.track_number, t.disc_number,
                  mb.release_id AS "mb_release_id?", mb.recording_id AS "mb_recording_id?", mb.track_id AS "mb_track_id?",
                  t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak
             FROM tracks t
             JOIN (SELECT MIN(s.rowid_pk) AS track_pk,
                          COALESCE(
                            (SELECT c.artist_pk FROM track_credits c
                              WHERE c.track_pk = s.rowid_pk AND TRIM(c.name) = TRIM(s.artist) COLLATE NOCASE
                              ORDER BY c.position LIMIT 1),
                            (SELECT c.artist_pk FROM track_credits c
                              WHERE c.track_pk = s.rowid_pk AND c.position = 0)) AS billed
                     FROM tracks s
                    WHERE s.source = ?1
                    GROUP BY billed) first ON first.track_pk = t.rowid_pk
             JOIN artists ar ON ar.id = first.billed
             LEFT JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk
            ORDER BY ar.name COLLATE NOCASE
            LIMIT ?2"#,
        src,
        limit
    )
    .fetch_all(pool)
    .await?;
    with_credits(pool, rows).await
}

pub async fn top_genre(pool: &SqlitePool, source: &Source) -> Result<Option<String>, DbError> {
    let src = source.as_str();
    Ok(sqlx::query_scalar!(
        r#"SELECT a.genre FROM tracks t
             JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             JOIN listen_counts lc ON lc.source = t.source AND lc.track_key = t.track_key
            WHERE t.source = ?1 AND TRIM(a.genre) != ''
            GROUP BY a.genre ORDER BY SUM(lc.count) DESC LIMIT 1"#,
        src
    )
    .fetch_optional(pool)
    .await?)
}

/// The one whole-source read: full-text search needs the corpus because its
/// Unicode-aware matching can't be SQLite `LIKE` (ASCII-only case folding).
/// Runs only when a query is typed — never on page mount.
pub async fn search_corpus(pool: &SqlitePool, source: &Source) -> Result<Vec<Track>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query_as!(
        TrackRow,
        r#"SELECT t.rowid_pk AS "rowid_pk!", t.track_key, t.service,
                  COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS "cover_path?: String",
                  t.source_album_id, t.title, t.artist, t.album, t.duration, t.khz, t.bitrate,
                  t.track_number, t.disc_number,
                  mb.release_id AS "mb_release_id?", mb.recording_id AS "mb_recording_id?", mb.track_id AS "mb_track_id?",
                  t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak
             FROM tracks t
             LEFT JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk
            WHERE t.source = ?1
            ORDER BY t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.disc_number, t.track_number"#,
        src
    )
    .fetch_all(pool)
    .await?;
    with_credits(pool, rows).await
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
    let src = source.as_str();
    let keys_json = serde_json::to_string(keys)?;
    let rows = sqlx::query_as!(
        TrackRow,
        r#"SELECT t.rowid_pk AS "rowid_pk!", t.track_key, t.service,
                  COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS "cover_path?: String",
                  t.source_album_id, t.title, t.artist, t.album, t.duration, t.khz, t.bitrate,
                  t.track_number, t.disc_number,
                  mb.release_id AS "mb_release_id?", mb.recording_id AS "mb_recording_id?", mb.track_id AS "mb_track_id?",
                  t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak
             FROM tracks t
             LEFT JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk
            WHERE t.source = ?1 AND t.track_key IN (SELECT value FROM json_each(?2))"#,
        src,
        keys_json
    )
    .fetch_all(pool)
    .await?;
    let by_key: std::collections::HashMap<String, Track> = with_credits(pool, rows)
        .await?
        .into_iter()
        .map(|t| (t.id.key().into_owned(), t))
        .collect();
    // get(), not remove(): a playlist can hold the same track twice.
    Ok(keys.iter().filter_map(|k| by_key.get(k).cloned()).collect())
}

/// Swap each queued track for its library row where one exists, so a restore shows what a sync since wrote.
pub(crate) async fn refresh_from_library(
    pool: &SqlitePool,
    source: &Source,
    queue: Vec<Track>,
) -> Result<Vec<Track>, DbError> {
    if queue.is_empty() {
        return Ok(queue);
    }
    let src = source.as_str();
    let keys: Vec<String> = queue.iter().map(|t| t.id.key().into_owned()).collect();
    let keys_json = serde_json::to_string(&keys)?;
    let rows = sqlx::query_as!(
        TrackRow,
        r#"SELECT t.rowid_pk AS "rowid_pk!", t.track_key, t.service,
                  COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS "cover_path?: String",
                  t.source_album_id, t.title, t.artist, t.album, t.duration, t.khz, t.bitrate,
                  t.track_number, t.disc_number,
                  mb.release_id AS "mb_release_id?", mb.recording_id AS "mb_recording_id?", mb.track_id AS "mb_track_id?",
                  t.rg_track_gain, t.rg_track_peak, t.rg_album_gain, t.rg_album_peak
             FROM tracks t
             LEFT JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id
             LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk
            WHERE t.source = ?1 AND t.track_key IN (SELECT value FROM json_each(?2))"#,
        src,
        keys_json
    )
    .fetch_all(pool)
    .await?;
    let library = with_credits(pool, rows).await?;
    Ok(queue
        .into_iter()
        .map(
            |queued| match library.iter().find(|row| row.id == queued.id) {
                // The playlist entry is the queued row's own; the library row has none.
                Some(row) => Track {
                    playlist_item_id: queued.playlist_item_id,
                    ..row.clone()
                },
                None => queued,
            },
        )
        .collect())
}

/// Every artist a track of `source` credits, one row per artist key.
pub async fn artists(pool: &SqlitePool, source: &Source) -> Result<Vec<crate::ArtistRow>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query!(
        r#"SELECT ar.key, ar.source_artist_id, ar.name, COUNT(*) AS "tracks!: i64"
             FROM artist_credit_rows cr
             JOIN artists ar ON ar.id = cr.artist_pk
            WHERE cr.source = ?1
            GROUP BY ar.id"#,
        src
    )
    .fetch_all(pool)
    .await?;
    let mut artists: Vec<crate::ArtistRow> = rows
        .into_iter()
        .map(|row| crate::ArtistRow {
            key: row.key,
            source_id: row.source_artist_id,
            name: row.name,
            tracks: row.tracks.max(0) as u32,
        })
        .collect();
    artists.sort_by_cached_key(|artist| (artist.name.to_lowercase(), artist.key.clone()));
    Ok(artists)
}

pub async fn artist_keys_unnamed_by_source(
    pool: &SqlitePool,
    source: &Source,
) -> Result<std::collections::HashSet<String>, DbError> {
    let src = source.as_str();
    let keys = sqlx::query_scalar!(
        "SELECT key FROM artists \
          WHERE source = ?1 AND source_artist_id IS NOT NULL AND named_by_source = 0",
        src
    )
    .fetch_all(pool)
    .await?;
    Ok(keys.into_iter().collect())
}

/// The key of each linked artist of `source`, by the id the source issued.
pub async fn linked_artist_keys(
    pool: &SqlitePool,
    source: &Source,
) -> Result<std::collections::HashMap<String, String>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query!(
        r#"SELECT source_artist_id AS "source_artist_id!: String", key FROM artists
            WHERE source = ?1 AND source_artist_id IS NOT NULL"#,
        src
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.source_artist_id, r.key))
        .collect())
}

/// The key of each unlinked artist of `source`, by folded name.
pub async fn unlinked_artist_keys(
    pool: &SqlitePool,
    source: &Source,
) -> Result<std::collections::HashMap<String, String>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query!(
        "SELECT name_key, key FROM artists WHERE source = ?1 AND source_artist_id IS NULL",
        src
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| (r.name_key, r.key)).collect())
}

/// One artist of `source` by its key, counted as [`artists`] counts it.
pub async fn artist(
    pool: &SqlitePool,
    source: &Source,
    artist: &str,
) -> Result<Option<crate::ArtistRow>, DbError> {
    let src = source.as_str();
    let row = sqlx::query!(
        r#"SELECT ar.key, ar.source_artist_id, ar.name,
                  (SELECT COUNT(*) FROM artist_credit_rows cr
                    WHERE cr.source = ?1 AND cr.artist_pk = ar.id) AS "tracks!: i64"
             FROM artists ar WHERE ar.source = ?1 AND ar.key = ?2"#,
        src,
        artist
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| crate::ArtistRow {
        key: row.key,
        source_id: row.source_artist_id,
        name: row.name,
        tracks: row.tracks.max(0) as u32,
    }))
}

/// The earliest covered album per credited artist, by key, stable so the advertised ref and served bytes agree.
pub async fn artist_album_covers(
    pool: &SqlitePool,
    source: &Source,
) -> Result<std::collections::HashMap<String, String>, DbError> {
    let src = source.as_str();
    let rows = sqlx::query!(
        r#"SELECT ar.key, cv.cover_path AS "cover_path!"
             FROM artist_cover_albums cv JOIN artists ar ON ar.id = cv.artist_pk
            WHERE cv.source = ?1"#,
        src
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.key, row.cover_path))
        .collect())
}

/// One artist's entry of [`artist_album_covers`].
pub async fn artist_album_cover(
    pool: &SqlitePool,
    source: &Source,
    artist: &str,
) -> Result<Option<String>, DbError> {
    let src = source.as_str();
    Ok(sqlx::query_scalar!(
        r#"SELECT cover_path AS "cover_path!" FROM artist_cover_albums
            WHERE source = ?1 AND artist_pk = (SELECT id FROM artists WHERE source = ?1 AND key = ?2)"#,
        src,
        artist
    )
    .fetch_optional(pool)
    .await?)
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
        r#"SELECT al.source_album_id, al.title, al.artist, al.genre, al.year, al.cover_path, al.manual_cover,
                  ar.key AS "artist_key?", ar.source_artist_id AS "artist_source_id?"
             FROM albums al LEFT JOIN artists ar ON ar.id = al.artist_pk
            WHERE al.source = ?1 AND al.source_album_id = ?2"#,
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
        r#"SELECT al.source_album_id, al.title, al.artist, al.genre, al.year, al.cover_path, al.manual_cover,
                  ar.key AS "artist_key?", ar.source_artist_id AS "artist_source_id?"
             FROM albums al LEFT JOIN artists ar ON ar.id = al.artist_pk
            WHERE al.source = ?1
            ORDER BY al.artist COLLATE NOCASE, al.title COLLATE NOCASE"#,
        src
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Albums ordered by the newest track they hold, newest first, on the same
/// `added_at`-then-insertion order [`TrackSort::DateAdded`] uses. An album is
/// recent when its music is, not when its row happened to be written, and
/// ordering on the tracks is also what lets music added to an album Kopuz
/// already knows pull that album back up.
///
/// [`TrackSort::DateAdded`]: crate::TrackSort::DateAdded
pub async fn albums_recently_added(
    pool: &SqlitePool,
    source: &Source,
    limit: u32,
) -> Result<Vec<Album>, DbError> {
    let src = source.as_str();
    let limit = i64::from(limit);
    let rows = sqlx::query_as!(
        AlbumRow,
        r#"SELECT al.source_album_id, al.title, al.artist, al.genre, al.year, al.cover_path, al.manual_cover,
                  ar.key AS "artist_key?", ar.source_artist_id AS "artist_source_id?"
             FROM albums al LEFT JOIN artists ar ON ar.id = al.artist_pk
             JOIN tracks t ON t.source = al.source AND t.source_album_id = al.source_album_id
            WHERE al.source = ?1
            GROUP BY al.rowid_pk
            ORDER BY MAX(t.added_at) DESC, MAX(t.rowid_pk) DESC
            LIMIT ?2"#,
        src,
        limit
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArtistRow;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::backend::migrations::run_migrations(&pool, None)
            .await
            .unwrap();
        pool
    }

    fn track(key: &str, artist: &str, credits: &[&str], album_id: &str) -> Track {
        Track {
            id: reader::TrackId::Local(std::path::PathBuf::from(key)),
            cover: None,
            album_id: album_id.to_string(),
            title: key.to_string(),
            artist: artist.to_string(),
            album: "Album".into(),
            duration: 60,
            khz: 44,
            bitrate: 320,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: credits.iter().map(|name| name.to_string()).collect(),
            replay_gain: config::ReplayGainInfo::default(),
            credits: Vec::new(),
            explicit: false,
            plays: None,
        }
    }

    fn linked_track(key: &str, artist: &str, credits: &[(&str, Option<&str>)]) -> Track {
        let names: Vec<&str> = credits.iter().map(|(name, _)| *name).collect();
        Track {
            credits: credits
                .iter()
                .map(|(name, id)| match id {
                    Some(id) => reader::ArtistCredit::linked(*name, *id),
                    None => reader::ArtistCredit::unlinked(*name),
                })
                .collect(),
            ..track(key, artist, &names, "al-1")
        }
    }

    fn album(id: &str, artist: &str, cover: Option<&str>) -> Album {
        Album {
            id: id.to_string(),
            title: "Album".into(),
            artist: artist.to_string(),
            genre: String::new(),
            year: 0,
            cover_path: cover.map(std::path::PathBuf::from),
            manual_cover: false,
            artist_id: None,
            artist_key: None,
        }
    }

    async fn seeded() -> (SqlitePool, Source) {
        let pool = mem_pool().await;
        let source = Source::default();
        // One collaboration: the artist column carries the joined credit, the credits its two names.
        let tracks = [
            track("/a.flac", "Ada feat. Boris", &["Ada", "Boris"], "al-1"),
            track("/b.flac", "Ada", &["Ada"], "al-1"),
            track("/c.flac", "Cyd", &["Cyd"], "al-2"),
        ];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();
        let albums = [
            album("al-1", "Ada", Some("/covers/one.jpg")),
            album("al-2", "Various Artists", Some("/covers/two.jpg")),
        ];
        super::super::writes::upsert_albums(&pool, &source, &albums)
            .await
            .unwrap();
        (pool, source)
    }

    fn named<'a>(listed: &'a [ArtistRow], name: &str) -> &'a ArtistRow {
        listed
            .iter()
            .find(|artist| artist.name == name)
            .unwrap_or_else(|| panic!("{name} missing from {listed:?}"))
    }

    fn keys(tracks: Vec<Track>) -> Vec<String> {
        let mut keys: Vec<String> = tracks.iter().map(|t| t.id.key().into_owned()).collect();
        keys.sort();
        keys
    }

    /// Every credit `artist_tracks` answers for is listed, or a tile the UI draws has no row for its picture.
    #[tokio::test]
    async fn every_credit_is_listed_not_just_the_artist_column() {
        let (pool, source) = seeded().await;

        let listed = artists(&pool, &source).await.unwrap();

        for expected in ["Ada", "Boris", "Cyd", "Various Artists"] {
            named(&listed, expected);
        }
    }

    #[tokio::test]
    async fn a_credited_artist_counts_the_tracks_they_are_on() {
        let (pool, source) = seeded().await;

        let listed = artists(&pool, &source).await.unwrap();

        assert_eq!(named(&listed, "Boris").tracks, 1, "one collaboration");
        assert_eq!(named(&listed, "Ada").tracks, 2, "both album tracks");
    }

    #[tokio::test]
    async fn a_credited_artist_carries_the_id_its_source_issued() {
        let pool = mem_pool().await;
        let source = Source::default();
        let tracks = [
            linked_track(
                "/a.flac",
                "Ada feat. Boris",
                &[("Ada", Some("UC-ada")), ("Boris", None)],
            ),
            linked_track("/b.flac", "Ada", &[("Ada", Some("UC-ada"))]),
        ];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();

        let listed = artists(&pool, &source).await.unwrap();

        assert_eq!(named(&listed, "Ada").source_id.as_deref(), Some("UC-ada"));
        assert_eq!(named(&listed, "Boris").source_id, None);
        assert_eq!(
            named(&listed, "Ada").key,
            "UC-ada",
            "a linked artist is keyed by its source's id"
        );
    }

    /// SQLite's `LOWER` folds ASCII only, yet two spellings of one unlinked name are one artist in any script.
    #[tokio::test]
    async fn an_unlinked_name_folds_beyond_ascii() {
        let pool = mem_pool().await;
        let source = Source::default();
        let tracks = [
            track("/a.flac", "ЛСП", &["ЛСП"], "al-1"),
            track("/b.flac", "Émilie", &["Émilie"], "al-1"),
            track("/c.flac", "émilie", &["émilie"], "al-1"),
        ];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();

        let listed = artists(&pool, &source).await.unwrap();

        assert_eq!(listed.len(), 2, "{listed:?}");
        assert_eq!(
            named(&listed, "Émilie").tracks,
            2,
            "the first spelling names it"
        );
    }

    #[tokio::test]
    async fn tracks_stored_without_credits_list_by_name_with_no_ids() {
        let (pool, source) = seeded().await;

        let listed = artists(&pool, &source).await.unwrap();

        assert!(listed.iter().all(|artist| artist.source_id.is_none()));
        assert_eq!(named(&listed, "Ada").tracks, 2);
    }

    async fn homonyms() -> (SqlitePool, Source) {
        let pool = mem_pool().await;
        let source = Source::Server("srv".into());
        // All four share a track-derived album, whose guessed artist must credit none of them.
        let tracks = [
            linked_track("a", "Ada", &[("Ada", Some("ar-1"))]),
            linked_track("b", "ADA", &[("ADA", Some("ar-1"))]),
            linked_track("c", "Ada", &[("Ada", Some("ar-2"))]),
            linked_track("d", "Ada", &[("Ada", None)]),
            track("e", "Ада", &["Ада"], "al-9"),
        ];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();
        (pool, source)
    }

    async fn unlinked(pool: &SqlitePool, source: &Source, name: &str) -> String {
        let listed = artists(pool, source).await.unwrap();
        listed
            .iter()
            .find(|artist| artist.source_id.is_none() && artist.name == name)
            .unwrap_or_else(|| panic!("no unlinked {name} in {listed:?}"))
            .key
            .clone()
    }

    #[tokio::test]
    async fn two_ids_behind_one_name_are_two_artists_and_an_unlinked_one_a_third() {
        let (pool, source) = homonyms().await;

        let listed: Vec<(Option<String>, u32)> = artists(&pool, &source)
            .await
            .unwrap()
            .into_iter()
            .filter(|artist| artist.name.eq_ignore_ascii_case("ada"))
            .map(|artist| (artist.source_id, artist.tracks))
            .collect();

        let mut listed = listed;
        listed.sort();
        assert_eq!(
            listed,
            vec![
                (None, 1),
                (Some("ar-1".into()), 2),
                (Some("ar-2".into()), 1)
            ]
        );
    }

    #[tokio::test]
    async fn a_linked_artist_is_named_as_its_source_last_named_it() {
        let (pool, source) = homonyms().await;

        let listed = artists(&pool, &source).await.unwrap();

        let ar1 = listed
            .iter()
            .find(|artist| artist.source_id.as_deref() == Some("ar-1"))
            .unwrap();
        assert_eq!(ar1.name, "ADA");
    }

    #[tokio::test]
    async fn an_artist_opens_only_the_tracks_filed_under_it() {
        let (pool, source) = homonyms().await;
        let found =
            async |artist: &str| keys(artist_tracks(&pool, &source, artist, None).await.unwrap());

        assert_eq!(found("ar-1").await, ["a", "b"]);
        assert_eq!(found("ar-2").await, ["c"]);
        assert_eq!(
            found(&unlinked(&pool, &source, "Ada").await).await,
            ["d"],
            "a name never reaches a linked credit"
        );
        assert_eq!(found(&unlinked(&pool, &source, "Ада").await).await, ["e"]);
    }

    #[tokio::test]
    async fn one_artist_is_named_and_counted_as_the_listing_does() {
        let (pool, source) = homonyms().await;

        let linked = artist(&pool, &source, "ar-1").await.unwrap().expect("ar-1");
        assert_eq!((linked.name.as_str(), linked.tracks), ("ADA", 2));
        let other = Source::Server("elsewhere".into());
        assert_eq!(
            artist(&pool, &other, "ar-1").await.unwrap(),
            None,
            "another source's row"
        );
        assert_eq!(artist(&pool, &source, "ar-9").await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_unlinked_joined_credit_is_its_own_listed_artist() {
        let pool = mem_pool().await;
        let source = Source::Server("srv".into());
        let tracks = [
            track("a", "COOL&CREATE", &["COOL&CREATE"], "al-1"),
            track(
                "b",
                "COOL&CREATE, beatMARIO",
                &["COOL&CREATE, beatMARIO"],
                "al-2",
            ),
            track("c", "Tyler, The Creator", &["Tyler, The Creator"], "al-3"),
        ];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();

        let names: Vec<String> = artists(&pool, &source)
            .await
            .unwrap()
            .into_iter()
            .map(|artist| artist.name)
            .collect();

        assert_eq!(
            names,
            [
                "COOL&CREATE",
                "COOL&CREATE, beatMARIO",
                "Tyler, The Creator"
            ]
        );
    }

    #[tokio::test]
    async fn a_blank_id_is_stored_as_none() {
        let pool = mem_pool().await;
        let source = Source::Server("srv".into());
        let tracks = [linked_track("a", "Ada", &[("Ada", Some("  "))])];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();

        let listed = artists(&pool, &source).await.unwrap();

        assert_eq!(listed[0].source_id, None);
    }

    #[tokio::test]
    async fn an_artists_albums_follow_the_same_identity() {
        let pool = mem_pool().await;
        let source = Source::Server("srv".into());
        let billed = |id: &str, artist_id: Option<&str>| Album {
            artist_id: artist_id.map(Into::into),
            artist_key: artist_id.map(Into::into),
            ..album(id, "Ada", None)
        };
        let albums = [
            billed("x", Some("ar-1")),
            billed("y", Some("ar-2")),
            billed("z", None),
        ];
        super::super::writes::upsert_albums(&pool, &source, &albums)
            .await
            .unwrap();

        let found = async |artist: &str| {
            let albums = artist_albums(&pool, &source, artist).await.unwrap();
            albums.into_iter().map(|a| a.id).collect::<Vec<_>>()
        };

        assert_eq!(found("ar-1").await, ["x"]);
        assert_eq!(
            found(&unlinked_row(&pool, &source, "Ada").await).await,
            ["z"]
        );
        let x = album_by_id(&pool, &source, "x").await;
        assert_eq!(x.artist_id.as_deref(), Some("ar-1"));
        assert_eq!(x.artist_key.as_deref(), Some("ar-1"));
    }

    async fn unlinked_row(pool: &SqlitePool, source: &Source, name: &str) -> String {
        sqlx::query_scalar(
            "SELECT key FROM artists WHERE source = ?1 AND name = ?2 AND source_artist_id IS NULL",
        )
        .bind(source.as_str())
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn album_by_id(pool: &SqlitePool, source: &Source, id: &str) -> Album {
        super::album(pool, source, id).await.unwrap().unwrap()
    }

    /// The listing's fallback and the single fetch's must agree, since a ref is versioned on the picture it names.
    #[tokio::test]
    async fn a_credited_artist_falls_back_to_the_cover_of_an_album_they_are_on() {
        let (pool, source) = seeded().await;
        let listed = artists(&pool, &source).await.unwrap();

        let covers = artist_album_covers(&pool, &source).await.unwrap();

        for (name, cover) in [
            ("Boris", "/covers/one.jpg"),
            ("Ada", "/covers/one.jpg"),
            // An album artist no track is credited to still names its own cover.
            ("Various Artists", "/covers/two.jpg"),
        ] {
            let key = &named(&listed, name).key;
            assert_eq!(covers.get(key).map(String::as_str), Some(cover), "{name}");
            let one = artist_album_cover(&pool, &source, key).await.unwrap();
            assert_eq!(one.as_deref(), Some(cover), "{name} alone");
        }
    }

    #[tokio::test]
    async fn a_track_reads_back_with_its_credits_in_billing_order() {
        let pool = mem_pool().await;
        let source = Source::Server("srv".into());
        let tracks = [linked_track(
            "a",
            "Ada feat. Boris",
            &[("Ada", Some("ar-1")), ("Boris", None)],
        )];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();

        let read = tracks_by_keys(&pool, &source, &["a".into()]).await.unwrap();

        let credits: Vec<(&str, Option<&str>, bool)> = read[0]
            .credits
            .iter()
            .map(|c| (c.name.as_str(), c.id.as_deref(), c.key.is_some()))
            .collect();
        assert_eq!(
            credits,
            [("Ada", Some("ar-1"), true), ("Boris", None, true)]
        );
        assert_eq!(read[0].artists, ["Ada", "Boris"]);
        assert!(
            read[0]
                .credits
                .iter()
                .all(|c| c.source.as_ref() == Some(&source))
        );
    }

    /// An id is its issuer's alone, so a credit another source listed is filed here by name.
    #[tokio::test]
    async fn an_id_another_source_listed_is_filed_unlinked() {
        let pool = mem_pool().await;
        let source = Source::Server("srv".into());
        let mut foreign = linked_track("a", "Ada", &[("Ada", Some("ar-1"))]);
        foreign.credits[0].source = Some(Source::Server("elsewhere".into()));
        super::super::writes::upsert_tracks(&pool, &source, &[foreign])
            .await
            .unwrap();

        let listed = artists(&pool, &source).await.unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].source_id, None);
        assert_eq!(artist(&pool, &source, "ar-1").await.unwrap(), None);
    }
}
