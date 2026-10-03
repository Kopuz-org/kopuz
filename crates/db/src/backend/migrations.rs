//! Database migrations, both senses:
//! 1. the sqlx **schema migrator** ([`run_migrations`]) that applies
//!    `migrations/*.sql`, with a line-ending-tolerant checksum reconciler and a
//!    pre-migration [`snapshot_if_pending`] backup; and
//! 2. the one-shot **legacy `*.json` → SQLite importer** ([`run_json_import`],
//!    issue #347).
//!
//! ## JSON importer
//!
//! Runs once per database (gated on the DB being empty), so it's safe to call
//! on every launch. The whole import is a single transaction — a crash before
//! commit leaves the DB empty and the JSONs untouched, so it re-runs cleanly.
//! A file that fails to parse is skipped (and left in place for repair); the
//! rest import normally. The names actually consumed are recorded in
//! `kv`, and [`finalize_migration`] renames exactly those files to
//! `*.json.bak` (kept for downgrade; never deleted).
//!
//! Legacy `Track.path` was the overloaded `"service:id[:cover]"` string; we
//! parse it here (the one place, via [`TrackId::from_legacy_path`]) into the
//! typed id, lifting the smuggled cover out of the 3rd segment.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use sqlx::SqlitePool;

use super::{open_pool, with_ext};
use crate::{DbError, ImportReport};
use reader::models::{Track, TrackId};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Run migrations, tolerating a checksum mismatch that's purely a line-ending
/// difference of the same migration SQL: sqlx checksums raw bytes, so a CRLF
/// (Windows) and an LF (Linux/macOS) checkout of an identical migration hash
/// differently. On a `VersionMismatch` we reconcile and retry; a checksum that
/// matches neither line ending is a genuine edit and still fails.
pub(super) async fn run_migrations(
    pool: &SqlitePool,
    settings_path: Option<&Path>,
) -> Result<(), DbError> {
    for fill in Fill::ALL {
        let (made_room, dropped_old) = fill.between();
        if applied(pool, dropped_old).await? {
            continue;
        }
        let mut through = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                MIGRATOR
                    .iter()
                    .filter(|m| m.version <= made_room)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        through.set_ignore_missing(true);
        migrate(pool, &through).await?;
        fill.run(pool, settings_path).await?;
    }
    migrate(pool, &MIGRATOR).await
}

/// Data SQL can't move, filled in Rust after the migration that makes room for it and before the one dropping its old home.
#[derive(Clone, Copy)]
enum Fill {
    Artists,
    Queue,
    State,
    DerivedAlbums,
}

impl Fill {
    const ALL: [Fill; 4] = [Fill::Artists, Fill::Queue, Fill::State, Fill::DerivedAlbums];

    /// The migration the fill follows, and the one it must precede.
    fn between(self) -> (i64, i64) {
        match self {
            Fill::Artists => (ARTISTS_CREATED, 20260922000001),
            Fill::Queue => (QUEUE_ROWS_CREATED, 20260930000001),
            Fill::State => (STATE_TABLES_CREATED, 20260930000006),
            Fill::DerivedAlbums => (LYRICS_CACHE_DROPPED, 20260930000008),
        }
    }

    async fn run(self, pool: &SqlitePool, settings_path: Option<&Path>) -> Result<(), DbError> {
        match self {
            Fill::Artists => fill_artists(pool).await,
            Fill::Queue => fill_queue(pool).await,
            Fill::State => fill_state(pool, settings_path).await,
            Fill::DerivedAlbums => fill_derived_albums(pool).await,
        }
    }
}

/// Points each derived album at the artist its tracks credit now, as the writes keep it; before, it kept
/// whatever row its first track was credited to when the album was made.
async fn fill_derived_albums(pool: &SqlitePool) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    // Each derived album against the first of its tracks, which is the one that made it.
    let derived: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT al.rowid_pk, t.rowid_pk, t.artist FROM albums al \
           JOIN tracks t ON t.rowid_pk = (SELECT MIN(rowid_pk) FROM tracks \
                WHERE source = al.source AND source_album_id = al.source_album_id) \
          WHERE al.derived = 1",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (album, track, byline) in derived {
        let credits: Vec<(String, i64)> = sqlx::query_as(
            "SELECT name, artist_pk FROM track_credits WHERE track_pk = ?1 ORDER BY position",
        )
        .bind(track)
        .fetch_all(&mut *tx)
        .await?;
        if let Some(artist) = super::writes::billed_credit(&byline, &credits) {
            sqlx::query("UPDATE albums SET artist_pk = ?1 WHERE rowid_pk = ?2")
                .bind(artist)
                .bind(album)
                .execute(&mut *tx)
                .await?;
        }
    }

    tx.commit().await?;
    Ok(())
}

const ARTISTS_CREATED: i64 = 20260922000000;
const QUEUE_ROWS_CREATED: i64 = 20260930000000;
const STATE_TABLES_CREATED: i64 = 20260930000005;
const LYRICS_CACHE_DROPPED: i64 = 20260930000007;

async fn migrate(pool: &SqlitePool, migrator: &sqlx::migrate::Migrator) -> Result<(), DbError> {
    let adapted;
    let migrator = if applied(pool, 20260924000001).await? {
        // A database from master can already have split server credentials before
        // seeing this branch's older WebView migration. Apply its equivalent on
        // the current schema, retaining the original checksum for older installs.
        let mut migrations = migrator.migrations.to_vec();
        for migration in &mut migrations {
            if migration.version == 20260919010000 {
                migration.sql =
                    include_str!("../../migrations/20261003000000_restore_webview_credentials.sql")
                        .into();
            }
        }
        adapted = sqlx::migrate::Migrator {
            migrations: migrations.into(),
            ignore_missing: migrator.ignore_missing,
            ..sqlx::migrate::Migrator::DEFAULT
        };
        &adapted
    } else {
        migrator
    };
    match migrator.run(pool).await {
        Ok(()) => Ok(()),
        Err(sqlx::migrate::MigrateError::VersionMismatch(_)) => {
            reconcile_eol_checksums(pool).await?;
            migrator.run(pool).await.map_err(Into::into)
        }
        Err(e) => Err(e.into()),
    }
}

async fn applied(pool: &SqlitePool, version: i64) -> Result<bool, DbError> {
    let tracked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    if !tracked {
        return Ok(false);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = ?1 AND success = 1)",
    )
    .bind(version)
    .fetch_one(pool)
    .await?)
}

