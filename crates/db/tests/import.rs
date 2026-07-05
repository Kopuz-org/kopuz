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

    // Creds landed on the server row (not in the config blob).
    let row = sqlx::query("SELECT access_token, auth_state FROM servers WHERE id = 'srv-1'")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        row.get::<Option<String>, _>("access_token").as_deref(),
        Some("SECRET_COOKIE")
    );
    assert_eq!(row.get::<String, _>("auth_state"), "active");

    // Config blob: creds/servers/listen_counts stripped, active_server_id stamped.
    let blob: String = sqlx::query_scalar("SELECT json FROM app_config WHERE id = 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&blob).unwrap();
    assert_eq!(
        v.get("active_source")
            .and_then(|s| s.get("Server"))
            .and_then(|x| x.as_str()),
        Some("srv-1")
    );
    assert!(
        v.get("server").is_none(),
        "creds must not remain in the blob"
    );
    assert!(v.get("servers").is_none());
    assert!(v.get("listen_counts").is_none());
    assert!(
        !blob.contains("SECRET_COOKIE"),
        "no token leaked into the blob"
    );

    // YT sync stamps land in the metadata cache (where the runtime reads them —
    // blob-only stamps caused a full YT re-stream on first favorites open).
    let stamp: Option<String> = sqlx::query_scalar(
        "SELECT payload FROM metadata_cache WHERE cache_key = 'yt_sync' AND kind = 'timestamps'",
    )
    .fetch_optional(&mut conn)
    .await
    .unwrap();
    let stamp: serde_json::Value = serde_json::from_str(&stamp.expect("yt_sync stamp")).unwrap();
    assert_eq!(
        stamp.get("last_yt_sync_at").and_then(|v| v.as_u64()),
        Some(1_700_000_000)
    );

    // listen_counts keyed by uid (cover dropped from the legacy key).
    let c: i64 =
        sqlx::query_scalar("SELECT count FROM listen_counts WHERE track_key = 'ytmusic:VID1'")
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

    // A SECOND database (the debug/release split) finds only the .bak files and
    // still imports — the gate is per-DB emptiness, not a shared sentinel.
    let db2_path = dir.join("kopuz-second.db");
    let db2 = db::init(&db2_path).await.unwrap();
    let second = db2.import_legacy_json(&dir).await.unwrap();
    assert!(second.ran, "second DB must import from the .bak files");
    assert_eq!(second.tracks, 2);
    assert_eq!(second.favorites, 3);

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

    let mut conn = open(&db_path).await;
    let leaked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM app_config WHERE json LIKE '%access_token%' OR json LIKE '%APISID%'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(leaked, 0, "no creds in the config blob");

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
    db.save_config(&config::AppConfig::default()).await.unwrap();

    let renamed = db.finalize_migration(&dir).await.unwrap();
    assert_eq!(
        renamed, 0,
        "finalize must not rename files no import consumed"
    );
    assert!(dir.join("config.json").exists());

    let _ = std::fs::remove_dir_all(&dir);
}
