//! One-shot importer: legacy `*.json` store → SQLite (issue #347).
//!
//! Runs once per database (gated on the DB being empty), so it's safe to call
//! on every launch. The whole import is a single transaction — a crash before
//! commit leaves the DB empty and the JSONs untouched, so it re-runs cleanly.
//! A file that fails to parse is skipped (and left in place for repair); the
//! rest import normally. The names actually consumed are recorded in
//! `metadata_cache`, and [`finalize_migration`] renames exactly those files to
//! `*.json.bak` (kept for downgrade; never deleted).
//!
//! Legacy `Track.path` was the overloaded `"service:id[:cover]"` string; we
//! parse it here (the one place, via [`TrackId::from_legacy_path`]) into the
//! typed id, lifting the smuggled cover out of the 3rd segment.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{DbError, ImportReport};
use reader::models::{Track, TrackId};

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
    import_config_blob(&mut tx, &cfg_val, &active_server_id, &lib).await?;
    import_listen_counts(&mut tx, &cfg_val).await?;

    // The YT sync timestamps ALSO go to the metadata cache — that's where the
    // runtime reads them ("yt_sync"/"timestamps"); blob keys alone would make
    // the favorites page think it never synced and re-stream the whole liked
    // library from YT on first open after a migration.
    if lib.last_yt_sync_at.is_some() || lib.last_yt_playlists_sync_at.is_some() {
        let stamps = serde_json::json!({
            "last_yt_sync_at": lib.last_yt_sync_at,
            "last_yt_playlists_sync_at": lib.last_yt_playlists_sync_at,
        })
        .to_string();
        sqlx::query!(
            "INSERT INTO metadata_cache (cache_key, kind, payload) VALUES ('yt_sync', 'timestamps', ?1) \
             ON CONFLICT(cache_key, kind) DO UPDATE SET payload = ?1",
            stamps
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
    import_artist_images(&mut tx, "server", &lib.server_artist_images).await?;
    import_artist_images(&mut tx, "local", &lib.local_artist_images).await?;
    import_artist_images(&mut tx, "custom", &lib.custom_artist_images).await?;

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
                "INSERT INTO metadata_cache (cache_key, kind, payload) VALUES ('fav_pull', ?1, ?2) \
                 ON CONFLICT(cache_key, kind) DO UPDATE SET payload = ?2",
                sid,
                now_s
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    // --- queue snapshot ----------------------------------------------------
    let queue_tracks: Vec<Track> = queue.queue.iter().filter_map(legacy_to_track).collect();
    let queue_json = serde_json::to_string(&queue_tracks)?;
    let shuffle_json = serde_json::to_string(&queue.shuffle_order)?;
    let cqi = queue.current_queue_index;
    let prog = queue.progress_secs;
    let shuffle_on = queue.shuffle_enabled as i64;
    let ver = queue.version as i64;
    sqlx::query!(
        "INSERT INTO queue_state \
           (id, version, queue_json, current_queue_index, progress_secs, shuffle_order_json, shuffle_enabled) \
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(id) DO UPDATE SET version=?1, queue_json=?2, current_queue_index=?3, \
           progress_secs=?4, shuffle_order_json=?5, shuffle_enabled=?6",
        ver,
        queue_json,
        cqi,
        prog,
        shuffle_json,
        shuffle_on
    )
    .execute(&mut *tx)
    .await?;

    // Record what this import actually consumed — finalize renames exactly
    // these files, so a skipped corrupt file is never moved aside unimported.
    let consumed_json = serde_json::to_string(&consumed)?;
    sqlx::query!(
        "INSERT INTO metadata_cache (cache_key, kind, payload) VALUES ('legacy_import', 'files', ?1) \
         ON CONFLICT(cache_key, kind) DO UPDATE SET payload = ?1",
        consumed_json
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

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
    let consumed: Option<String> = sqlx::query_scalar!(
        "SELECT payload FROM metadata_cache WHERE cache_key = 'legacy_import' AND kind = 'files'"
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    let Some(consumed) = consumed else {
        return Ok(0);
    };
    let consumed: Vec<String> = serde_json::from_str(&consumed).unwrap_or_default();
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
                .unwrap_or(false) as i64;
            sqlx::query!(
                "INSERT OR IGNORE INTO servers (id, name, url, service, yt_browser, yt_anonymous) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                id,
                name,
                url,
                service,
                yt_browser,
                yt_anon
            )
            .execute(&mut **tx)
            .await?;
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
        .unwrap_or(false) as i64;

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

    let auth_state = if token.is_some() || yt_anon == 1 {
        "active"
    } else {
        "unauthenticated"
    };
    sqlx::query!(
        "INSERT INTO servers \
           (id, name, url, service, access_token, user_id, yt_browser, yt_anonymous, auth_state, cred_updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
         ON CONFLICT(id) DO UPDATE SET name=?2, url=?3, service=?4, access_token=?5, user_id=?6, \
           yt_browser=?7, yt_anonymous=?8, auth_state=?9, cred_updated_at=?10",
        resolved,
        name,
        url,
        service,
        token,
        user_id,
        yt_browser,
        yt_anon,
        auth_state,
        now
    )
    .execute(&mut **tx)
    .await?;

    Ok(Some(resolved))
}

/// Store the config JSON blob, stripped of `server`/`servers`/`listen_counts`
/// (now their own tables) and stamped with `active_server_id` + the YT sync
/// timestamps carried over from `library.json`.
async fn import_config_blob(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    cfg: &serde_json::Value,
    active_server_id: &Option<String>,
    lib: &LegacyLibrary,
) -> Result<(), DbError> {
    let mut blob = if cfg.is_null() {
        serde_json::json!({})
    } else {
        cfg.clone()
    };
    if let Some(obj) = blob.as_object_mut() {
        obj.remove("server");
        obj.remove("servers");
        obj.remove("listen_counts");
        obj.insert(
            "active_server_id".into(),
            serde_json::json!(active_server_id),
        );
        obj.insert(
            "last_yt_sync_at".into(),
            serde_json::json!(lib.last_yt_sync_at),
        );
        obj.insert(
            "last_yt_playlists_sync_at".into(),
            serde_json::json!(lib.last_yt_playlists_sync_at),
        );
    }
    let blob_str = serde_json::to_string(&blob)?;
    sqlx::query!(
        "INSERT INTO app_config (id, json) VALUES (1, ?1) \
         ON CONFLICT(id) DO UPDATE SET json = ?1",
        blob_str
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// `listen_counts` map → its own table, keyed by the source-qualified id
/// ([`TrackId::uid`]) the runtime looks up by (legacy keys carried the cover).
async fn import_listen_counts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    cfg: &serde_json::Value,
) -> Result<(), DbError> {
    let Some(map) = cfg.get("listen_counts").and_then(|v| v.as_object()) else {
        return Ok(());
    };
    for (k, v) in map {
        let key = TrackId::from_legacy_path(k).uid();
        let count = v.as_i64().unwrap_or(0);
        // Accumulate: distinct legacy keys can collapse to one uid (the old
        // "service:id:cover" form re-keyed when a cover changed).
        sqlx::query!(
            "INSERT INTO listen_counts (track_key, count) VALUES (?1, ?2) \
             ON CONFLICT(track_key) DO UPDATE SET count = count + ?2",
            key,
            count
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn import_artist_images(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    kind: &str,
    map: &HashMap<String, String>,
) -> Result<(), DbError> {
    for (artist, image) in map {
        sqlx::query!(
            "INSERT OR IGNORE INTO artist_images (artist_norm, kind, image_ref) VALUES (?1, ?2, ?3)",
            artist,
            kind,
            image
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn insert_album(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source: &str,
    a: &LegacyAlbum,
) -> Result<(), DbError> {
    let manual = a.manual_cover as i64;
    sqlx::query!(
        "INSERT OR IGNORE INTO albums \
           (source, source_album_id, title, artist, genre, year, cover_path, manual_cover) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        source,
        a.id,
        a.title,
        a.artist,
        a.genre,
        a.year,
        a.cover_path,
        manual
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
    let artists_json = serde_json::to_string(&t.artists)?;
    sqlx::query!(
        "INSERT OR IGNORE INTO tracks \
           (source, track_key, path, service, source_album_id, title, artist, album, duration, \
            khz, bitrate, track_number, disc_number, mb_release_id, mb_recording_id, mb_track_id, \
            playlist_item_id, artists_json, cover_path) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
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
        t.musicbrainz_release_id,
        t.musicbrainz_recording_id,
        t.musicbrainz_track_id,
        t.playlist_item_id,
        artists_json,
        t.cover
    )
    .execute(&mut **tx)
    .await?;
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
    let has_cfg: Option<i64> = sqlx::query_scalar!("SELECT 1 FROM app_config WHERE id = 1")
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
