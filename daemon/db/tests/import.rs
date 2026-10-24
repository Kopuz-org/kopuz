//! Legacy-JSON → SQLite importer tests (issue #347, step 3).
//!
//! `imports_synthetic_fixture` runs in CI against hand-built fixtures. `smoke_real`
//! is `#[ignore]` and imports a copy of a real `~/.config/kopuz` when
//! `KOPUZ_IMPORT_DIR` points at one — handy for validating against live data
//! without committing it.

use std::path::{Path, PathBuf};

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Row, SqliteConnection};

fn unique_dir(tag: &str) -> PathBuf {
    // pid + counter, not just clock: macOS's µs clock let parallel tests
    // collide on a nanos-only name and delete each other's live DB.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("kopuz-import-{tag}-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn open(db_path: &Path) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap()
}

#[tokio::test]
async fn imports_synthetic_fixture() {
    let dir = unique_dir("synthetic");

    std::fs::write(
        dir.join("config.json"),
        r#"{
            "theme": "dark",
            "volume": 0.8,
            "discord_presence_paused": false,
            "hero_height": 410,
            "album_sort": [{ "field": "Year", "direction": "Desc" }],
            "active_source": "Server",
            "listen_counts": { "ytmusic:VID1:urlhex_deadbeef": 5, "/music/a.flac": 2 },
            "server": {
                "id": "srv-1", "name": "yt", "url": "https://music.youtube.com",
                "service": "YtMusic", "access_token": "SECRET_COOKIE", "yt_anonymous": false
            },
            "servers": [
                { "id": "srv-1", "name": "yt", "url": "https://music.youtube.com", "service": "YtMusic" }
            ]
        }"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("library.json"),
        r#"{
            "root_paths": ["/music"],
            "tracks": [
                { "path": "/music/a.flac", "album_id": "alb-local", "title": "A", "artist": "Loc",
                  "album": "L", "duration": 100, "khz": 44100, "bitrate": 900, "track_number": 1,
                  "disc_number": null, "artists": ["Loc"] }
            ],
            "albums": [
                { "id": "alb-local", "title": "L", "artist": "Loc", "genre": "Rock", "year": 2020,
                  "cover_path": "/cache/l.png", "manual_cover": false }
            ],
            "jellyfin_tracks": [
                { "path": "ytmusic:VID1:urlhex_68747470733a2f2f78", "album_id": "ytmusic:album:AL1",
                  "title": "Yt One", "artist": "Art", "album": "YA", "duration": 200, "khz": 0,
                  "bitrate": 0, "track_number": null, "disc_number": null, "artists": ["Art"] }
            ],
            "jellyfin_albums": [
                { "id": "ytmusic:album:AL1", "title": "YA", "artist": "Art", "genre": "", "year": 0,
                  "cover_path": null, "manual_cover": false }
            ],
            "last_yt_sync_at": 1700000000,
            "server_artist_images": { "art": "https://img/art.jpg" }
        }"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("playlists.json"),
        r#"{
            "playlists": [
                { "id": "pl-1", "name": "Mine", "tracks": ["/music/a.flac"], "cover_path": null }
            ],
            "jellyfin_playlists": [
                { "id": "LM", "name": "Liked Songs", "tracks": ["VID1"], "image_tag": "urlhex_ab" }
            ],
            "folders": []
        }"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("favorites.json"),
        r#"{ "local_favorites": ["/music/a.flac"], "jellyfin_favorites": ["VID1", "VID9"] }"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("queue_state.json"),
        r#"{
            "version": 1,
            "queue": [
                { "path": "ytmusic:VID1:urlhex_68747470733a2f2f78", "album_id": "ytmusic:album:AL1",
                  "title": "Yt One", "artist": "Art", "album": "YA", "duration": 200, "khz": 0,
                  "bitrate": 0, "track_number": null, "disc_number": null, "artists": ["Art"] }
            ],
            "current_queue_index": 0,
            "progress_secs": 42,
            "shuffle_order": [],
            "shuffle_enabled": false
        }"#,
    )
    .unwrap();

    let db_path = dir.join("kopuz.db");
    let db = db::init(&db_path).await.unwrap();
    let report = db.import_legacy_json(&dir).await.unwrap();

    assert!(report.ran, "import should have run");
    assert_eq!(report.tracks, 2, "1 local + 1 server track");
    assert_eq!(report.albums, 2);
    assert_eq!(report.playlists, 2);
    assert_eq!(report.favorites, 3, "1 local + 2 server favorites");
    assert_eq!(report.servers, 1);

    // Import leaves the JSONs in place; only finalize moves them aside.
    assert!(dir.join("config.json").exists());
    assert!(!dir.join("config.json.bak").exists());

    let mut conn = open(&db_path).await;

    // Server track: key is the bare id, cover lifted out of the path, service set.
    let row = sqlx::query(
        "SELECT source, track_key, path, service, cover_path FROM tracks WHERE title = 'Yt One'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("source"), "srv-1");
    assert_eq!(row.get::<String, _>("track_key"), "VID1");
    assert_eq!(row.get::<Option<String>, _>("path"), None);
    assert_eq!(row.get::<String, _>("service"), "YtMusic");
    assert_eq!(
        row.get::<Option<String>, _>("cover_path").as_deref(),
        Some("urlhex_68747470733a2f2f78")
    );

    // Local track: path preserved, no service, key is the path.
    let row = sqlx::query("SELECT source, track_key, path, service FROM tracks WHERE title = 'A'")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("source"), "local");
    assert_eq!(row.get::<String, _>("track_key"), "/music/a.flac");
    assert_eq!(
        row.get::<Option<String>, _>("path").as_deref(),
        Some("/music/a.flac")
    );
    assert_eq!(row.get::<Option<String>, _>("service"), None);

    // Creds landed beside the server row (not in the config blob).
    let token: String =
        sqlx::query_scalar("SELECT access_token FROM server_credentials WHERE server_id = 'srv-1'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(token, "SECRET_COOKIE");

    // The legacy config lands as state rows and a settings file that carries no creds.
    let active: String = sqlx::query_scalar("SELECT active_source FROM app_state WHERE id = 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(active, "srv-1");
    assert_eq!(db.playback_state().await.unwrap(), (0.8, false));
    assert_eq!(db.discord_presence_paused().await.unwrap(), Some(false));
    let settings = std::fs::read_to_string(config::store::settings_path_for(&dir)).unwrap();
    assert!(
        !settings.contains("SECRET_COOKIE"),
        "no token leaked into the settings file"
    );

    // Its UI state seeds both frontends' prefs alike.
    for frontend in ["kopuz", "gpui"] {
        let prefs: std::collections::HashMap<String, String> = db
            .frontend_prefs(frontend)
            .await
            .unwrap()
            .into_iter()
            .collect();
        let ui = config::UiPrefs::from_entries(prefs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        assert_eq!(ui.hero_height, 410, "{frontend}");
        assert_eq!(
            ui.album_sort,
            [config::SortCriterion::new(
                config::AlbumSortField::Year,
                config::SortDirection::Desc
            )],
            "{frontend}"
        );
        assert_eq!(prefs["hero_height"], "410");
    }

    // The YT sync time becomes the YT server's favorites stamp, or its first open would re-stream the liked library.
    let stamp: Option<String> = sqlx::query_scalar(
        "SELECT value FROM kv WHERE name = 'synced:favorites' AND kind = 'srv-1'",
    )
    .fetch_optional(&mut conn)
    .await
    .unwrap();
    assert_eq!(stamp.as_deref(), Some("1700000000"));

    // listen_counts keyed by source and track (cover dropped from the legacy key).
    let c: i64 = sqlx::query_scalar(
        "SELECT count FROM listen_counts WHERE source = 'srv-1' AND track_key = 'VID1'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(c, 5);

    // Liked-songs playlist membership preserved.
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM playlist_tracks pt JOIN playlists p ON p.rowid_pk = pt.playlist_pk \
         WHERE p.source_pl_id = 'LM'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(n, 1);

    // Re-running is a no-op (this DB already has data).
    let again = db.import_legacy_json(&dir).await.unwrap();
    assert!(!again.ran);

    // Finalize renames the JSONs aside (kept as .bak for downgrade).
    let renamed = db.finalize_migration(&dir).await.unwrap();
    assert_eq!(renamed, 5);
    assert!(!dir.join("config.json").exists());
    assert!(dir.join("config.json.bak").exists());

    // The install imported once; a second database (the debug/release split) does not import the .bak copies again.
    let db2 = db::init(&dir.join("kopuz-second.db")).await.unwrap();
    assert!(!db2.import_legacy_json(&dir).await.unwrap().ran);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore = "set KOPUZ_IMPORT_DIR to a copy of a real ~/.config/kopuz"]
async fn smoke_real() {
    let Ok(src) = std::env::var("KOPUZ_IMPORT_DIR") else {
        return;
    };
    let dir = unique_dir("real");
    for entry in std::fs::read_dir(&src).unwrap().flatten() {
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
        }
    }
    let db_path = dir.join("kopuz.db");
    let db = db::init(&db_path).await.unwrap();
    let report = db.import_legacy_json(&dir).await.unwrap();
    tracing::info!("real import report: {report:?}");
    assert!(report.ran);

    let settings =
        std::fs::read_to_string(config::store::settings_path_for(&dir)).unwrap_or_default();
    assert!(
        !settings.contains("access_token") && !settings.contains("APISID"),
        "no creds in the settings file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn corrupt_file_skipped_rest_imports_and_finalize_leaves_it() {
    let dir = unique_dir("corrupt");

    // Truncated queue (the power-loss case) + valid favorites + valid library.
    std::fs::write(dir.join("queue_state.json"), r#"{"queue": [{"path": "/m"#).unwrap();
    std::fs::write(
        dir.join("favorites.json"),
        r#"{ "local_favorites": ["/music/a.flac"] }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("library.json"),
        r#"{ "tracks": [
            { "path": "/music/a.flac", "album_id": "alb", "title": "A", "artist": "X",
              "album": "L", "duration": 1, "khz": 1, "bitrate": 1, "artists": ["X"] }
        ] }"#,
    )
    .unwrap();

    let db_path = dir.join("kopuz.db");
    let db = db::init(&db_path).await.unwrap();
    let report = db.import_legacy_json(&dir).await.unwrap();
    assert!(report.ran, "a corrupt file must not abort the import");
    assert_eq!(report.tracks, 1);
    assert_eq!(report.favorites, 1);

    // Finalize renames only what was consumed; the corrupt file stays put.
    let renamed = db.finalize_migration(&dir).await.unwrap();
    assert_eq!(renamed, 2);
    assert!(dir.join("library.json.bak").exists());
    assert!(dir.join("favorites.json.bak").exists());
    assert!(
        dir.join("queue_state.json").exists(),
        "corrupt file left in place for repair"
    );
    assert!(!dir.join("queue_state.json.bak").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn finalize_is_inert_when_no_import_ran() {
    let dir = unique_dir("noimport");
    std::fs::write(dir.join("config.json"), r#"{ "theme": "dark" }"#).unwrap();

    let db_path = dir.join("kopuz.db");
    let db = db::init(&db_path).await.unwrap();
    // Simulate "import failed, runtime wrote data anyway": no import, but the
    // DB becomes non-empty.
    db.ensure_app_state().await.unwrap();

    let renamed = db.finalize_migration(&dir).await.unwrap();
    assert_eq!(
        renamed, 0,
        "finalize must not rename files no import consumed"
    );
    assert!(dir.join("config.json").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

/// A database a reset or a corruption recreated is empty again, and the legacy files are still there; it must not take them in a second time.
#[tokio::test]
async fn a_recreated_database_does_not_import_again() {
    let dir = unique_dir("recreated");
    std::fs::write(
        dir.join("library.json"),
        r#"{ "tracks": [
            { "path": "/music/a.flac", "album_id": "alb", "title": "A", "artist": "X",
              "album": "L", "duration": 1, "khz": 1, "bitrate": 1, "artists": ["X"] }
        ] }"#,
    )
    .unwrap();
    let db_path = dir.join("kopuz.db");
    let db = db::init(&db_path).await.unwrap();
    assert!(db.import_legacy_json(&dir).await.unwrap().ran);
    drop(db);
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("kopuz.db{ext}")));
    }

    let recreated = db::init(&db_path).await.unwrap();
    assert!(!recreated.import_legacy_json(&dir).await.unwrap().ran);
    let mut conn = open(&db_path).await;
    let tracks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tracks")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(tracks, 0);

    let _ = std::fs::remove_dir_all(&dir);
}

/// An install that imported before the marker existed is marked the first time it boots with its database in use.
#[tokio::test]
async fn a_database_in_use_marks_the_install_imported() {
    let dir = unique_dir("premarker");
    std::fs::write(dir.join("config.json"), r#"{ "theme": "dark" }"#).unwrap();
    let db_path = dir.join("kopuz.db");
    let db = db::init(&db_path).await.unwrap();
    db.ensure_app_state().await.unwrap();
    assert!(!db.import_legacy_json(&dir).await.unwrap().ran);
    drop(db);
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("kopuz.db{ext}")));
    }

    let recreated = db::init(&db_path).await.unwrap();
    assert!(!recreated.import_legacy_json(&dir).await.unwrap().ran);

    let _ = std::fs::remove_dir_all(&dir);
}
