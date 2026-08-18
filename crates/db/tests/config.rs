//! Config persistence round-trip (issue #347, step 4): the in-memory `AppConfig`
//! survives save→load, creds live in the `servers` table (never the blob), and
//! play counts live in `listen_counts`.

use std::path::PathBuf;

use config::{AppConfig, MusicServer, MusicService, SavedLocalSource, SavedServer, Source};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqliteConnection};

fn unique_db() -> PathBuf {
    // pid + counter, not just clock: macOS's µs clock let parallel tests
    // collide on a nanos-only name and delete each other's live DB.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("kopuz-cfg-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

#[tokio::test]
async fn config_round_trips_with_creds_in_servers_table() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let cfg = AppConfig {
        servers: vec![
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
        ],
        server: Some(MusicServer {
            name: "Yt".into(),
            url: "https://music.youtube.com".into(),
            service: MusicService::YtMusic,
            access_token: Some("TOPSECRET_COOKIE".into()),
            user_id: Some("u-1".into()),
            id: Some("srv-b".into()),
            yt_browser: Some(config::Browser::Brave),
            yt_anonymous: false,
        }),
        active_source: config::Source::Server("srv-b".into()),
        theme: "midnight".into(),
        ..Default::default()
    };

    db.save_config(&cfg).await.unwrap();

    // Play counts are written ONLY through bump_listen_count (a per-play
    // 1-row upsert), never by save_config — but load_config hydrates them.
    for _ in 0..7 {
        db.bump_listen_count(&Source::Server("srv-b".into()), "ytmusic:VID1")
            .await
            .unwrap();
    }
    for _ in 0..3 {
        db.bump_listen_count(&Source::Local, "/music/a.flac")
            .await
            .unwrap();
    }

    let loaded = db.load_config().await.unwrap().expect("config present");
    assert_eq!(loaded.theme, "midnight");
    assert_eq!(loaded.active_source.server_id(), Some("srv-b"));
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
    assert!(
        !blob.contains("TOPSECRET_COOKIE"),
        "token leaked into the blob"
    );
    let v: serde_json::Value = serde_json::from_str(&blob).unwrap();
    assert!(v.get("server").is_none());
    assert!(v.get("servers").is_none());
    assert!(v.get("listen_counts").is_none());
    assert_eq!(
        v.get("active_source")
            .and_then(|s| s.get("Server"))
            .and_then(|x| x.as_str()),
        Some("srv-b")
    );

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

#[tokio::test]
async fn named_local_source_round_trips_as_active() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let local = SavedLocalSource {
        id: "local:test-library".into(),
        name: "Work music".into(),
        directories: vec![PathBuf::from("/music/work")],
    };
    let cfg = AppConfig {
        active_source: Source::LocalLibrary(local.id.clone()),
        local_sources: vec![local.clone()],
        ..Default::default()
    };

    db.save_config(&cfg).await.unwrap();
    db.bump_listen_count(&cfg.active_source, "/music/work/a.flac")
        .await
        .unwrap();
    let loaded = db.load_config().await.unwrap().expect("config present");

    assert_eq!(loaded.active_source, Source::LocalLibrary(local.id.clone()));
    assert_eq!(loaded.local_sources, vec![local]);
    assert!(loaded.server.is_none());
    assert_eq!(
        loaded
            .listen_counts
            .get("local:test-library|/music/work/a.flac"),
        Some(&1),
    );
}

#[tokio::test]
async fn settings_file_mirrors_saves_and_overrides_the_blob_on_load() {
    let db_path = unique_db();
    let settings_path = config::store::settings_path_for(db_path.parent().unwrap());
    let db = db::init(&db_path).await.unwrap();

    let cfg = AppConfig {
        theme: "midnight".into(),
        ..Default::default()
    };
    db.save_config(&cfg).await.unwrap();

    // The save mirrored the settings into the standalone file.
    let text = std::fs::read_to_string(&settings_path).expect("settings file written");
    let mut written: toml::Table = text.parse().unwrap();
    assert_eq!(written["theme"].as_str(), Some("midnight"));

    // A hand-edit (or hjem-managed value) in the file wins over the blob.
    written.insert("theme".into(), "nord".into());
    std::fs::write(&settings_path, written.to_string()).unwrap();
    let loaded = db.load_config().await.unwrap().expect("config present");
    assert_eq!(loaded.theme, "nord");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

// The allow is for cleanup only: re-enabling write on our own temp file so the
// temp dir can be removed.
#[allow(clippy::permissions_set_readonly_false)]
#[tokio::test]
async fn managed_settings_file_is_never_written_but_still_applies() {
    let db_path = unique_db();
    let settings_path = config::store::settings_path_for(db_path.parent().unwrap());
    std::fs::write(&settings_path, "theme = \"nord\"\nvolume = 0.25\n").unwrap();
    let mut perms = std::fs::metadata(&settings_path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&settings_path, perms).unwrap();

    let db = db::init(&db_path).await.unwrap();

    // No blob yet: the file layers alone configure the app.
    let loaded = db
        .load_config()
        .await
        .unwrap()
        .expect("file layers present");
    assert_eq!(loaded.theme, "nord");
    assert_eq!(loaded.volume, 0.25);

    // Saving persists to the blob and leaves the immutable file untouched;
    // its keys keep overriding what the UI changed.
    let mut cfg = loaded;
    cfg.theme = "dracula".into();
    cfg.crossfade_seconds = 4;
    db.save_config(&cfg).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&settings_path).unwrap(),
        "theme = \"nord\"\nvolume = 0.25\n"
    );
    let reloaded = db.load_config().await.unwrap().expect("config present");
    assert_eq!(reloaded.theme, "nord", "managed key wins over the blob");
    assert_eq!(reloaded.crossfade_seconds, 4, "unmanaged key persists");

    let mut perms = std::fs::metadata(&settings_path).unwrap().permissions();
    perms.set_readonly(false);
    let _ = std::fs::set_permissions(&settings_path, perms);
    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// A value a higher layer pins is not the user's choice, so a save must not
