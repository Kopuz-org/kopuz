//! Config persistence round-trip (issue #347, step 4): settings survive save→load beside the database, creds live in the `servers` table (never the file), and play counts live in `listen_counts`.

use std::path::{Path, PathBuf};

use config::{MusicServer, MusicService, SavedLocalSource, SavedServer, Settings, Source};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqliteConnection};

fn unique_db() -> PathBuf {
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
async fn webview_upgrade_removes_registered_sessions_and_keeps_cookie_sessions() {
    let path = unique_db();
    let pool = sqlx::SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    let mut previous =
        sqlx::migrate::Migrator::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations"))
            .await
            .unwrap();
    previous
        .migrations
        .to_mut()
        .retain(|migration| migration.version <= 20260919000000);
    previous.run(&pool).await.unwrap();
    let sessions = [
        ("yt-oauth", "YtMusic", "kopuz:youtube:oauth:v1", false),
        ("sc-oauth", "SoundCloud", "kopuz:soundcloud:oauth:v1", false),
        (
            "am-kit",
            "AppleMusic",
            "kopuz:musickit:v1:{\"music_user_token\":\"old\"}",
            false,
        ),
        ("yt-webview", "YtMusic", "SAPISID=keep; SID=session", true),
        ("sc-webview", "SoundCloud", "keep-sc-token", true),
        ("am-webview", "AppleMusic", "keep-am-token", true),
        ("spotify", "Spotify", "access\nrefresh", true),
    ];
    for (id, service, token, _) in sessions {
        sqlx::query("INSERT INTO servers (id, name, url, service, access_token, user_id, auth_state) VALUES (?1, ?1, '', ?2, ?3, 'user', 'active')")
            .bind(id).bind(service).bind(token).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO browser_auth (server_id, credentials) VALUES (?1, ?2)")
            .bind(id)
            .bind(r#"{"client_secret":"discard-me"}"#)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;
    let db = db::init(&path).await.unwrap();
    for (id, _, token, keep) in sessions {
        let source = db.load_server(id).await.unwrap().unwrap();
        assert_eq!(source.access_token.as_deref(), keep.then_some(token));
        assert_eq!(source.user_id.as_deref(), keep.then_some("user"));
    }
    let mut connection = SqliteConnectOptions::new()
        .filename(&path)
        .connect()
        .await
        .unwrap();
    let tables: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name = 'browser_auth'")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(tables, 0);
    let signed_out: i64 =
        sqlx::query_scalar("SELECT count(*) FROM servers s LEFT JOIN server_credentials c ON c.server_id = s.id WHERE c.server_id IS NULL")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(signed_out, 3);
}

#[tokio::test]
async fn webview_upgrade_after_server_split_keeps_current_credentials() {
    for browser_table_applied in [false, true] {
        let path = unique_db();

        let db = db::init(&path).await.unwrap();
        drop(db);
        let pool = sqlx::SqlitePool::connect_with(SqliteConnectOptions::new().filename(&path))
            .await
            .unwrap();
        sqlx::raw_sql(
            "DELETE FROM _sqlx_migrations WHERE version IN (20260919000000, 20260919010000, 20261003000000); \
             INSERT INTO servers (id, name, url, service) VALUES ('keep', 'Keep', '', 'YtMusic'), ('old', 'Old', '', 'YtMusic'); \
             INSERT INTO server_credentials (server_id, access_token, user_id) VALUES ('keep', 'SAPISID=keep; SID=session', 'user'), ('old', 'kopuz:youtube:oauth:v1', 'old-user');",
        )
        .execute(&pool)
        .await
        .unwrap();
        if browser_table_applied {
            let mut browser = sqlx::migrate::Migrator::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations"),
            )
            .await
            .unwrap();
            browser
                .migrations
                .to_mut()
                .retain(|m| m.version == 20260919000000);
            browser.set_ignore_missing(true);
            browser.run(&pool).await.unwrap();
        }
        pool.close().await;
        for _ in 0..2 {
            let db = db::init(&path).await.unwrap();
            let kept = db.load_server("keep").await.unwrap().unwrap();
            assert_eq!(
                kept.access_token.as_deref(),
                Some("SAPISID=keep; SID=session")
            );
            assert_eq!(kept.user_id.as_deref(), Some("user"));
            let old = db.load_server("old").await.unwrap().unwrap();
            assert!(old.access_token.is_none());
            assert!(old.user_id.is_none());
        }
    }
}

fn settings_path(db_path: &Path) -> PathBuf {
    config::store::settings_path_for(db_path.parent().unwrap())
}

/// What the daemon's settings service does on a save: make sure of the state row, then write the file.
async fn save(db: &db::Db, db_path: &Path, settings: &Settings) {
    db.ensure_app_state().await.unwrap();
    let every_key: Vec<config::SettingKey> = config::SettingKey::iter().collect();
    config::store::save_settings(&settings_path(db_path), settings, &every_key).unwrap();
}

fn load(db_path: &Path) -> Settings {
    config::store::FileLayers::read(&settings_path(db_path))
        .settings()
        .unwrap()
}

#[tokio::test]
async fn config_round_trips_with_creds_in_servers_table() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let servers = [
        SavedServer {
            id: "srv-a".into(),
            name: "Jelly".into(),
            url: "https://jelly.example".into(),
            service: MusicService::Jellyfin,
            yt_browser: None,
            yt_anonymous: false,
            yt_profile: None,
            account: None,
            apple_music_storefront: "us".into(),
            apple_music_language: "en".into(),
        },
        SavedServer {
            id: "srv-b".into(),
            name: "Yt".into(),
            url: "https://music.youtube.com".into(),
            service: MusicService::YtMusic,
            yt_browser: Some(config::Browser::Brave),
            yt_anonymous: false,
            yt_profile: Some("brave:/profiles/Default".into()),
            account: Some("brand-7".into()),
            apple_music_storefront: "us".into(),
            apple_music_language: "en".into(),
        },
    ];
    let mut cfg = Settings::default();
    cfg.ui.theme = "midnight".into();

    save(&db, &db_path, &cfg).await;
    for server in &servers {
        db.save_server(server).await.unwrap();
    }
    db.save_active_server(&MusicServer {
        name: "Yt".into(),
        url: "https://music.youtube.com".into(),
        service: MusicService::YtMusic,
        access_token: Some("TOPSECRET_COOKIE".into()),
        user_id: Some("u-1".into()),
        id: Some("srv-b".into()),
        yt_browser: Some(config::Browser::Brave),
        yt_anonymous: false,
        yt_profile: Some("brave:/profiles/Default".into()),
        account: Some("brand-7".into()),
        apple_music_storefront: "us".into(),
        apple_music_language: "en".into(),
    })
    .await
    .unwrap();
    db.set_active_source(&Source::Server("srv-b".into()))
        .await
        .unwrap();

    // Play counts are written only by bump_listen_count and read back by listen_counts.
    for _ in 0..7 {
        db.bump_listen_count(&Source::Server("srv-b".into()), "VID1")
            .await
            .unwrap();
    }
    for _ in 0..3 {
        db.bump_listen_count(&Source::default(), "/music/a.flac")
            .await
            .unwrap();
    }

    let loaded = load(&db_path);
    assert_eq!(loaded.ui.theme, "midnight");
    let sources = db.load_sources().await.unwrap();
    assert_eq!(sources.active.server_id(), Some("srv-b"));
    assert_eq!(sources.servers.len(), 2);
    let active = sources.server.as_ref().expect("active server hydrated");
    assert_eq!(active.id.as_deref(), Some("srv-b"));
    assert_eq!(active.access_token.as_deref(), Some("TOPSECRET_COOKIE"));
    assert_eq!(active.yt_browser, Some(config::Browser::Brave));
    let saved = sources.servers.iter().find(|s| s.id == "srv-b").unwrap();
    let jelly = sources.servers.iter().find(|s| s.id == "srv-a").unwrap();
    let hydrated = db.load_server("srv-b").await.unwrap().unwrap();
    assert_eq!(active.account.as_deref(), Some("brand-7"));
    assert_eq!(saved.account.as_deref(), Some("brand-7"));
    assert_eq!(
        jelly.account, None,
        "only the server that chose one has one"
    );
    assert_eq!(hydrated.account.as_deref(), Some("brand-7"));
    assert_eq!(
        hydrated.yt_profile.as_deref(),
        Some("brave:/profiles/Default")
    );
    assert_eq!(saved.yt_profile, hydrated.yt_profile);
    assert_eq!(jelly.yt_profile, None);
    let counts = db.listen_counts().await.unwrap();
    assert_eq!(counts.get("ytmusic:VID1"), Some(&7));
    assert_eq!(counts.get("/music/a.flac"), Some(&3));

    // The settings file carries settings only: no creds, servers, counts or state.
    let settings_path = settings_path(&db_path);
    let written = std::fs::read_to_string(&settings_path).expect("settings file written");
    assert!(
        !written.contains("TOPSECRET_COOKIE"),
        "token leaked into the file"
    );
    let written: toml::Table = written.parse().unwrap();
    for key in ["server", "servers", "listen_counts", "active_source"] {
        assert!(!written.contains_key(key), "{key} leaked into the file");
    }
    let mut conn = open(&db_path).await;
    let active: String = sqlx::query_scalar("SELECT active_source FROM app_state WHERE id = 1")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(active, "srv-b");

    // A settings save leaves the sources alone; deleting a server drops its row.
    save(&db, &db_path, &loaded).await;
    assert_eq!(db.load_sources().await.unwrap(), sources);
    db.delete_server("srv-a").await.unwrap();
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
    let mut cfg = Settings::default();
    cfg.library.local_sources = vec![local.clone()];
    let active = Source::LocalLibrary(local.id.clone());

    save(&db, &db_path, &cfg).await;
    db.set_active_source(&active).await.unwrap();
    db.bump_listen_count(&active, "/music/work/a.flac")
        .await
        .unwrap();
    let loaded = load(&db_path);
    let sources = db.load_sources().await.unwrap();

    assert_eq!(sources.active, active);
    assert_eq!(
        loaded.library.local_sources.len(),
        2,
        "the default folder source is kept alongside"
    );
    assert_eq!(loaded.library.local_sources[0].id, config::DEFAULT_LOCAL_ID);
    assert_eq!(loaded.library.local_sources[1], local);
    assert!(sources.server.is_none());
    assert_eq!(
        db.listen_counts()
            .await
            .unwrap()
            .get("local:test-library|/music/work/a.flac"),
        Some(&1),
    );
}

#[tokio::test]
async fn a_save_writes_the_settings_file_and_a_hand_edit_wins_on_load() {
    let db_path = unique_db();
    let settings_path = settings_path(&db_path);
    let db = db::init(&db_path).await.unwrap();

    let mut cfg = Settings::default();
    cfg.ui.theme = "midnight".into();
    save(&db, &db_path, &cfg).await;

    let text = std::fs::read_to_string(&settings_path).expect("settings file written");
    let mut written: toml::Table = text.parse().unwrap();
    assert_eq!(written["theme"].as_str(), Some("midnight"));

    written.insert("theme".into(), "nord".into());
    std::fs::write(&settings_path, written.to_string()).unwrap();
    assert_eq!(load(&db_path).ui.theme, "nord");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

// The allow is for cleanup only: re-enabling write on our own temp file so the temp dir can be removed.
#[allow(clippy::permissions_set_readonly_false)]
#[tokio::test]
async fn managed_settings_file_is_never_written_but_still_applies() {
    let db_path = unique_db();
    let settings_path = settings_path(&db_path);
    std::fs::write(&settings_path, "theme = \"nord\"\n").unwrap();
    let mut perms = std::fs::metadata(&settings_path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&settings_path, perms).unwrap();

    let db = db::init(&db_path).await.unwrap();

    // Nothing saved yet: the file alone configures the app.
    assert!(!db.has_app_state().await.unwrap());
    let layers = config::store::FileLayers::read(&settings_path);
    assert!(layers.has_overrides(), "file layers present");
    let loaded = layers.settings().unwrap();
    assert_eq!(loaded.ui.theme, "nord");

    // The immutable file is left alone; what it leaves unset goes beside it.
    let mut cfg = loaded;
    cfg.ui.theme = "dracula".into();
    cfg.playback.crossfade_seconds = 4;
    save(&db, &db_path, &cfg).await;
    assert_eq!(
        std::fs::read_to_string(&settings_path).unwrap(),
        "theme = \"nord\"\n"
    );
    let local: toml::Table = std::fs::read_to_string(config::store::local_path_for(&settings_path))
        .expect("local settings written")
        .parse()
        .unwrap();
    assert!(!local.contains_key("theme"));
    let reloaded = load(&db_path);
    assert_eq!(reloaded.ui.theme, "nord", "the managed key wins");
    assert_eq!(
        reloaded.playback.crossfade_seconds, 4,
        "an unmanaged setting persists"
    );

    let mut perms = std::fs::metadata(&settings_path).unwrap().permissions();
    perms.set_readonly(false);
    let _ = std::fs::set_permissions(&settings_path, perms);
    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// A value a higher layer pins is not the user's choice, so a save must not bake it into the settings file; uses a drop-in, which takes the env's locked-key path without mutating the process environment.
#[tokio::test]
async fn layered_overrides_are_not_persisted_as_base_config() {
    let db_path = unique_db();
    let settings_path = settings_path(&db_path);
    let db = db::init(&db_path).await.unwrap();

    let mut cfg = Settings::default();
    cfg.ui.theme = "midnight".into();
    save(&db, &db_path, &cfg).await;

    let dropin_dir = config::store::dropin_dir_for(&settings_path);
    std::fs::create_dir_all(&dropin_dir).unwrap();
    std::fs::write(dropin_dir.join("10-theme.toml"), "theme = \"nord\"\n").unwrap();

    let loaded = load(&db_path);
    assert_eq!(loaded.ui.theme, "nord", "the drop-in applies");

    let mut cfg = loaded;
    cfg.background.cover_art_darkening = 42;
    save(&db, &db_path, &cfg).await;

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
    let reloaded = load(&db_path);
    assert_eq!(
        reloaded.ui.theme, "midnight",
        "removing the drop-in must restore the configured theme"
    );
    assert_eq!(
        reloaded.background.cover_art_darkening, 42,
        "unpinned key persists"
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// A partial hand-written `settings.toml` plus a drop-in, one unusable value in each: the rest applies in precedence order.
#[tokio::test]
async fn hand_written_layers_apply_in_order_and_survive_bad_keys() {
    let db_path = unique_db();
    let settings_path = settings_path(&db_path);
    let db = db::init(&db_path).await.unwrap();

    let mut cfg = Settings::default();
    cfg.ui.theme = "midnight".into();
    cfg.ui.language = "en".into();
    cfg.playback.crossfade_seconds = 3;
    save(&db, &db_path, &cfg).await;

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

    let loaded = load(&db_path);
    assert_eq!(loaded.ui.theme, "dracula", "the drop-in out-ranks the file");
    assert_eq!(loaded.ui.language, "tr");
    assert_eq!(
        loaded.playback.crossfade_seconds,
        Settings::default().playback.crossfade_seconds,
        "a bad value falls back to the default"
    );
    assert_eq!(loaded.ui.ui_style, config::UiStyle::default());

    let mut cfg = loaded;
    cfg.ui.language = "de".into();
    save(&db, &db_path, &cfg).await;
    let written: toml::Table = std::fs::read_to_string(&settings_path)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(written["theme"].as_str(), Some("nord"));
    assert_eq!(written["language"].as_str(), Some("de"));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn playback_state_and_presence_pause_survive_a_reopen_and_a_config_save() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    assert_eq!(db.playback_state().await.unwrap(), (1.0, false));
    assert_eq!(db.discord_presence_paused().await.unwrap(), None);

    db.set_playback_state(0.35, true).await.unwrap();
    db.set_discord_presence_paused(false).await.unwrap();
    save(&db, &db_path, &Settings::default()).await;
    drop(db);

    let db = db::init(&db_path).await.unwrap();
    assert_eq!(db.playback_state().await.unwrap(), (0.35, true));
    assert_eq!(db.discord_presence_paused().await.unwrap(), Some(false));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

async fn open(db_path: &std::path::Path) -> SqliteConnection {
    SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap()
}
