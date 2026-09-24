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
    mb.release_id AS mb_release_id, mb.recording_id AS mb_recording_id, mb.track_id AS mb_track_id, \
    (SELECT json_group_array(c.name ORDER BY c.position) \
       FROM track_credits c WHERE c.track_pk = t.rowid_pk) AS artists_json, \
    (SELECT json_group_array(json_object('name', c.name, 'id', c.artist_id) ORDER BY c.position) \
       FROM track_credits c WHERE c.track_pk = t.rowid_pk) AS credits_json";

/// `FROM tracks t` + the album join that backs the `COALESCE` in [`TRACK_COLUMNS`].
/// LEFT so a track whose album row is missing still returns (cover → NULL → default).
/// `albums` shares column names with `tracks` (`artist`/`title`/`cover_path`/…), so
/// every query using this must `t.`-qualify its WHERE/ORDER BY columns.
const TRACKS_FROM: &str = "FROM tracks t LEFT JOIN albums a \
    ON a.source = t.source AND a.source_album_id = t.source_album_id \
    LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk";

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

/// `(track, name, id)` per track credit and per album artist; binds `?1`.
const CREDIT_ROWS: &str = "\
    SELECT c.track_pk AS track, c.name AS name, c.artist_id AS id \
      FROM track_credits c JOIN tracks t ON t.rowid_pk = c.track_pk \
     WHERE t.source = ?1 \
    UNION \
    SELECT t.rowid_pk, TRIM(a.artist), a.artist_id FROM tracks t JOIN albums a \
        ON a.source = t.source AND a.source_album_id = t.source_album_id \
     WHERE t.source = ?1";

const ARTIST_ORDER: &str =
    "ORDER BY t.album COLLATE NOCASE, t.disc_number, t.track_number, t.title COLLATE NOCASE";

/// Matched in Rust: SQLite's `NOCASE` folds ASCII only, and the grid groups on the Unicode fold.
async fn unlinked_artist_rowids(
    pool: &SqlitePool,
    source: &Source,
    key: &utils::artist::ArtistKey,
) -> Result<Vec<i64>, DbError> {
    let sql = format!("SELECT track, name FROM ({CREDIT_ROWS}) WHERE id IS NULL AND name != ''");
    let rows: Vec<(i64, String)> = sqlx::query_as(&sql)
        .bind(source.as_str())
        .fetch_all(pool)
        .await?;
    let mut ids: Vec<i64> = rows
        .into_iter()
        .filter(|(_, name)| utils::artist::ArtistKey::of(name, None) == *key)
        .map(|(track, _)| track)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// Strict identity: an id matches only credits carrying it, and a name only credits carrying none.
pub async fn artist_tracks(
    pool: &SqlitePool,
    source: &Source,
    artist: &reader::ArtistCredit,
    limit: Option<u32>,
) -> Result<Vec<Track>, DbError> {
    let limit_clause = limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default();
    let key = utils::artist::ArtistKey::of(&artist.name, artist.id.as_deref());
    let rows = match &key {
        utils::artist::ArtistKey::Id(id) => {
            let sql = format!(
                "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1 AND ( \
                    t.rowid_pk IN (SELECT track_pk FROM track_credits WHERE artist_id = ?2) \
                    OR t.source_album_id IN \
                       (SELECT source_album_id FROM albums WHERE source = ?1 AND artist_id = ?2) \
                 ) {ARTIST_ORDER}{limit_clause}"
            );
            sqlx::query_as::<_, TrackRow>(&sql)
                .bind(source.as_str())
                .bind(id)
                .fetch_all(pool)
                .await?
        }
        utils::artist::ArtistKey::Name(_) => {
            let rowids = unlinked_artist_rowids(pool, source, &key).await?;
            let sql = format!(
                "SELECT {TRACK_COLUMNS} {TRACKS_FROM} WHERE t.source = ?1 \
                 AND t.rowid_pk IN (SELECT value FROM json_each(?2)) {ARTIST_ORDER}{limit_clause}"
            );
            sqlx::query_as::<_, TrackRow>(&sql)
                .bind(source.as_str())
                .bind(serde_json::to_string(&rowids)?)
                .fetch_all(pool)
                .await?
        }
    };
    Ok(rows.into_iter().map(Into::into).collect())
}

/// The albums billed to one artist, keyed the way [`artist_tracks`] keys it.
pub async fn artist_albums(
    pool: &SqlitePool,
    source: &Source,
    artist: &reader::ArtistCredit,
) -> Result<Vec<Album>, DbError> {
    const COLUMNS: &str =
        "source_album_id, title, artist, genre, year, cover_path, manual_cover, artist_id";
    const ORDER: &str = "ORDER BY year DESC, title COLLATE NOCASE";
    let key = utils::artist::ArtistKey::of(&artist.name, artist.id.as_deref());
    let rows: Vec<AlbumRow> = match &key {
        utils::artist::ArtistKey::Id(id) => {
            sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM albums WHERE source = ?1 AND artist_id = ?2 {ORDER}"
            ))
            .bind(source.as_str())
            .bind(id)
            .fetch_all(pool)
            .await?
        }
        utils::artist::ArtistKey::Name(_) => {
            let rows: Vec<AlbumRow> = sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM albums WHERE source = ?1 AND artist_id IS NULL {ORDER}"
            ))
            .bind(source.as_str())
            .fetch_all(pool)
            .await?;
            rows.into_iter()
                .filter(|row| utils::artist::ArtistKey::of(&row.artist, None) == key)
                .collect()
        }
    };
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
         LEFT JOIN track_musicbrainz mb ON mb.track_pk = t.rowid_pk \
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