/// bake it into the blob or the settings file: removing the layer has to
/// restore what was configured before. Uses a drop-in rather than
/// `KOPUZ_CONFIG_THEME` — same locked-key path, without mutating the process
/// environment out from under the other tests in this binary.
#[tokio::test]
async fn layered_overrides_are_not_persisted_as_base_config() {
    let db_path = unique_db();
    let settings_path = config::store::settings_path_for(db_path.parent().unwrap());
    let db = db::init(&db_path).await.unwrap();

    let cfg = AppConfig {
        theme: "midnight".into(),
        ..Default::default()
    };
    db.save_config(&cfg).await.unwrap();

    let dropin_dir = config::store::dropin_dir_for(&settings_path);
    std::fs::create_dir_all(&dropin_dir).unwrap();
    std::fs::write(dropin_dir.join("10-theme.toml"), "theme = \"nord\"\n").unwrap();

    let loaded = db.load_config().await.unwrap().expect("config present");
    assert_eq!(loaded.theme, "nord", "the drop-in applies");

    // Change something unrelated while the override is in force.
    let mut cfg = loaded;
    cfg.volume = 0.42;
    db.save_config(&cfg).await.unwrap();

    let written: toml::Table = std::fs::read_to_string(&settings_path)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        written["theme"].as_str(),
        Some("midnight"),
        "the drop-in's theme leaked into the settings file"
    );

    std::fs::remove_dir_all(&dropin_dir).unwrap();
    let reloaded = db.load_config().await.unwrap().expect("config present");
    assert_eq!(
        reloaded.theme, "midnight",
        "removing the drop-in must restore the configured theme"
    );
    assert_eq!(reloaded.volume, 0.42, "unpinned key persists");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// The hand-written path end to end: a partial `settings.toml` plus a drop-in
/// over an existing blob, with one unusable value in each. Everything the app
/// can use applies, in precedence order, and the bad keys cost only themselves.
#[tokio::test]
async fn hand_written_layers_apply_over_the_blob_and_survive_bad_keys() {
    let db_path = unique_db();
    let settings_path = config::store::settings_path_for(db_path.parent().unwrap());
    let db = db::init(&db_path).await.unwrap();

    let cfg = AppConfig {
        theme: "midnight".into(),
        language: "en".into(),
        crossfade_seconds: 3,
        volume: 0.8,
        ..Default::default()
    };
    db.save_config(&cfg).await.unwrap();

    std::fs::write(
        &settings_path,
        "theme = \"nord\"\nlanguage = \"tr\"\ncrossfade_seconds = \"loud\"\n",
    )
    .unwrap();
    let dropin_dir = config::store::dropin_dir_for(&settings_path);
    std::fs::create_dir_all(&dropin_dir).unwrap();
    std::fs::write(
        dropin_dir.join("20-theme.toml"),
        "theme = \"dracula\"\nui_style = \"Fancy\"\n",
    )
    .unwrap();

    let loaded = db.load_config().await.unwrap().expect("config present");
    assert_eq!(loaded.theme, "dracula", "the drop-in out-ranks the file");
    assert_eq!(loaded.language, "tr", "the file out-ranks the blob");
    assert_eq!(loaded.volume, 0.8, "untouched keys come from the blob");
    assert_eq!(
        loaded.crossfade_seconds, 3,
        "a bad value falls back to the stored one, not to the default"
    );
    assert_eq!(loaded.ui_style, config::UiStyle::default());

    // Saving on top of that doesn't corrupt the hand-written file: the pinned
    // drop-in key keeps the file's own value and the rest mirrors normally.
    let mut cfg = loaded;
    cfg.volume = 0.25;
    db.save_config(&cfg).await.unwrap();
    let written: toml::Table = std::fs::read_to_string(&settings_path)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(written["theme"].as_str(), Some("nord"));
    assert_eq!(written["language"].as_str(), Some("tr"));
    assert_eq!(written["volume"].as_float(), Some(0.25));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

async fn open(db_path: &std::path::Path) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap()
}
