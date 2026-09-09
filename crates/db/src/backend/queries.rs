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
const TRACK_COLUMNS: &str = "t.track_key, t.service, \
    COALESCE(t.cover_path, CASE WHEN t.service IS NULL THEN a.cover_path END) AS cover_path, \
    t.source_album_id, t.title, \
    t.artist, t.album, t.duration, t.khz, t.bitrate, t.track_number, t.disc_number, \
    t.mb_release_id, t.mb_recording_id, t.mb_track_id, t.playlist_item_id, t.artists_json";

/// `FROM tracks t` + the album join that backs the `COALESCE` in [`TRACK_COLUMNS`].
/// LEFT so a track whose album row is missing still returns (cover → NULL → default).
/// `albums` shares column names with `tracks` (`artist`/`title`/`cover_path`/…), so
/// every query using this must `t.`-qualify its WHERE/ORDER BY columns.
const TRACKS_FROM: &str = "FROM tracks t LEFT JOIN albums a \
    ON a.source = t.source AND a.source_album_id = t.source_album_id";

/// SQL for a track row's listen-count key: built-in Local keeps the legacy
/// path key, named local sources prefix it with their source id, and servers
/// keep the lowercase legacy `service:item_id` uid.
const UID_EXPR: &str = "(CASE WHEN t.service IS NULL THEN \
    (CASE WHEN t.source = 'local' THEN t.track_key ELSE t.source || '|' || t.track_key END) ELSE \
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
    limit: Option<u32>,
) -> Result<Vec<Track>, DbError> {
    // Match like the old in-memory derivation did: the primary artist column,
    // secondary credits (artists_json — featured artists get their own tiles),
    // and tracks on albums credited to the artist; all case-insensitively.
    let limit_clause = limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();
    let sql = format!(
        "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1 AND ( \
            t.artist = ?2 COLLATE NOCASE \
            OR EXISTS (SELECT 1 FROM json_each(t.artists_json) WHERE value = ?2 COLLATE NOCASE) \
            OR t.source_album_id IN \
               (SELECT source_album_id FROM albums WHERE source = ?1 AND artist = ?2 COLLATE NOCASE) \
         ) ORDER BY t.album COLLATE NOCASE, t.disc_number, t.track_number, t.title COLLATE NOCASE\
         {limit_clause}"
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

pub async fn folder_tracks(
    pool: &SqlitePool,
    source: &Source,
    prefix: &str,
) -> Result<Vec<Track>, DbError> {
    // Local track_key IS the path. Escape LIKE metachars so a folder named
    // "100%" doesn't widen the match.
    let escaped = escape_like(prefix);
    let sql = format!(
        "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1 \
         AND t.track_key LIKE ?2 ESCAPE '\\' ORDER BY t.track_key"
    );
    let rows = sqlx::query_as::<_, TrackRow>(&sql)
        .bind(source.as_str())
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

/// Every artist the library credits, primary and secondary alike.
///
/// Grouping on the `artist` column alone would list "A feat. B" and never
/// "B", while [`artist_tracks`] happily answers for "B" and the UI gives it a
/// tile -- so the listing has to enumerate the same credits that one matches
/// on, or the tile has no row and nothing can hang a picture on it.
pub async fn artists(pool: &SqlitePool, source: &Source) -> Result<Vec<(String, u32)>, DbError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT name, COUNT(*) AS cnt FROM ( \
             SELECT t.rowid_pk AS id, TRIM(t.artist) AS name FROM tracks t \
              WHERE t.source = ?1 AND TRIM(t.artist) != '' \
             UNION \
             SELECT t.rowid_pk AS id, TRIM(credit.value) AS name \
               FROM tracks t, json_each(t.artists_json) AS credit \
              WHERE t.source = ?1 AND TRIM(credit.value) != '' \
             UNION \
             SELECT t.rowid_pk AS id, TRIM(a.artist) AS name \
               FROM tracks t JOIN albums a \
                 ON a.source = t.source AND a.source_album_id = t.source_album_id \
              WHERE t.source = ?1 AND TRIM(a.artist) != '' \
         ) GROUP BY name COLLATE NOCASE ORDER BY name COLLATE NOCASE",
    )
    .bind(source.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, count)| (name, count.max(0) as u32))
        .collect())
}