/// Grouped on the identity [`artist_tracks`] matches, so a tile opens exactly the tracks it counts.
pub async fn artists(pool: &SqlitePool, source: &Source) -> Result<Vec<crate::ArtistRow>, DbError> {
    use std::collections::{HashMap, HashSet};
    use utils::artist::ArtistKey;

    #[derive(Default)]
    struct Group {
        tracks: HashSet<i64>,
        spellings: HashMap<String, u32>,
    }

    let sql = format!("SELECT track, name, id FROM ({CREDIT_ROWS}) WHERE name != ''");
    let rows: Vec<(i64, String, Option<String>)> = sqlx::query_as(&sql)
        .bind(source.as_str())
        .fetch_all(pool)
        .await?;
    let mut groups: HashMap<ArtistKey, Group> = HashMap::new();
    for (track, name, id) in rows {
        let group = groups
            .entry(ArtistKey::of(&name, id.as_deref()))
            .or_default();
        group.tracks.insert(track);
        *group.spellings.entry(name).or_default() += 1;
    }
    let mut artists: Vec<crate::ArtistRow> = groups
        .into_iter()
        .map(|(key, group)| {
            let name = group
                .spellings
                .into_iter()
                .max_by(|(a, a_n), (b, b_n)| a_n.cmp(b_n).then_with(|| b.cmp(a)))
                .map(|(name, _)| name)
                .unwrap_or_default();
            crate::ArtistRow {
                name,
                id: match key {
                    ArtistKey::Id(id) => Some(id),
                    ArtistKey::Name(_) => None,
                },
                tracks: group.tracks.len() as u32,
            }
        })
        .collect();
    artists.sort_by_cached_key(|artist| (artist.name.to_lowercase(), artist.id.clone()));
    Ok(artists)
}

/// The most-credited source id per normalized name, so a row order never picks the answer.
pub async fn artist_ids(
    pool: &SqlitePool,
    source: &Source,
) -> Result<std::collections::HashMap<String, String>, DbError> {
    // Keyed in Rust: SQLite's `LOWER` folds ASCII only, so "ЛСП" would miss every lookup.
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT c.name, c.artist_id, COUNT(*) FROM track_credits c \
           JOIN tracks t ON t.rowid_pk = c.track_pk \
          WHERE t.source = ?1 AND c.artist_id IS NOT NULL AND c.name != '' \
          GROUP BY c.name, c.artist_id",
    )
    .bind(source.as_str())
    .fetch_all(pool)
    .await?;
    let mut counts: std::collections::HashMap<(String, String), i64> =
        std::collections::HashMap::new();
    for (name, id, cnt) in rows {
        *counts
            .entry((utils::artist::normalize_artist_key(&name), id))
            .or_default() += cnt;
    }
    let mut best: std::collections::HashMap<String, (String, i64)> =
        std::collections::HashMap::new();
    for ((key, id), cnt) in counts {
        let wins = match best.get(&key) {
            None => true,
            Some((held, held_cnt)) => cnt > *held_cnt || (cnt == *held_cnt && id < *held),
        };
        if wins {
            best.insert(key, (id, cnt));
        }
    }
    Ok(best.into_iter().map(|(key, (id, _))| (key, id)).collect())
}

