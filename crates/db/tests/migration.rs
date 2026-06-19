//! Legacy JSON → SQLite migration (issue #347). Exercises `run_json_import`
//! (via `Db::import_legacy_json`) against a real on-disk legacy `config_dir` and
//! a fresh in-migration DB (`db::init` applies every migration). Each domain is
//! asserted independently — the importer had no test coverage, and a missing
//! domain (recently-played) shipped silently.

use std::path::PathBuf;

fn unique_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-migrate-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A legacy `config.json` with two saved servers, an active YT server, and both
/// recently-played lists (local paths + server item ids), newest-first.
const LEGACY_CONFIG: &str = r#"{
  "servers": [
    {"id":"s1","name":"Yt","url":"https://music.youtube.com","service":"YtMusic"},
    {"id":"s2","name":"Jelly","url":"https://jelly.lan","service":"Jellyfin"}
  ],
  "server": {"id":"s1","name":"Yt","url":"https://music.youtube.com","service":"YtMusic","access_token":"tok"},
  "active_source": "Server",
  "active_server_id": "s1",
  "recently_played": ["/m/a.flac", "/m/b.flac"],
  "recently_played_server": ["VID1", "VID2", "VID3"]
}"#;

/// Fresh DB + a `config_dir` holding `config.json`, then run the importer.
async fn import(config_json: &str) -> (db::Db, PathBuf) {
    let dir = unique_dir();
    std::fs::write(dir.join("config.json"), config_json).unwrap();
    let db = db::init(&dir.join("kopuz.db")).await.unwrap();
    db.import_legacy_json(&dir).await.unwrap();
    (db, dir)
}

#[tokio::test]
async fn migration_imports_recently_played_per_source() {
    let (db, dir) = import(LEGACY_CONFIG).await;

    // Local list → the local partition, newest-first order preserved.
    assert_eq!(
        db.recently_played(&db::Source::Local, 50).await.unwrap(),
        vec!["/m/a.flac", "/m/b.flac"]
    );
    // Server list → the active server's partition.
    assert_eq!(
        db.recently_played(&db::Source::Server("s1".into()), 50)
            .await
            .unwrap(),
        vec!["VID1", "VID2", "VID3"]
    );
    // A non-active server gets none (the legacy list was the active server's).
    assert!(
        db.recently_played(&db::Source::Server("s2".into()), 50)
            .await
            .unwrap()
            .is_empty()
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn migration_is_idempotent() {
    let (db, dir) = import(LEGACY_CONFIG).await;
    // A second run is gated off (the DB now has data) — no duplicated rows.
    db.import_legacy_json(&dir).await.unwrap();
    assert_eq!(
        db.recently_played(&db::Source::Server("s1".into()), 50)
            .await
            .unwrap(),
        vec!["VID1", "VID2", "VID3"]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn migration_imports_servers_and_active_source() {
    let (db, dir) = import(LEGACY_CONFIG).await;
    let cfg = db.load_config().await.unwrap().expect("config present");

    assert_eq!(cfg.servers.len(), 2);
    assert_eq!(cfg.active_source.server_id(), Some("s1"));
    assert_eq!(
        cfg.server.as_ref().and_then(|s| s.access_token.as_deref()),
        Some("tok")
    );

    let _ = std::fs::remove_dir_all(dir);
}