/// Files every stored credit and album artist under an artist row, in Rust because SQLite's `LOWER` folds ASCII only.
async fn fill_artists(pool: &SqlitePool) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    for sql in [
        "DELETE FROM track_credits",
        "UPDATE albums SET artist_pk = NULL",
        "DELETE FROM artists",
    ] {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    let tracks: Vec<(i64, String, String, String)> =
        sqlx::query_as("SELECT rowid_pk, source, artist, artists_json FROM tracks")
            .fetch_all(&mut *tx)
            .await?;
    for (pk, source, artist, artists_json) in tracks {
        let listed: Vec<String> = serde_json::from_str(&artists_json).unwrap_or_default();
        let names = match listed.is_empty() {
            true => vec![artist],
            false => listed,
        };
        let names = names
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty());
        for (position, name) in names.enumerate() {
            let artist_pk = file_unlinked(&mut tx, &source, name).await?;
            sqlx::query(
                "INSERT INTO track_credits (track_pk, position, artist_pk, name) VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(pk)
            .bind(position as i64)
            .bind(artist_pk)
            .bind(name)
            .execute(&mut *tx)
            .await?;
        }
    }
    let albums: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT rowid_pk, source, artist FROM albums")
            .fetch_all(&mut *tx)
            .await?;
    for (pk, source, artist) in albums {
        let artist = artist.trim();
        if artist.is_empty() {
            continue;
        }
        let artist_pk = file_unlinked(&mut tx, &source, artist).await?;
        sqlx::query("UPDATE albums SET artist_pk = ?1 WHERE rowid_pk = ?2")
            .bind(artist_pk)
            .bind(pk)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Kept apart from the live writer so a later change to `artists` can't rewrite what this step did.
async fn file_unlinked(
    conn: &mut sqlx::SqliteConnection,
    source: &str,
    name: &str,
) -> Result<i64, DbError> {
    Ok(sqlx::query_scalar(
        "INSERT INTO artists (source, name, name_key) VALUES (?1, ?2, ?3) \
         ON CONFLICT(source, name_key) WHERE source_artist_id IS NULL DO UPDATE SET name = artists.name \
         RETURNING id",
    )
    .bind(source)
    .bind(name)
    .bind(utils::artist::normalize_artist_key(name))
    .fetch_one(conn)
    .await?)
}

/// Moves the stored queue out of its JSON columns into rows, read through the same serde that wrote it.
async fn fill_queue(pool: &SqlitePool) -> Result<(), DbError> {
    let stored: Option<(String, String)> =
        sqlx::query_as("SELECT queue_json, shuffle_order_json FROM queue_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    let Some((queue_json, shuffle_json)) = stored else {
        return Ok(());
    };
    let queue: Vec<Track> = serde_json::from_str(&queue_json).unwrap_or_default();
    let shuffle: Vec<usize> = serde_json::from_str(&shuffle_json).unwrap_or_default();
    let mut tx = pool.begin().await?;
    for sql in [
        "DELETE FROM queue_shuffle",
        "DELETE FROM queue_credits",
        "DELETE FROM queue_tracks",
    ] {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    for (position, t) in queue.iter().enumerate() {
        let position = position as i64;
        sqlx::query(
            "INSERT INTO queue_tracks (position, track_key, service, source_album_id, title, artist, \
               album, duration, khz, bitrate, track_number, disc_number, cover_path, mb_release_id, \
               mb_recording_id, mb_track_id, playlist_item_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        )
        .bind(position)
        .bind(t.id.key().into_owned())
        .bind(t.id.service().map(service_str))
        .bind(&t.album_id)
        .bind(&t.title)
        .bind(&t.artist)
        .bind(&t.album)
        .bind((t.duration != u64::MAX).then_some(t.duration as i64))
        .bind(t.khz as i64)
        .bind(t.bitrate as i64)
        .bind(t.track_number.map(|n| n as i64))
        .bind(t.disc_number.map(|n| n as i64))
        .bind(&t.cover)
        .bind(&t.musicbrainz_release_id)
        .bind(&t.musicbrainz_recording_id)
        .bind(&t.musicbrainz_track_id)
        .bind(&t.playlist_item_id)
        .execute(&mut *tx)
        .await?;
        // A row stored before credits existed keeps its names as unlinked credits.
        let credits: std::borrow::Cow<[reader::ArtistCredit]> = match t.credits.is_empty() {
            false => t.credits.as_slice().into(),
            true => t
                .artists
                .iter()
                .map(reader::ArtistCredit::unlinked)
                .collect(),
        };
        for (at, credit) in credits.iter().enumerate() {
            sqlx::query(
                "INSERT INTO queue_credits (queue_position, position, name, source_artist_id, source) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(position)
            .bind(at as i64)
            .bind(&credit.name)
            .bind(&credit.id)
            .bind(credit.source.as_ref().map(|source| source.as_str()))
            .execute(&mut *tx)
            .await?;
        }
    }
    let queued = queue.len();
    for (step, position) in shuffle.into_iter().enumerate() {
        if position >= queued {
            continue;
        }
        sqlx::query("INSERT INTO queue_shuffle (step, position) VALUES (?1, ?2)")
            .bind(step as i64)
            .bind(position as i64)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Moves the blob's state, credentials and lists into their tables, and its settings into the settings file.
async fn fill_state(pool: &SqlitePool, settings_path: Option<&Path>) -> Result<(), DbError> {
    let stored: Option<String> = sqlx::query_scalar("SELECT json FROM app_config WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    let Some(stored) = stored else {
        return Ok(());
    };
    let blob: serde_json::Value = serde_json::from_str(&stored)?;
    let layers = settings_path
        .map(config::store::FileLayers::read)
        .unwrap_or_default();
    // The file's settings win over the blob's mirror of them, as every load already applied them.
    let cfg: config::AppConfig = layers.merge_and_parse(blob)?;
    let name = |value: serde_json::Value| match value {
        serde_json::Value::String(name) => Ok(name),
        other => Err(DbError::Serde(format!("{other} is not a unit variant"))),
    };

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT OR REPLACE INTO app_state (id, device_id, active_source, source_explicitly_set, volume, \
           discord_presence_paused, fullscreen_tabs_collapsed, sort_order, album_view_mode, \
           artist_album_view_mode, artists_view_mode, artist_view_order, listen_now_style, hero_height) \
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )
    .bind(&cfg.device_id)
    .bind(cfg.active_source.as_str())
    .bind(cfg.source_explicitly_set)
    .bind(f64::from(cfg.volume))
    .bind(cfg.discord_presence_paused)
    .bind(cfg.fullscreen_tabs_collapsed)
    .bind(name(serde_json::to_value(&cfg.sort_order)?)?)
    .bind(name(serde_json::to_value(cfg.album_view_mode)?)?)
    .bind(name(serde_json::to_value(cfg.artist_album_view_mode)?)?)
    .bind(name(serde_json::to_value(cfg.artists_view_mode)?)?)
    .bind(name(serde_json::to_value(&cfg.artist_view_order)?)?)
    .bind(name(serde_json::to_value(cfg.listen_now_style)?)?)
    .bind(i64::from(cfg.hero_height))
    .execute(&mut *tx)
    .await?;

    let sorts = [
        ("albums", serde_json::to_value(&cfg.album_sort)?),
        ("library", serde_json::to_value(&cfg.library_sort)?),
        (
            "artist_albums",
            serde_json::to_value(&cfg.artist_album_sort)?,
        ),
        ("artists", serde_json::to_value(&cfg.artist_sort)?),
    ];
    for (view, criteria) in sorts {
        let criteria = criteria.as_array().cloned().unwrap_or_default();
        for (position, criterion) in criteria.into_iter().enumerate() {
            sqlx::query(
                "INSERT OR REPLACE INTO view_sorts (view, position, field, direction) VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(view)
            .bind(position as i64)
            .bind(name(criterion["field"].clone())?)
            .bind(name(criterion["direction"].clone())?)
            .execute(&mut *tx)
            .await?;
        }
    }
    for (position, item) in cfg.sidebar_order.iter().enumerate() {
        sqlx::query("INSERT OR REPLACE INTO sidebar_items (position, item) VALUES (?1, ?2)")
            .bind(position as i64)
            .bind(item)
            .execute(&mut *tx)
            .await?;
    }
    for (position, section) in cfg.home_sections.iter().enumerate() {
        sqlx::query(
            "INSERT OR REPLACE INTO home_sections (position, key, enabled) VALUES (?1, ?2, ?3)",
        )
        .bind(position as i64)
        .bind(&section.key)
        .bind(section.enabled)
        .execute(&mut *tx)
        .await?;
    }
    let credentials = [
        ("musicbrainz_token", &cfg.musicbrainz_token),
        ("lastfm_api_key", &cfg.lastfm_api_key),
        ("lastfm_api_secret", &cfg.lastfm_api_secret),
        ("lastfm_session_key", &cfg.lastfm_session_key),
        ("librefm_api_key", &cfg.librefm_api_key),
        ("librefm_api_secret", &cfg.librefm_api_secret),
        ("librefm_session_key", &cfg.librefm_session_key),
    ];
    for (key, value) in credentials {
        if value.is_empty() {
            continue;
        }
        sqlx::query("INSERT OR REPLACE INTO integration_credentials (key, value) VALUES (?1, ?2)")
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }
    for (position, entry) in cfg.downloader_history.iter().enumerate() {
        sqlx::query(
            "INSERT OR REPLACE INTO ytdlp_history (position, url, title, format, status, error) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(position as i64)
        .bind(&entry.url)
        .bind(&entry.title)
        .bind(&entry.format)
        .bind(&entry.status)
        .bind(&entry.error)
        .execute(&mut *tx)
        .await?;
    }
    for (server, paths) in &cfg.server_folders {
        for (position, path) in paths.iter().enumerate() {
            sqlx::query(
                "INSERT OR REPLACE INTO server_folders (server_id, position, path) \
                 SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM servers WHERE id = ?1)",
            )
            .bind(server)
            .bind(position as i64)
            .bind(path)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;

    if let Some(path) = settings_path {
        config::store::save_settings_file(path, &serde_json::to_value(&cfg)?, &layers.locked_keys)
            .map_err(|error| DbError::Io(format!("{}: {error}", path.display())))?;
    }
    Ok(())
}

/// Re-stamp `_sqlx_migrations` rows whose checksum differs from this binary's
/// only by line endings. `VersionMismatch` reports just the first offender, so
/// reconcile every applied migration in one pass before retrying.
async fn reconcile_eol_checksums(pool: &SqlitePool) -> Result<(), DbError> {
    use sha2::{Digest, Sha384};

    let stored: HashMap<i64, Vec<u8>> =
        sqlx::query_as::<_, (i64, Vec<u8>)>("SELECT version, checksum FROM _sqlx_migrations")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();

    for m in MIGRATOR.iter() {
        let Some(stored_ck) = stored.get(&m.version) else {
            continue;
        };
        if stored_ck.as_slice() == m.checksum.as_ref() {
            continue;
        }
        // If either line-ending variant matches the stored checksum, the SQL
        // (hence schema) is identical — only EOL differs.
        let lf = m.sql.replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        let matches_eol_variant = [lf.as_bytes(), crlf.as_bytes()]
            .into_iter()
            .any(|bytes| Sha384::digest(bytes).as_slice() == stored_ck.as_slice());
        if matches_eol_variant {
            sqlx::query("UPDATE _sqlx_migrations SET checksum = ?1 WHERE version = ?2")
                .bind(m.checksum.as_ref())
                .bind(m.version)
                .execute(pool)
                .await?;
            tracing::warn!(
                version = m.version,
                "reconciled migration checksum (line-ending-only difference)"
            );
        }
    }
    Ok(())
}

/// Before applying new migrations to an existing DB, copy it (plus WAL sidecars)
/// to `<db>.pre-<applied_version>.bak` so a downgrade can restore it. Best-effort.
pub(super) async fn snapshot_if_pending(path: &Path) {
    if !path.exists() {
        return; // fresh DB, nothing to snapshot
    }
    let Ok(pool) = open_pool(path).await else {
        return;
    };
    // Max applied version (the table won't exist on a pre-migration legacy DB).
    let applied: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap_or(None);
    let available = MIGRATOR.iter().map(|m| m.version).max();
    let pending = match (applied, available) {
        (Some(a), Some(v)) => v > a,
        (None, Some(_)) => false, // fresh/just-created DB with no migrations yet → not a downgrade risk
        _ => false,
    };
    pool.close().await;
    if !pending {
        return;
    }
    let stamp = applied.unwrap_or(0);
    // Keep the first snapshot at this version as the authoritative rollback
    // point. A retry of the same pending migration (or the EOL reconciler having
    // since re-stamped `_sqlx_migrations`) must not overwrite it with an
    // already-modified DB — `backup_name` is deterministic, so guard on it.
    if backup_name(path, stamp, "").exists() {
        return;
    }
    for ext in ["", "-wal", "-shm"] {
        let src = with_ext(path, ext);
        if src.exists() {
            let dst = backup_name(path, stamp, ext);
            if let Err(e) = std::fs::copy(&src, &dst) {
                tracing::warn!(error = %e, src = %src.display(), "db: pre-migration snapshot failed");
            }
        }
    }
    tracing::info!(
        applied = stamp,
        "db: snapshotted before applying pending migrations"
    );
}

fn backup_name(path: &Path, stamp: i64, suffix: &str) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(format!(".pre-{stamp}.bak{suffix}"));
    std::path::PathBuf::from(s)
}

const LEGACY_FILES: [&str; 5] = [
    "config.json",
    "library.json",
    "playlists.json",
    "favorites.json",
    "queue_state.json",
];

/// The on-disk source for one legacy store: the plain `X.json` if it's still
/// there, else the `X.json.bak` a previous finalize moved it to. The fallback
/// matters because debug (`kopuz-debug.db`) and release (`kopuz.db`) are
/// separate databases — whichever imports second finds only the `.bak`s.
fn legacy_source(config_dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    let plain = config_dir.join(name);
    if plain.exists() {
        return Some(plain);
    }
    let bak = config_dir.join(format!("{name}.bak"));
    bak.exists().then_some(bak)
}

#[tracing::instrument(skip_all)]
pub async fn run_json_import(
    pool: &SqlitePool,
    config_dir: &Path,
    settings_path: &Path,
) -> Result<ImportReport, DbError> {
    // Gate on THIS database being empty — no shared sentinel, so each DB
    // (debug/release) imports once on its own.
    if db_has_data(pool).await? {
        return Ok(ImportReport::default());
    }
    if !LEGACY_FILES
        .iter()
        .any(|f| legacy_source(config_dir, f).is_some())
    {
        return Ok(ImportReport::default());
    }

    // Per-file tolerance: a corrupt file (truncated by a power loss, say)
    // imports as its default and is NOT recorded as consumed, so finalize
    // leaves it on disk for repair while everything else migrates.
    let mut consumed: Vec<&str> = Vec::new();
    let read_src = |name: &str| legacy_source(config_dir, name);
    let cfg_val: serde_json::Value = match read_src("config.json") {
        Some(p) => match read_json_tolerant(&p) {
            Some(v) => {
                consumed.push("config.json");
                v
            }
            None => serde_json::Value::Null,
        },
        None => serde_json::Value::Null,
    };
    let lib: LegacyLibrary = match read_src("library.json") {
        Some(p) => match read_json_tolerant(&p) {
            Some(v) => {
                consumed.push("library.json");
                v
            }
            None => LegacyLibrary::default(),
        },
        None => LegacyLibrary::default(),
    };
    let plists: LegacyPlaylists = match read_src("playlists.json") {
        Some(p) => match read_json_tolerant(&p) {
            Some(v) => {
                consumed.push("playlists.json");
                v
            }
            None => LegacyPlaylists::default(),
        },
        None => LegacyPlaylists::default(),
    };
    let favs: LegacyFavorites = match read_src("favorites.json") {
        Some(p) => match read_json_tolerant(&p) {
            Some(v) => {
                consumed.push("favorites.json");
                v
            }
            None => LegacyFavorites::default(),
        },
        None => LegacyFavorites::default(),
    };
    let queue: LegacyQueue = match read_src("queue_state.json") {
        Some(p) => match read_json_tolerant(&p) {
            Some(v) => {
                consumed.push("queue_state.json");
                v
            }
            None => LegacyQueue::default(),
        },
        None => LegacyQueue::default(),
    };

    let now = now_secs();
    let mut tx = pool.begin().await?;

    // --- servers + active-server creds, resolving the active server id -----
    let active_server_id = import_servers(&mut tx, &cfg_val, now).await?;
    let server_src = active_server_id.clone();

    // --- app_config blob (minus servers/creds/listen_counts) + listen_counts -
    let imported_config = import_config(&mut tx, &cfg_val, &active_server_id).await?;
    import_listen_counts(&mut tx, &cfg_val, active_server_id.as_deref()).await?;
    import_recently_played(&mut tx, &cfg_val, &active_server_id).await?;

    // The YT sync times become each YT server's stamps, or its first open would re-stream the whole liked library.
    let yt_stamps = [
        ("synced:favorites", lib.last_yt_sync_at),
        ("synced:playlists", lib.last_yt_playlists_sync_at),
    ];
    for (stamp, at) in yt_stamps {
        let Some(at) = at.map(|at| at.to_string()) else {
            continue;
        };
        sqlx::query!(
            "INSERT INTO kv (name, kind, value) SELECT ?1, id, ?2 FROM servers WHERE service = 'YtMusic' \
             ON CONFLICT(kind, name) DO UPDATE SET value = ?2",
            stamp,
            at
        )
        .execute(&mut *tx)
        .await?;
    }

    // Server-scoped rows need a real server id: every reader keys on 'local'
    // or a servers.id, and a server added later gets a fresh id — rows filed
    // under a made-up source would be unreachable forever. Signed out at
    // migration time ⇒ skip them; the server re-syncs everything after
    // sign-in, and the originals stay in *.json.bak regardless.
    if server_src.is_none()
        && (!lib.jellyfin_tracks.is_empty()
            || !plists.jellyfin_playlists.is_empty()
            || !favs.jellyfin_favorites.is_empty())
    {
        tracing::warn!(
            "db: legacy server data present but no signed-in server — skipping it (re-syncs after sign-in)"
        );
    }

    // --- albums (local + server) ------------------------------------------
    for a in &lib.albums {
        insert_album(&mut tx, "local", a).await?;
    }
    if let Some(sid) = &server_src {
        for a in &lib.jellyfin_albums {
            insert_album(&mut tx, sid, a).await?;
        }
    }

    // --- tracks (local + server) ------------------------------------------
    for lt in &lib.tracks {
        if let Some(t) = legacy_to_track(lt) {
            insert_track(&mut tx, "local", &t).await?;
        }
    }
    if let Some(sid) = &server_src {
        for lt in &lib.jellyfin_tracks {
            if let Some(t) = legacy_to_track(lt) {
                insert_track(&mut tx, sid, &t).await?;
            }
        }
    }

    // --- artist images -----------------------------------------------------
    let server_sources: Vec<&str> = server_src.as_deref().into_iter().collect();
    let both_sources: Vec<&str> = ["local"]
        .into_iter()
        .chain(server_sources.clone())
        .collect();
    import_artist_images(
        &mut tx,
        &server_sources,
        "server",
        &lib.server_artist_images,
    )
    .await?;
    import_artist_images(&mut tx, &["local"], "local", &lib.local_artist_images).await?;
    import_artist_images(&mut tx, &both_sources, "custom", &lib.custom_artist_images).await?;

    // --- playlists + membership -------------------------------------------
    for (i, p) in plists.playlists.iter().enumerate() {
        let pk = insert_playlist(
            &mut tx,
            "local",
            &p.id,
            &p.name,
            &p.cover_path,
            None,
            i as i64,
        )
        .await?;
        insert_playlist_tracks(&mut tx, pk, &p.tracks).await?;
    }
    if let Some(sid) = &server_src {
        for (i, p) in plists.jellyfin_playlists.iter().enumerate() {
            let pk = insert_playlist(
                &mut tx,
                sid,
                &p.id,
                &p.name,
                &p.cover_path,
                p.image_tag.as_deref(),
                i as i64,
            )
            .await?;
            insert_playlist_tracks(&mut tx, pk, &p.tracks).await?;
        }
    }
    for f in &plists.folders {
        sqlx::query!(
            "INSERT OR IGNORE INTO folders (id, source, name) VALUES (?1, 'local', ?2)",
            f.id,
            f.name
        )
        .execute(&mut *tx)
        .await?;
        for (pos, pl) in f.playlist_ids.iter().enumerate() {
            let pos = pos as i64;
            sqlx::query!(
                "INSERT OR IGNORE INTO folder_playlists (folder_id, playlist_ref, position) \
                 VALUES (?1, ?2, ?3)",
                f.id,
                pl,
                pos
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    // --- favorites ---------------------------------------------------------
    for r in &favs.local_favorites {
        insert_favorite(&mut tx, "local", r, now).await?;
    }
    if let Some(sid) = server_src.as_deref() {
        for r in &favs.jellyfin_favorites {
            insert_favorite(&mut tx, sid, r, now).await?;
        }
        // The imported set IS the pull baseline — stamp it so the first
        // reconcile after migration doesn't immediately re-fetch the whole
        // remote favorites list (a full browse stream on YT).
        if !favs.jellyfin_favorites.is_empty() {
            let now_s = now.to_string();
            sqlx::query!(
                "INSERT INTO kv (name, kind, value) VALUES ('fav_pull', ?1, ?2) \
                 ON CONFLICT(kind, name) DO UPDATE SET value = ?2",
                sid,
                now_s
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    // --- queue snapshot ----------------------------------------------------
    let snapshot = crate::QueueSnapshot {
        version: queue.version.min(u8::MAX as u32) as u8,
        queue: queue.queue.iter().filter_map(legacy_to_track).collect(),
        current_queue_index: queue.current_queue_index.max(0) as usize,
        progress_secs: queue.progress_secs.max(0) as u64,
        shuffle_order: queue.shuffle_order.iter().map(|&at| at as usize).collect(),
        shuffle_enabled: queue.shuffle_enabled,
    };
    super::writes::write_queue(&mut tx, imported_config.active_source.as_str(), &snapshot).await?;

    // Record what this import actually consumed, so finalize never moves aside a skipped corrupt file.
    for file in &consumed {
        sqlx::query!(
            "INSERT INTO kv (name, kind, value) VALUES (?1, 'legacy_import', '') \
             ON CONFLICT(kind, name) DO NOTHING",
            file
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    super::cfg_store::write_settings(&imported_config, settings_path)?;

    let report = ImportReport {
        ran: true,
        tracks: count(pool, "SELECT COUNT(*) FROM tracks").await,
        albums: count(pool, "SELECT COUNT(*) FROM albums").await,
        playlists: count(pool, "SELECT COUNT(*) FROM playlists").await,
        favorites: count(pool, "SELECT COUNT(*) FROM favorites").await,
        servers: count(pool, "SELECT COUNT(*) FROM servers").await,
    };
    tracing::info!(
        tracks = report.tracks,
        albums = report.albums,
        playlists = report.playlists,
        favorites = report.favorites,
        servers = report.servers,
        "db: legacy JSON import complete"
    );
    Ok(report)
}

/// Rename each plain `X.json` a real import consumed → `X.json.bak` (kept for
/// downgrade; never deleted). Gated on the consumed-files record the importer
/// writes — gating on "DB non-empty" would also fire when the import failed
/// and later runtime writes populated the DB, renaming files that were never
/// imported. A file the importer skipped as corrupt stays in place. Idempotent.
/// Also drops the obsolete `.db_migrated` sentinel from earlier builds.
/// Returns how many files were renamed.
pub async fn finalize_migration(pool: &SqlitePool, config_dir: &Path) -> Result<usize, DbError> {
    let consumed: Vec<String> =
        sqlx::query_scalar!("SELECT name FROM kv WHERE kind = 'legacy_import'")
            .fetch_all(pool)
            .await?;
    if consumed.is_empty() {
        return Ok(0);
    }
    let mut renamed = 0;
    for f in LEGACY_FILES {
        let src = config_dir.join(f);
        if consumed.iter().any(|c| c == f) && src.exists() {
            backup_aside(&src);
            renamed += 1;
        }
    }
    let _ = std::fs::remove_file(config_dir.join(".db_migrated"));
    Ok(renamed)
}

// ---------------------------------------------------------------------------
// Section importers
// ---------------------------------------------------------------------------

/// Insert the saved-servers list, then upsert the active server WITH its creds.
/// Returns the resolved active server id (for `active_server_id` + server-track
/// source stamping). Creds (tokens/cookies) are handled locally and never logged.
async fn import_servers(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    cfg: &serde_json::Value,
    now: i64,
) -> Result<Option<String>, DbError> {
    if let Some(arr) = cfg.get("servers").and_then(|v| v.as_array()) {
        for s in arr {
            let id = str_at(s, "id");
            if id.is_empty() {
                continue;
            }
            let name = str_at(s, "name");
            let url = str_at(s, "url");
            let service = service_at(s);
            let yt_browser = opt_str_at(s, "yt_browser");
            let yt_anon = s
                .get("yt_anonymous")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let inserted = sqlx::query!(
                "INSERT OR IGNORE INTO servers (id, name, url, service) VALUES (?1, ?2, ?3, ?4)",
                id,
                name,
                url,
                service
            )
            .execute(&mut **tx)
            .await?
            .rows_affected();
            if inserted > 0 {
                let options = legacy_options(yt_browser.as_deref(), yt_anon);
                super::cfg_store::write_server_options(tx, &id, &options).await?;
            }
        }
    }

    let Some(srv) = cfg.get("server").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let name = str_at(srv, "name");
    let url = str_at(srv, "url");
    let service = service_at(srv);
    let token = opt_str_at(srv, "access_token");
    let user_id = opt_str_at(srv, "user_id");
    let yt_browser = opt_str_at(srv, "yt_browser");
    let yt_anon = srv
        .get("yt_anonymous")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Resolve id: explicit, else match a saved server by (url, service), else synth.
    let resolved = opt_str_at(srv, "id")
        .or_else(|| {
            cfg.get("servers")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|s| str_at(s, "url") == url && service_at(s) == service)
                        .map(|s| str_at(s, "id"))
                })
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| format!("legacy-{service}"));

    super::cfg_store::upsert_server_row(tx, &resolved, &name, &url, &service, now).await?;
    let options = legacy_options(yt_browser.as_deref(), yt_anon);
    super::cfg_store::write_server_options(tx, &resolved, &options).await?;
    super::cfg_store::write_server_credentials(
        tx,
        &resolved,
        token.as_deref(),
        user_id.as_deref(),
        now,
    )
    .await?;

    Ok(Some(resolved))
}

/// The legacy file predates Apple Music, so only the YT options can be set.
fn legacy_options(yt_browser: Option<&str>, yt_anonymous: bool) -> Vec<(&'static str, String)> {
    let defaults = config::MusicServer::default();
    super::cfg_store::server_options(
        yt_browser,
        yt_anonymous,
        &defaults.apple_music_storefront,
        &defaults.apple_music_language,
    )
}

/// The legacy config as an `AppConfig`, its state and credentials written to their tables; the caller writes its settings file after commit.
async fn import_config(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    cfg: &serde_json::Value,
    active_server_id: &Option<String>,
) -> Result<config::AppConfig, DbError> {
    let mut legacy = if cfg.is_null() {
        serde_json::json!({})
    } else {
        cfg.clone()
    };
    if let Some(obj) = legacy.as_object_mut() {
        // Servers, creds and counts were imported into their own tables already.
        for key in ["server", "servers", "listen_counts", "active_server_id"] {
            obj.remove(key);
        }
        obj.insert(
            "active_source".into(),
            match active_server_id {
                Some(id) => serde_json::json!({ "Server": id }),
                None => serde_json::json!("Local"),
            },
        );
    }
    let config: config::AppConfig = match serde_json::from_value(legacy) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(%error, "legacy config.json is unreadable; importing defaults");
            config::AppConfig::default()
        }
    };
    super::cfg_store::write_state(tx, &config, &config.active_source).await?;
    for (item_id, path) in &config.offline_tracks {
        sqlx::query!(
            "INSERT OR REPLACE INTO offline_tracks (item_id, path) VALUES (?1, ?2)",
            item_id,
            path
        )
        .execute(&mut **tx)
        .await?;
    }
    for manifest in &config.pinned_stations {
        let id = serde_json::from_str::<serde_json::Value>(manifest)
            .ok()
            .and_then(|station| station.get("id")?.as_str().map(str::to_owned));
        let Some(id) = id else {
            continue;
        };
        super::writes::pin_station(tx, &id, manifest).await?;
    }
    Ok(config)
}

/// `listen_counts` map → its own table, keyed by the source-qualified id
/// ([`TrackId::uid`]) the runtime looks up by (legacy keys carried the cover).
async fn import_listen_counts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    cfg: &serde_json::Value,
    active_server: Option<&str>,
) -> Result<(), DbError> {
    let Some(map) = cfg.get("listen_counts").and_then(|v| v.as_object()) else {
        return Ok(());
    };
    for (k, v) in map {
        // Legacy server counts all belong to the one server the file knew.
        let (source, key) = match TrackId::from_legacy_path(k) {
            TrackId::Local(path) => ("local", path.to_string_lossy().into_owned()),
            TrackId::Server { item_id, .. } => match active_server {
                Some(server) => (server, item_id),
                None => continue,
            },
        };
        let count = v.as_i64().unwrap_or(0);
        // Accumulate: distinct legacy keys can collapse to one track (the old
        // "service:id:cover" form re-keyed when a cover changed).
        sqlx::query!(
            "INSERT INTO listen_counts (source, track_key, count) VALUES (?1, ?2, ?3) \
             ON CONFLICT(source, track_key) DO UPDATE SET count = count + ?3",
            source,
            key,
            count
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Lift the legacy recently-played lists into the per-source `recently_played`
/// table (the importer predated it). The local list keys the local partition;
/// the single legacy server list keys the active server. Lists are newest-first,
/// so the head gets the highest rank (matching `push_recent`'s MAX+1 ordering).
async fn import_recently_played(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    cfg: &serde_json::Value,
    active_server_id: &Option<String>,
) -> Result<(), DbError> {
    let mut lists: Vec<(String, &str)> = vec![("local".to_string(), "recently_played")];
    if let Some(id) = active_server_id {
        lists.push((id.clone(), "recently_played_server"));
    }
    for (source, field) in lists {
        let Some(arr) = cfg.get(field).and_then(|v| v.as_array()) else {
            continue;
        };
        let n = arr.len() as i64;
        for (i, item) in arr.iter().enumerate() {
            if let Some(key) = item.as_str() {
                let rank = n - i as i64; // newest (i=0) → highest rank
                sqlx::query(
                    "INSERT OR IGNORE INTO recently_played (source, track_key, played_at) VALUES (?1, ?2, ?3)",
                )
                .bind(&source)
                .bind(key)
                .bind(rank)
                .execute(&mut **tx)
                .await?;
            }
        }
    }
    Ok(())
}

async fn import_artist_images(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    sources: &[&str],
    kind: &str,
    map: &HashMap<String, String>,
) -> Result<(), DbError> {
    // The legacy maps are keyed by folded name: the unlinked rows of that name, and a linked one only for a custom photo.
    for source in sources {
        for (artist, image) in map {
            sqlx::query!(
                "INSERT OR IGNORE INTO artist_images (source, artist_key, kind, image_ref) \
                 SELECT source, key, ?3, ?4 FROM artists \
                  WHERE source = ?1 AND name_key = ?2 AND (source_artist_id IS NULL OR ?3 = 'custom')",
                source,
                artist,
                kind,
                image
            )
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

async fn insert_album(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source: &str,
    a: &LegacyAlbum,
) -> Result<(), DbError> {
    let manual = a.manual_cover as i64;
    let billed = a.artist.trim();
    let artist_pk = match billed.is_empty() {
        true => None,
        false => Some(super::writes::file_artist(tx, source, billed, None).await?),
    };
    sqlx::query!(
        "INSERT OR IGNORE INTO albums \
           (source, source_album_id, title, artist, genre, year, cover_path, manual_cover, artist_pk) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        source,
        a.id,
        a.title,
        a.artist,
        a.genre,
        a.year,
        a.cover_path,
        manual,
        artist_pk
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_track(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source: &str,
    t: &Track,
) -> Result<(), DbError> {
    let track_key = t.id.key().into_owned();
    let path = t.id.local_path().map(|p| p.to_string_lossy().into_owned());
    let service = t.id.service().map(|s| service_str(s).to_string());
    let duration = t.duration as i64;
    let khz = t.khz as i64;
    let bitrate = t.bitrate as i64;
    let track_number = t.track_number.map(|n| n as i64);
    let disc_number = t.disc_number.map(|n| n as i64);
    let inserted = sqlx::query_scalar!(
        "INSERT OR IGNORE INTO tracks \
           (source, track_key, path, service, source_album_id, title, artist, album, duration, \
            khz, bitrate, track_number, disc_number, cover_path) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
         RETURNING rowid_pk AS \"pk!: i64\"",
        source,
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
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(pk) = inserted {
        super::writes::write_track_children(tx, source, pk, t).await?;
    }
    Ok(())
}

async fn insert_playlist(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source: &str,
    source_pl_id: &str,
    name: &str,
    cover_path: &Option<String>,
    image_tag: Option<&str>,
    position: i64,
) -> Result<i64, DbError> {
    let rec = sqlx::query!(
        "INSERT INTO playlists (source, source_pl_id, name, cover_path, image_tag, position) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(source, source_pl_id) DO UPDATE SET name=?3, cover_path=?4, image_tag=?5, position=?6 \
         RETURNING rowid_pk",
        source,
        source_pl_id,
        name,
        cover_path,
        image_tag,
        position
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(rec.rowid_pk)
}

async fn insert_playlist_tracks(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    playlist_pk: i64,
    refs: &[String],
) -> Result<(), DbError> {
    for (pos, r) in refs.iter().enumerate() {
        let pos = pos as i64;
        sqlx::query!(
            "INSERT OR IGNORE INTO playlist_tracks (playlist_pk, position, track_ref) \
             VALUES (?1, ?2, ?3)",
            playlist_pk,
            pos,
            r
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn insert_favorite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    server_id: &str,
    ref_: &str,
    now: i64,
) -> Result<(), DbError> {
    sqlx::query!(
        "INSERT OR IGNORE INTO favorites (server_id, ref, created_at) VALUES (?1, ?2, ?3)",
        server_id,
        ref_,
        now
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a mirrored track to the typed shape. Tolerates BOTH on-disk forms:
/// the legacy `"path"` string AND the new `"id"`+`"cover"` (a file rewritten by
/// an intermediate build carries the new shape). Returns `None` for an entry
/// with neither — skipped rather than failing the whole import.
fn legacy_to_track(l: &LegacyTrack) -> Option<Track> {
    let (id, cover) = if let Some(id) = &l.id {
        (id.clone(), l.cover.clone())
    } else if let Some(path) = &l.path {
        (TrackId::from_legacy_path(path), split_legacy_cover(path))
    } else {
        return None;
    };
    Some(Track {
        id,
        cover,
        album_id: l.album_id.clone(),
        title: l.title.clone(),
        artist: l.artist.clone(),
        album: l.album.clone(),
        duration: l.duration,
        khz: l.khz,
        bitrate: l.bitrate,
        track_number: l.track_number,
        disc_number: l.disc_number,
        musicbrainz_release_id: l.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: l.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: l.musicbrainz_track_id.clone(),
        playlist_item_id: l.playlist_item_id.clone(),
        artists: l.artists.clone(),
        credits: Vec::new(),
    })
}

/// The cover smuggled as the legacy path's 3rd `:` segment (`service:id:cover`),
/// if any. Local paths and bare server ids have none.
fn split_legacy_cover(path: &str) -> Option<String> {
    for prefix in ["ytmusic", "jellyfin", "subsonic", "custom"] {
        if let Some(rest) = path.strip_prefix(prefix).and_then(|r| r.strip_prefix(':')) {
            return rest
                .split_once(':')
                .map(|(_, cover)| cover.to_string())
                .filter(|c| !c.is_empty());
        }
    }
    None
}

fn service_str(s: config::MusicService) -> &'static str {
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

fn str_at(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn opt_str_at(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn service_at(v: &serde_json::Value) -> String {
    let s = str_at(v, "service");
    if s.is_empty() {
        "Jellyfin".to_string()
    } else {
        s
    }
}

async fn db_has_data(pool: &SqlitePool) -> Result<bool, DbError> {
    let has_cfg: Option<i64> =
        sqlx::query_scalar!("SELECT 1 AS \"configured!: i64\" FROM app_state WHERE id = 1")
            .fetch_optional(pool)
            .await?;
    let ntracks: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM tracks")
        .fetch_one(pool)
        .await?;
    Ok(has_cfg.is_some() || ntracks > 0)
}

async fn count(pool: &SqlitePool, sql: &str) -> usize {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
        .max(0) as usize
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Read + parse one legacy file, tolerating damage: an unreadable or
/// unparseable file logs a warning and yields `None` (the caller imports its
/// default and leaves the file un-renamed for repair) instead of aborting the
/// whole import.
fn read_json_tolerant<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let s = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, file = %path.display(), "db: unreadable legacy json — skipping it");
            return None;
        }
    };
    match serde_json::from_str(&s) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(error = %e, file = %path.display(), "db: corrupt legacy json — skipping it (file left in place)");
            None
        }
    }
}

/// Rename `X.json` → `X.json.bak`. If a `.bak` already exists, the OLD one is
/// aged to `.bak.<unix>` so the plain `.bak` is always the freshest copy —
/// `legacy_source` only ever reads the plain `.bak`. Never deletes. Best-effort.
fn backup_aside(src: &Path) {
    let mut dst = src.as_os_str().to_os_string();
    dst.push(".bak");
    let dst = std::path::PathBuf::from(dst);
    if dst.exists() {
        let mut aged = dst.as_os_str().to_os_string();
        aged.push(format!(".{}", now_secs()));
        if let Err(e) = std::fs::rename(&dst, std::path::PathBuf::from(aged)) {
            tracing::warn!(error = %e, "db: could not age old .bak aside");
            return;
        }
    }
    if let Err(e) = std::fs::rename(src, &dst) {
        tracing::warn!(error = %e, src = %src.display(), "db: could not back up legacy json");
    }
}

// ---------------------------------------------------------------------------
// Legacy on-disk shapes (pre-#347). Only `Track` changed, but the containers
// embed it, so we mirror the lot to deserialize the old files faithfully.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LegacyTrack {
    /// Legacy form: the overloaded `"service:id[:cover]"` / filesystem path.
    #[serde(default)]
    path: Option<String>,
    /// New form (a file rewritten by an intermediate build): the typed id.
    #[serde(default)]
    id: Option<TrackId>,
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    album_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    album: String,
    #[serde(default)]
    duration: u64,
    #[serde(default)]
    khz: u32,
    #[serde(default)]
    bitrate: u16,
    #[serde(default)]
    track_number: Option<u32>,
    #[serde(default)]
    disc_number: Option<u32>,
    #[serde(default)]
    musicbrainz_release_id: Option<String>,
    #[serde(default)]
    musicbrainz_recording_id: Option<String>,
    #[serde(default)]
    musicbrainz_track_id: Option<String>,
    #[serde(default)]
    playlist_item_id: Option<String>,
    #[serde(default)]
    artists: Vec<String>,
}

#[derive(Deserialize)]
struct LegacyAlbum {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    genre: String,
    #[serde(default)]
    year: i64,
    #[serde(default)]
    cover_path: Option<String>,
    #[serde(default)]
    manual_cover: bool,
}

#[derive(Deserialize, Default)]
struct LegacyLibrary {
    #[serde(default)]
    tracks: Vec<LegacyTrack>,
    #[serde(default)]
    albums: Vec<LegacyAlbum>,
    #[serde(default)]
    jellyfin_tracks: Vec<LegacyTrack>,
    #[serde(default)]
    jellyfin_albums: Vec<LegacyAlbum>,
    #[serde(default)]
    last_yt_sync_at: Option<i64>,
    #[serde(default)]
    last_yt_playlists_sync_at: Option<i64>,
    #[serde(default)]
    server_artist_images: HashMap<String, String>,
    #[serde(default)]
    local_artist_images: HashMap<String, String>,
    #[serde(default)]
    custom_artist_images: HashMap<String, String>,
}

#[derive(Deserialize)]
struct LegacyPlaylist {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tracks: Vec<String>,
    #[serde(default)]
    cover_path: Option<String>,
}

#[derive(Deserialize)]
struct LegacyJellyfinPlaylist {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tracks: Vec<String>,
    #[serde(default)]
    image_tag: Option<String>,
    #[serde(default)]
    cover_path: Option<String>,
}

#[derive(Deserialize)]
struct LegacyFolder {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    playlist_ids: Vec<String>,
}

#[derive(Deserialize, Default)]
struct LegacyPlaylists {
    #[serde(default)]
    playlists: Vec<LegacyPlaylist>,
    #[serde(default)]
    jellyfin_playlists: Vec<LegacyJellyfinPlaylist>,
    #[serde(default)]
    folders: Vec<LegacyFolder>,
}

#[derive(Deserialize, Default)]
struct LegacyFavorites {
    #[serde(default)]
    local_favorites: Vec<String>,
    #[serde(default)]
    jellyfin_favorites: Vec<String>,
}

fn default_version() -> u32 {
    1
}

#[derive(Deserialize, Default)]
struct LegacyQueue {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    queue: Vec<LegacyTrack>,
    #[serde(default)]
    current_queue_index: i64,
    #[serde(default)]
    progress_secs: i64,
    #[serde(default)]
    shuffle_order: Vec<u32>,
    #[serde(default)]
    shuffle_enabled: bool,
}

#[cfg(test)]
mod eol_reconcile_tests {
    use super::*;
    use sha2::{Digest, Sha384};

    /// A DB created by a CRLF build (Windows) must open under this (LF) build,
    /// and the stored checksums get re-stamped to canonical — while a genuine
    /// migration edit is still rejected.
    #[tokio::test]
    async fn crlf_db_reconciles_but_real_edit_still_fails() {
        let dir = std::env::temp_dir().join(format!("kopuz-eol-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("t.db");
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(with_ext(&db, ext));
        }

        let pool = open_pool(&db).await.unwrap();
        run_migrations(&pool, None).await.unwrap();

        // Simulate a DB written by a CRLF build: every stored checksum becomes
        // the CRLF-variant hash of the same migration SQL.
        for m in MIGRATOR.iter() {
            let crlf = m.sql.replace("\r\n", "\n").replace('\n', "\r\n");
            sqlx::query("UPDATE _sqlx_migrations SET checksum = ?1 WHERE version = ?2")
                .bind(Sha384::digest(crlf.as_bytes()).to_vec())
                .bind(m.version)
                .execute(&pool)
                .await
                .unwrap();
        }

        // Reopen: the EOL-only mismatch must reconcile, not error.
        run_migrations(&pool, None)
            .await
            .expect("CRLF-only checksum mismatch should reconcile");

        // Checksums are now canonical (this binary's).
        let stored: Vec<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations")
                .fetch_all(&pool)
                .await
                .unwrap();
        for m in MIGRATOR.iter() {
            let ck = stored.iter().find(|(v, _)| *v == m.version).map(|(_, c)| c);
            assert_eq!(ck.unwrap().as_slice(), m.checksum.as_ref());
        }

        // A genuine modification (checksum matching neither line ending) must
        // still be refused.
        sqlx::query(
            "UPDATE _sqlx_migrations SET checksum = ?1 \
             WHERE version = (SELECT MIN(version) FROM _sqlx_migrations)",
        )
        .bind(vec![0u8; 48])
        .execute(&pool)
        .await
        .unwrap();
        assert!(
            run_migrations(&pool, None).await.is_err(),
            "a real migration edit must still be rejected"
        );

        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod artist_fill_tests {
    use super::*;

    /// A library stored before artist rows existed comes out with each name filed once per source, Unicode-folded.
    #[tokio::test]
    async fn stored_names_are_filed_as_artists_on_the_way_up() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let before = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                MIGRATOR
                    .iter()
                    .filter(|m| m.version < ARTISTS_CREATED)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        before.run(&pool).await.unwrap();
        sqlx::raw_sql(
            "INSERT INTO tracks (rowid_pk, source, track_key, source_album_id, title, artist, album, artists_json) VALUES \
               (1, 'local', '/a', 'al-1', 'a', 'Émilie feat. Ada', 'One', '[\"Émilie\", \" Ada \"]'), \
               (2, 'local', '/b', 'al-1', 'b', 'ÉMILIE', 'One', '[]'), \
               (3, 'local:x', 'b', 'al-2', 'c', 'Émilie', 'Two', '[\"Émilie\"]'); \
             INSERT INTO albums (source, source_album_id, title, artist) VALUES \
               ('local', 'al-1', 'One', 'émilie'), ('local', 'al-3', 'Three', '  ');",
        )
        .execute(&pool)
        .await
        .unwrap();

        run_migrations(&pool, None).await.unwrap();

        let artists: Vec<(String, String)> =
            sqlx::query_as("SELECT source, name FROM artists ORDER BY source, name")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            artists,
            [
                ("local".into(), "Ada".into()),
                ("local".into(), "Émilie".into()),
                ("local:x".into(), "Émilie".into()),
            ]
        );
        let credits: Vec<(i64, i64, String, String)> = sqlx::query_as(
            "SELECT c.track_pk, c.position, c.name, a.name FROM track_credits c \
               JOIN artists a ON a.id = c.artist_pk ORDER BY c.track_pk, c.position",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            credits,
            [
                (1, 0, "Émilie".into(), "Émilie".into()),
                (1, 1, "Ada".into(), "Ada".into()),
                (2, 0, "ÉMILIE".into(), "Émilie".into()),
                (3, 0, "Émilie".into(), "Émilie".into()),
            ]
        );
        let billed: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT al.source_album_id, a.name FROM albums al \
               LEFT JOIN artists a ON a.id = al.artist_pk ORDER BY al.source_album_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            billed,
            [
                ("al-1".into(), Some("Émilie".into())),
                ("al-2".into(), Some("Émilie".into())),
                ("al-3".into(), None),
            ]
        );
    }

    /// A derived album follows the artist its track credits now, a synced one keeps its billing, and a row nothing points at goes.
    #[tokio::test]
    async fn a_derived_album_follows_its_tracks_credits_on_the_way_up() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let before = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                MIGRATOR
                    .iter()
                    .filter(|m| m.version <= LYRICS_CACHE_DROPPED)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        before.run(&pool).await.unwrap();
        sqlx::raw_sql(
            "INSERT INTO artists (id, source, source_artist_id, name, name_key) VALUES \
               (1, 'srv', 'UC-ada', 'Ada', 'ada'), \
               (2, 'srv', NULL, 'Ada', 'ada'), \
               (3, 'srv', NULL, 'Boris', 'boris'); \
             INSERT INTO albums (source, source_album_id, title, artist, artist_pk, derived) VALUES \
               ('srv', 'al-1', 'One', 'Ada', 2, 1), \
               ('srv', 'al-2', 'Two', 'Boris', 3, 0); \
             INSERT INTO tracks (rowid_pk, source, track_key, source_album_id, title, artist, album) VALUES \
               (10, 'srv', 't1', 'al-1', 'a', 'Ada', 'One'), \
               (11, 'srv', 't2', 'al-2', 'b', 'Boris', 'Two'); \
             INSERT INTO track_credits (track_pk, position, artist_pk, name) VALUES \
               (10, 0, 1, 'Ada'), (11, 0, 1, 'Ada');",
        )
        .execute(&pool)
        .await
        .unwrap();

        run_migrations(&pool, None).await.unwrap();

        let billed: Vec<(String, i64)> = sqlx::query_as(
            "SELECT source_album_id, artist_pk FROM albums ORDER BY source_album_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(billed, [("al-1".into(), 1), ("al-2".into(), 3)]);
        let rows: Vec<i64> = sqlx::query_scalar("SELECT id FROM artists ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(
            rows,
            [1, 3],
            "the unlinked Ada nothing points at any more is gone"
        );
    }
}

#[cfg(test)]
mod row_fill_tests {
    use super::*;

    fn queued(key: &str) -> Track {
        Track {
            id: TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: key.into(),
            },
            cover: None,
            album_id: String::new(),
            title: key.into(),
            artist: "Ada".into(),
            album: String::new(),
            duration: u64::MAX,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec!["Ada".into()],
            credits: vec![reader::ArtistCredit::linked("Ada", "UC-ada")],
        }
    }

    /// Every document the blob and kv held comes out as rows, and the documents are gone.
    #[tokio::test]
    async fn the_stored_documents_become_rows_on_the_way_up() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool, None).await.unwrap();
        // Back to just before the new tables, with every document an older build left.
        for sql in [
            "DROP TABLE queue_shuffle",
            "DROP TABLE queue_credits",
            "DROP TABLE queue_tracks",
            "DROP TABLE offline_tracks",
            "DROP TABLE pinned_stations",
            "DROP TABLE lyric_chunks",
            "DROP TABLE lyric_lines",
            "DROP TABLE lyrics",
            "DROP TABLE app_state",
            "DROP TABLE view_sorts",
            "DROP TABLE sidebar_items",
            "DROP TABLE home_sections",
            "DROP TABLE integration_credentials",
            "DROP TABLE ytdlp_history",
            "DROP TABLE server_folders",
            "CREATE TABLE app_config (id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL)",
            "DROP TABLE queue_state",
            "CREATE TABLE queue_state (id INTEGER PRIMARY KEY CHECK (id = 1), version INTEGER NOT NULL DEFAULT 1, current_queue_index INTEGER NOT NULL DEFAULT 0, progress_secs INTEGER NOT NULL DEFAULT 0, shuffle_enabled INTEGER NOT NULL DEFAULT 0)",
            "ALTER TABLE queue_state ADD COLUMN queue_json TEXT NOT NULL DEFAULT '[]'",
            "ALTER TABLE queue_state ADD COLUMN shuffle_order_json TEXT NOT NULL DEFAULT '[]'",
            "DROP TABLE artist_images",
            "CREATE TABLE artist_images (artist_norm TEXT NOT NULL, kind TEXT NOT NULL, image_ref TEXT NOT NULL, PRIMARY KEY (artist_norm, kind))",
            "DROP VIEW artist_cover_albums",
            "DROP VIEW artist_credit_rows",
            "DROP TRIGGER artists_key_required",
            "DROP INDEX idx_artists_key",
            "ALTER TABLE artists DROP COLUMN key",
            "ALTER TABLE artists DROP COLUMN named_by_source",
            "DELETE FROM _sqlx_migrations WHERE version >= 20260930000000",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let queue = serde_json::to_string(&[queued("a"), queued("b")]).unwrap();
        sqlx::query(
            "INSERT INTO queue_state (id, queue_json, current_queue_index, shuffle_order_json) \
             VALUES (1, ?1, 1, '[1, 0]')",
        )
        .bind(queue)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(
            r#"INSERT INTO app_config (id, json) VALUES (1, '{"theme":"dark","volume":0.3,"active_source":{"Server":"yt-1"},"album_sort":[{"field":"Year","direction":"Desc"}],"lastfm_session_key":"sk","offline_tracks":{"t1":"/c/t1.flac"},"pinned_stations":["{\"id\":\"moe\"}", "not json"]}');
               INSERT INTO servers (id, name, url, service, updated_at) VALUES ('yt-1', 'yt', '', 'YtMusic', 0);
               INSERT INTO kv (name, kind, value) VALUES
                 ('yt_sync', 'timestamps', '{"last_yt_sync_at":1700000000,"last_yt_playlists_sync_at":null}'),
                 ('legacy_import', 'files', '["config.json","library.json"]'),
                 ('k1', 'lyrics', '{"kind":"synced2","lines":[{"start_time":1.5,"text":"hi","chunks":[{"start_time":1.5,"text":"h"}],"background":true}]}'),
                 ('k2', 'lyrics', '{"kind":"plain","text":"words"}'),
                 ('k3', 'lyrics', '{"kind":"none","ts":42}'),
                 ('k4', 'lyrics', '{"kind":"synced","lines":[]}');"#,
        )
        .execute(&pool)
        .await
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.toml");
        run_migrations(&pool, Some(&settings)).await.unwrap();

        let queue: Vec<(i64, String, Option<i64>)> = sqlx::query_as(
            "SELECT position, track_key, duration FROM queue_tracks ORDER BY position",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(queue, [(0, "a".into(), None), (1, "b".into(), None)]);
        let credit: (String, Option<String>) = sqlx::query_as(
            "SELECT name, source_artist_id FROM queue_credits WHERE queue_position = 0",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(credit, ("Ada".into(), Some("UC-ada".into())));
        let shuffle: Vec<i64> =
            sqlx::query_scalar("SELECT position FROM queue_shuffle ORDER BY step")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(shuffle, [1, 0]);

        let offline: Vec<(String, String)> =
            sqlx::query_as("SELECT item_id, path FROM offline_tracks")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(offline, [("t1".into(), "/c/t1.flac".into())]);
        let pins: Vec<String> = sqlx::query_scalar("SELECT id FROM pinned_stations")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(pins, ["moe"], "an unreadable pin is dropped");
        let state: (String, f64) =
            sqlx::query_as("SELECT active_source, volume FROM app_state WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            state,
            ("yt-1".into(), f64::from(0.3f32)),
            "the blob's state is rows now"
        );
        let sort: (String, String, String) =
            sqlx::query_as("SELECT view, field, direction FROM view_sorts")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sort, ("albums".into(), "Year".into(), "Desc".into()));
        let secret: String = sqlx::query_scalar(
            "SELECT value FROM integration_credentials WHERE key = 'lastfm_session_key'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(secret, "sk");
        let written: toml::Table = std::fs::read_to_string(&settings)
            .expect("the blob's settings went to the file")
            .parse()
            .unwrap();
        assert_eq!(written["theme"].as_str(), Some("dark"));
        assert!(!written.contains_key("volume") && !written.contains_key("lastfm_session_key"));

        let kv: Vec<(String, String, String)> =
            sqlx::query_as("SELECT name, kind, value FROM kv ORDER BY kind, name")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            kv,
            [
                ("config.json".into(), "legacy_import".into(), String::new()),
                ("library.json".into(), "legacy_import".into(), String::new()),
                (
                    "synced:favorites".into(),
                    "yt-1".into(),
                    "1700000000".into()
                ),
            ]
        );

        let lyrics: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT cache_key, kind, plain_text FROM lyrics ORDER BY cache_key")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            lyrics,
            [
                ("k1".into(), "synced".into(), None),
                ("k2".into(), "plain".into(), Some("words".into())),
                ("k3".into(), "none".into(), None),
            ],
            "an entry in a format no build reads any more is dropped"
        );
        let line: (f64, String, i64) = sqlx::query_as(
            "SELECT start_time, text, background FROM lyric_lines WHERE cache_key = 'k1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(line, (1.5, "hi".into(), 1));
        let chunk: String =
            sqlx::query_scalar("SELECT text FROM lyric_chunks WHERE cache_key = 'k1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(chunk, "h");
        let missed_at: i64 =
            sqlx::query_scalar("SELECT fetched_at FROM lyrics WHERE cache_key = 'k3'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(missed_at, 42);
    }
}