/// The earliest covered album per credited artist, stable so the advertised ref and served bytes agree.
pub async fn artist_album_covers(
    pool: &SqlitePool,
    source: &Source,
) -> Result<std::collections::HashMap<utils::artist::ArtistKey, String>, DbError> {
    let sql = format!(
        "SELECT cr.name, cr.id, a.rowid_pk, a.cover_path FROM ({CREDIT_ROWS}) AS cr \
           JOIN tracks t ON t.rowid_pk = cr.track \
           JOIN albums a ON a.source = t.source AND a.source_album_id = t.source_album_id \
          WHERE cr.name != '' AND a.cover_path IS NOT NULL"
    );
    let rows: Vec<(String, Option<String>, i64, String)> = sqlx::query_as(&sql)
        .bind(source.as_str())
        .fetch_all(pool)
        .await?;
    let mut earliest: std::collections::HashMap<utils::artist::ArtistKey, (i64, String)> =
        std::collections::HashMap::new();
    for (name, id, album, cover) in rows {
        let key = utils::artist::ArtistKey::of(&name, id.as_deref());
        match earliest.get(&key) {
            Some((held, _)) if *held <= album => {}
            _ => {
                earliest.insert(key, (album, cover));
            }
        }
    }
    Ok(earliest
        .into_iter()
        .map(|(key, (_, cover))| (key, cover))
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
        "SELECT source_album_id, title, artist, genre, year, cover_path, manual_cover, artist_id \
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
        "SELECT source_album_id, title, artist, genre, year, cover_path, manual_cover, artist_id \
         FROM albums WHERE source = ?1 ORDER BY artist COLLATE NOCASE, title COLLATE NOCASE",
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
    let limit = limit as i64;
    let rows = sqlx::query_as!(
        AlbumRow,
        "SELECT a.source_album_id, a.title, a.artist, a.genre, a.year, a.cover_path, \
                a.manual_cover, a.artist_id \
         FROM albums a JOIN tracks t \
           ON t.source = a.source AND t.source_album_id = a.source_album_id \
         WHERE a.source = ?1 \
         GROUP BY a.rowid_pk \
         ORDER BY MAX(t.added_at) DESC, MAX(t.rowid_pk) DESC \
         LIMIT ?2",
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
            credits: Vec::new(),
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
            .map(|artist| artist.name)
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

        let counts: std::collections::HashMap<String, u32> = artists(&pool, &source)
            .await
            .unwrap()
            .into_iter()
            .map(|artist| (artist.name, artist.tracks))
            .collect();

        assert_eq!(counts.get("Boris"), Some(&1), "one collaboration");
        assert_eq!(counts.get("Ada"), Some(&2), "both album tracks");
    }

    #[tokio::test]
    async fn a_credited_artist_answers_with_the_id_its_source_issued() {
        let pool = mem_pool().await;
        let source = Source::Local;
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

        let ids = artist_ids(&pool, &source).await.unwrap();

        assert_eq!(ids.get("ada").map(String::as_str), Some("UC-ada"));
        assert_eq!(ids.get("boris"), None, "an unlinked credit has no id");
    }

    /// Answering with whichever row came first would make a tile's target
    /// depend on row order.
    #[tokio::test]
    async fn a_name_two_ids_disagree_on_resolves_to_the_most_credited() {
        let pool = mem_pool().await;
        let source = Source::Local;
        let tracks = [
            linked_track("/a.flac", "Ada", &[("Ada", Some("UC-real"))]),
            linked_track("/b.flac", "Ada", &[("Ada", Some("UC-real"))]),
            linked_track("/c.flac", "Ada", &[("Ada", Some("UC-topic"))]),
        ];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();

        let ids = artist_ids(&pool, &source).await.unwrap();

        assert_eq!(ids.get("ada").map(String::as_str), Some("UC-real"));
    }

    /// SQLite's `LOWER` folds ASCII only; the key has to be the one every
    /// lookup builds, or a Cyrillic or accented name never finds its id.
    #[tokio::test]
    async fn a_non_ascii_name_is_keyed_as_every_lookup_keys_it() {
        let pool = mem_pool().await;
        let source = Source::Local;
        let tracks = [
            linked_track("/a.flac", "ЛСП", &[("ЛСП", Some("UC-lsp"))]),
            linked_track("/b.flac", "Émilie", &[("Émilie", Some("UC-em"))]),
            linked_track("/c.flac", "émilie", &[("émilie", Some("UC-em"))]),
        ];
        super::super::writes::upsert_tracks(&pool, &source, &tracks)
            .await
            .unwrap();

        let ids = artist_ids(&pool, &source).await.unwrap();

        let key = |name: &str| utils::artist::normalize_artist_key(name);
        assert_eq!(ids.get(&key("ЛСП")).map(String::as_str), Some("UC-lsp"));
        assert_eq!(ids.get(&key("ÉMILIE")).map(String::as_str), Some("UC-em"));
        assert_eq!(ids.len(), 2, "two spellings of one name are one artist");
    }

    /// Every row predates the column until a sync rewrites it.
    #[tokio::test]
    async fn tracks_stored_without_credits_have_no_ids_and_still_list() {
        let (pool, source) = seeded().await;

        assert!(artist_ids(&pool, &source).await.unwrap().is_empty());

        let counts: std::collections::HashMap<String, u32> = artists(&pool, &source)
            .await
            .unwrap()
            .into_iter()
            .map(|artist| (artist.name, artist.tracks))
            .collect();
        assert_eq!(counts.get("Ada"), Some(&2));
    }

    async fn homonyms() -> (SqlitePool, Source) {
        let pool = mem_pool().await;
        let source = Source::Server("srv".into());
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

    fn keys(tracks: Vec<Track>) -> Vec<String> {
        let mut keys: Vec<String> = tracks.iter().map(|t| t.id.key().into_owned()).collect();
        keys.sort();
        keys
    }

    #[tokio::test]
    async fn two_ids_behind_one_name_are_two_artists_and_an_unlinked_one_a_third() {
        let (pool, source) = homonyms().await;

        let listed: Vec<(Option<String>, u32)> = artists(&pool, &source)
            .await
            .unwrap()
            .into_iter()
            .filter(|artist| artist.name.eq_ignore_ascii_case("ada"))
            .map(|artist| (artist.id, artist.tracks))
            .collect();

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
    async fn an_id_opens_only_the_tracks_that_carry_it() {
        let (pool, source) = homonyms().await;
        let open = |id: Option<&str>| reader::ArtistCredit {
            name: "Ada".into(),
            id: id.map(Into::into),
        };

        let found = async |id| {
            keys(
                artist_tracks(&pool, &source, &open(id), None)
                    .await
                    .unwrap(),
            )
        };

        assert_eq!(found(Some("ar-1")).await, ["a", "b"]);
        assert_eq!(found(Some("ar-2")).await, ["c"]);
        assert_eq!(
            found(None).await,
            ["d"],
            "a name never reaches a linked credit"
        );
    }

    #[tokio::test]
    async fn a_name_folds_beyond_ascii() {
        let (pool, source) = homonyms().await;
        let open = reader::ArtistCredit::unlinked("АДА");

        let found = artist_tracks(&pool, &source, &open, None).await.unwrap();

        assert_eq!(keys(found), ["e"]);
    }

    #[tokio::test]
    async fn an_artists_albums_follow_the_same_identity() {
        let pool = mem_pool().await;
        let source = Source::Server("srv".into());
        let billed = |id: &str, artist_id: Option<&str>| Album {
            artist_id: artist_id.map(Into::into),
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

        let found = async |artist: reader::ArtistCredit| {
            let albums = artist_albums(&pool, &source, &artist).await.unwrap();
            albums.into_iter().map(|a| a.id).collect::<Vec<_>>()
        };

        assert_eq!(
            found(reader::ArtistCredit::linked("Ada", "ar-1")).await,
            ["x"]
        );
        assert_eq!(found(reader::ArtistCredit::unlinked("ada")).await, ["z"]);
    }

    /// A ref is versioned on the picture it names, so the fallback the listing
    /// advertises and the one the fetch serves come from this one map.
    #[tokio::test]
    async fn a_credited_artist_falls_back_to_the_cover_of_an_album_they_are_on() {
        let (pool, source) = seeded().await;

        let covers = artist_album_covers(&pool, &source).await.unwrap();
        let name = |name: &str| utils::artist::ArtistKey::of(name, None);

        assert_eq!(
            covers.get(&name("Boris")).map(String::as_str),
            Some("/covers/one.jpg")
        );
        assert_eq!(
            covers.get(&name("Ada")).map(String::as_str),
            Some("/covers/one.jpg")
        );
        // An album artist no track is credited to still names its own cover.
        assert_eq!(
            covers.get(&name("Various Artists")).map(String::as_str),
            Some("/covers/two.jpg")
        );
    }
}
