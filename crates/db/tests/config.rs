//! Config persistence round-trip (issue #347, step 4): the in-memory `AppConfig`
//! survives save→load, creds live in the `servers` table (never the blob), and
//! play counts live in `listen_counts`.

use std::path::PathBuf;

use config::{AppConfig, MusicServer, MusicService, SavedServer};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqliteConnection};

fn unique_db() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-cfg-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

#[tokio::test]
async fn config_round_trips_with_creds_in_servers_table() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let mut cfg = AppConfig::default();
    cfg.servers = vec![
        SavedServer {
            id: "srv-a".into(),
            name: "Jelly".into(),
            url: "https://jelly.example".into(),
            service: MusicService::Jellyfin,
            yt_browser: None,
            yt_anonymous: false,
        },
        SavedServer {
            id: "srv-b".into(),
            name: "Yt".into(),
            url: "https://music.youtube.com".into(),
            service: MusicService::YtMusic,
            yt_browser: Some(config::Browser::Brave),
            yt_anonymous: false,
        },
    ];
    cfg.server = Some(MusicServer {
        name: "Yt".into(),
        url: "https://music.youtube.com".into(),
        service: MusicService::YtMusic,
        access_token: Some("TOPSECRET_COOKIE".into()),
        user_id: Some("u-1".into()),
        id: Some("srv-b".into()),
        yt_browser: Some(config::Browser::Brave),
        yt_anonymous: false,
    });
    cfg.active_server_id = Some("srv-b".into());
    cfg.theme = "midnight".into();

    db.save_config(&cfg).await.unwrap();

    // Play counts are written ONLY through bump_listen_count (a per-play
    // 1-row upsert), never by save_config — but load_config hydrates them.
    for _ in 0..7 {
        db.bump_listen_count("ytmusic:VID1").await.unwrap();
    }
    for _ in 0..3 {
        db.bump_listen_count("/music/a.flac").await.unwrap();
    }

    let loaded = db.load_config().await.unwrap().expect("config present");
    assert_eq!(loaded.theme, "midnight");
    assert_eq!(loaded.active_server_id.as_deref(), Some("srv-b"));
    assert_eq!(loaded.servers.len(), 2);
    let active = loaded.server.as_ref().expect("active server hydrated");
    assert_eq!(active.id.as_deref(), Some("srv-b"));
    assert_eq!(active.access_token.as_deref(), Some("TOPSECRET_COOKIE"));
    assert_eq!(active.yt_browser, Some(config::Browser::Brave));
    assert_eq!(loaded.listen_counts.get("ytmusic:VID1"), Some(&7));
    assert_eq!(loaded.listen_counts.get("/music/a.flac"), Some(&3));

    // The blob must not carry creds, the servers list, or the counts.
    let mut conn = open(&db_path).await;
    let blob: String = sqlx::query_scalar("SELECT json FROM app_config WHERE id = 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert!(!blob.contains("TOPSECRET_COOKIE"), "token leaked into the blob");
    let v: serde_json::Value = serde_json::from_str(&blob).unwrap();
    assert!(v.get("server").is_none());
    assert!(v.get("servers").is_none());
    assert!(v.get("listen_counts").is_none());
    assert_eq!(v.get("active_server_id").and_then(|x| x.as_str()), Some("srv-b"));

    // Removing a server from the list drops its row (the active one is kept).
    let mut cfg2 = loaded;
    cfg2.servers.retain(|s| s.id == "srv-b");
    cfg2.server = None;
    db.save_config(&cfg2).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM servers")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(n, 1, "srv-a removed, srv-b kept");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

async fn open(db_path: &std::path::Path) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap()
}