/// One album cover per credited artist: the cover of the earliest album
/// holding a track they are credited on, keyed by the trimmed lowercase name.
///
/// This is the fallback an artist with no photo renders, so the read that
/// advertises it and the fetch that serves the bytes must agree -- a ref is
/// versioned on the picture it names.
pub async fn artist_album_covers(
    pool: &SqlitePool,
    source: &Source,
) -> Result<std::collections::HashMap<String, String>, DbError> {
    // MIN(id) fixes which row the bare `cover` comes from, so the answer does
    // not drift between calls for an artist with several covered albums.
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT name, cover, MIN(id) FROM ( \
             SELECT a.rowid_pk AS id, LOWER(TRIM(t.artist)) AS name, a.cover_path AS cover \
               FROM tracks t JOIN albums a \
                 ON a.source = t.source AND a.source_album_id = t.source_album_id \
              WHERE t.source = ?1 AND a.cover_path IS NOT NULL AND TRIM(t.artist) != '' \
             UNION ALL \
             SELECT a.rowid_pk AS id, LOWER(TRIM(credit.value)) AS name, a.cover_path AS cover \
               FROM tracks t, json_each(t.artists_json) AS credit \
               JOIN albums a \
                 ON a.source = t.source AND a.source_album_id = t.source_album_id \
              WHERE t.source = ?1 AND a.cover_path IS NOT NULL AND TRIM(credit.value) != '' \
             UNION ALL \
             SELECT a.rowid_pk AS id, LOWER(TRIM(a.artist)) AS name, a.cover_path AS cover \
               FROM albums a \
              WHERE a.source = ?1 AND a.cover_path IS NOT NULL AND TRIM(a.artist) != '' \
         ) GROUP BY name",
    )
    .bind(source.as_str())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, cover, _)| (name, cover))
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::backend::migrations::run_migrations(&pool)
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
        }
    }

    async fn seeded() -> (SqlitePool, Source) {
        let pool = mem_pool().await;
        let source = Source::Local;
        // One collaboration: the artist column carries the joined credit, the
        // credit list carries the two names the UI gives tiles to.
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

    /// The listing has to name every credit `artist_tracks` will answer for,
    /// or a tile the UI draws has no row to hang its picture on.
    #[tokio::test]
    async fn every_credit_is_listed_not_just_the_artist_column() {
        let (pool, source) = seeded().await;

        let listed: Vec<String> = artists(&pool, &source)
            .await
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();

        for expected in ["Ada", "Boris", "Cyd", "Various Artists"] {
            assert!(
                listed.iter().any(|name| name == expected),
                "{expected} missing from {listed:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_credited_artist_counts_the_tracks_they_are_on() {
        let (pool, source) = seeded().await;

        let counts: std::collections::HashMap<String, u32> =
            artists(&pool, &source).await.unwrap().into_iter().collect();

        assert_eq!(counts.get("Boris"), Some(&1), "one collaboration");
        assert_eq!(counts.get("Ada"), Some(&2), "both album tracks");
    }

    /// A ref is versioned on the picture it names, so the fallback the listing
    /// advertises and the one the fetch serves come from this one map.
    #[tokio::test]
    async fn a_credited_artist_falls_back_to_the_cover_of_an_album_they_are_on() {
        let (pool, source) = seeded().await;

        let covers = artist_album_covers(&pool, &source).await.unwrap();

        assert_eq!(
            covers.get("boris").map(String::as_str),
            Some("/covers/one.jpg")
        );
        assert_eq!(
            covers.get("ada").map(String::as_str),
            Some("/covers/one.jpg")
        );
        // An album artist no track is credited to still names its own cover.
        assert_eq!(
            covers.get("various artists").map(String::as_str),
            Some("/covers/two.jpg")
        );
    }
}
