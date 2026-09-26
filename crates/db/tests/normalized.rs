//! What each split-out table holds, and that a round trip through it loses nothing.

use std::path::PathBuf;

use config::{AppConfig, Browser, MusicServer, MusicService, SavedServer};
use db::Source;
use reader::models::{ArtistCredit, PlaylistEntry, Track, TrackId};

fn unique_db() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-norm-{}-{nanos}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

fn track(key: &str, credits: Vec<ArtistCredit>) -> Track {
    Track {
        id: TrackId::Server {
            service: MusicService::Jellyfin,
            item_id: key.into(),
        },
        cover: None,
        album_id: String::new(),
        title: key.into(),
        artist: "Ada".into(),
        album: String::new(),
        duration: 1,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec!["Ada".into()],
        credits,
    }
}

fn entry(key: &str, item_id: &str) -> PlaylistEntry {
    PlaylistEntry {
        key: key.into(),
        item_id: Some(item_id.into()),
    }
}

#[tokio::test]
async fn each_playlist_keeps_its_own_entry_id_for_a_shared_track() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("jf".into());
    db.set_playlist_tracks(&source, "A", &[entry("t1", "a-1")])
        .await
        .unwrap();
    db.set_playlist_tracks(&source, "B", &[entry("t1", "b-1")])
        .await
        .unwrap();

    let a = db.playlist_entries(&source, "A").await.unwrap();
    let b = db.playlist_entries(&source, "B").await.unwrap();

    assert_eq!(a, [entry("t1", "a-1")]);
    assert_eq!(b, [entry("t1", "b-1")]);
}

#[tokio::test]
async fn removing_one_copy_of_a_twice_listed_track_keeps_the_other() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("jf".into());
    let listed = [entry("t1", "e-1"), entry("t2", "e-2"), entry("t1", "e-3")];
    db.set_playlist_tracks(&source, "A", &listed).await.unwrap();

    db.remove_playlist_entry(&source, "A", 2).await.unwrap();

    let left = db.playlist_entries(&source, "A").await.unwrap();
    assert_eq!(left, [entry("t1", "e-1"), entry("t2", "e-2")]);
}

#[tokio::test]
async fn credits_round_trip_in_order_and_names_never_replace_ids() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("jf".into());
    let linked = track(
        "t1",
        vec![
            ArtistCredit::linked("Ada", "ar-1"),
            ArtistCredit::unlinked("Boris"),
        ],
    );
    db.upsert_tracks(&source, std::slice::from_ref(&linked))
        .await
        .unwrap();
    db.upsert_tracks(&source, &[track("t1", Vec::new())])
        .await
        .unwrap();

    let stored = db
        .tracks_by_keys(&source, &["t1".into()])
        .await
        .unwrap()
        .remove(0);

    assert_eq!(stored.credits, linked.credits);
    assert_eq!(stored.artists, ["Ada", "Boris"]);
}

#[tokio::test]
async fn a_row_without_credits_stores_its_names_and_follows_a_retag() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Local;
    let mut local = track("t1", Vec::new());
    local.id = TrackId::Local("/music/t1.flac".into());
    db.upsert_tracks(&source, &[local.clone()]).await.unwrap();
    local.artists = vec!["Cyd".into()];
    db.upsert_tracks(&source, &[local]).await.unwrap();

    let stored = db
        .tracks_by_keys(&source, &["/music/t1.flac".into()])
        .await
        .unwrap()
        .remove(0);

    assert_eq!(stored.credits, [ArtistCredit::unlinked("Cyd")]);
}

#[tokio::test]
async fn musicbrainz_ids_round_trip_and_clear() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("jf".into());
    let mut tagged = track("t1", Vec::new());
    tagged.musicbrainz_recording_id = Some("rec-1".into());
    db.upsert_tracks(&source, &[tagged.clone()]).await.unwrap();
    let read = |db: db::Db| async move {
        db.tracks_by_keys(&Source::Server("jf".into()), &["t1".into()])
            .await
            .unwrap()
            .remove(0)
    };

    assert_eq!(
        read(db.clone()).await.musicbrainz_recording_id.as_deref(),
        Some("rec-1")
    );

    tagged.musicbrainz_recording_id = None;
    db.upsert_tracks(&source, &[tagged]).await.unwrap();
    assert_eq!(read(db).await.musicbrainz_recording_id, None);
}

fn server(token: Option<&str>) -> AppConfig {
    AppConfig {
        servers: vec![SavedServer {
            id: "s1".into(),
            name: "am".into(),
            url: "https://example.test".into(),
            service: MusicService::AppleMusic,
            yt_browser: Some(Browser::Brave),
            yt_anonymous: true,
            apple_music_storefront: "jp".into(),
            apple_music_language: "en".into(),
        }],
        server: Some(MusicServer {
            name: "am".into(),
            url: "https://example.test".into(),
            service: MusicService::AppleMusic,
            access_token: token.map(Into::into),
            user_id: Some("u1".into()),
            id: Some("s1".into()),
            yt_browser: Some(Browser::Brave),
            yt_anonymous: true,
            apple_music_storefront: "jp".into(),
            apple_music_language: "en".into(),
        }),
        active_source: config::Source::Server("s1".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_server_round_trips_through_its_credentials_and_settings() {
    let db = db::init(&unique_db()).await.unwrap();
    let cfg = server(Some("token"));
    db.save_config(&cfg).await.unwrap();

    let loaded = db.load_config().await.unwrap().unwrap();

    assert_eq!(loaded.servers, cfg.servers);
    assert_eq!(loaded.server, cfg.server);
}

#[tokio::test]
async fn signing_out_drops_the_credentials_and_keeps_the_server() {
    let db = db::init(&unique_db()).await.unwrap();
    db.save_config(&server(Some("token"))).await.unwrap();
    db.set_server_credentials("s1", None, None).await.unwrap();

    let loaded = db.load_server("s1").await.unwrap().unwrap();

    assert_eq!(loaded.access_token, None);
    assert_eq!(loaded.apple_music_storefront, "jp");
}

#[tokio::test]
async fn credentials_for_an_unknown_server_are_ignored() {
    let db = db::init(&unique_db()).await.unwrap();

    db.set_server_credentials("nope", Some("token"), None)
        .await
        .unwrap();

    assert!(db.load_server("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn a_kv_value_round_trips() {
    let db = db::init(&unique_db()).await.unwrap();

    db.meta_put("synced:library", "srv", "123").await.unwrap();
    db.meta_put("synced:library", "srv", "456").await.unwrap();

    assert_eq!(
        db.meta_get("synced:library", "srv")
            .await
            .unwrap()
            .as_deref(),
        Some("456")
    );
    assert_eq!(db.meta_get("missing", "srv").await.unwrap(), None);
}

#[tokio::test]
async fn removing_a_server_takes_its_rows_with_it() {
    let db = db::init(&unique_db()).await.unwrap();
    let mut cfg = server(Some("token"));
    db.save_config(&cfg).await.unwrap();
    let source = Source::Server("s1".into());
    db.upsert_tracks(&source, &[track("t1", Vec::new())])
        .await
        .unwrap();
    db.set_playlist_tracks(&source, "P", &[entry("t1", "e-1")])
        .await
        .unwrap();
    db.set_favorite("s1", "t1", true).await.unwrap();
    db.bump_listen_count(&source, "t1").await.unwrap();

    cfg.servers.clear();
    cfg.server = None;
    cfg.active_source = config::Source::Local;
    db.save_config(&cfg).await.unwrap();

    assert!(
        db.tracks_by_keys(&source, &["t1".into()])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(db.playlist_entries(&source, "P").await.unwrap().is_empty());
    assert!(db.favorites("s1").await.unwrap().is_empty());
    assert!(
        db.load_config()
            .await
            .unwrap()
            .unwrap()
            .listen_counts
            .is_empty()
    );
}

#[tokio::test]
async fn a_track_gives_its_album_a_row_without_overwriting_a_real_one() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("yt".into());
    let mut billed = track("t1", vec![ArtistCredit::linked("Ada", "UC-ada")]);
    billed.album_id = "MPRE1".into();
    billed.album = "First".into();
    let mut other = track("t2", Vec::new());
    other.album_id = "MPRE2".into();
    db.upsert_albums(
        &source,
        &[reader::Album {
            id: "MPRE2".into(),
            title: "Real".into(),
            artist: "Boris".into(),
            genre: "Jazz".into(),
            year: 2001,
            cover_path: None,
            manual_cover: false,
            artist_id: None,
        }],
    )
    .await
    .unwrap();

    db.upsert_tracks(&source, &[billed, other]).await.unwrap();

    let first = db.album(&source, "MPRE1").await.unwrap().unwrap();
    assert_eq!(first.title, "First");
    assert_eq!(first.artist_id.as_deref(), Some("UC-ada"));
    let real = db.album(&source, "MPRE2").await.unwrap().unwrap();
    assert_eq!((real.title.as_str(), real.year), ("Real", 2001));
}

#[tokio::test]
async fn a_library_prune_keeps_albums_its_tracks_still_name() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("yt".into());
    let mut liked = track("t1", Vec::new());
    liked.album_id = "MPRE1".into();
    db.upsert_tracks(&source, &[liked]).await.unwrap();
    db.set_favorite("yt", "t1", true).await.unwrap();

    db.prune_source(&source, &[], &[]).await.unwrap();

    assert!(db.album(&source, "MPRE1").await.unwrap().is_some());
}

#[tokio::test]
async fn a_restored_queue_shows_what_the_library_holds_now() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("jf".into());
    let stale = track("t1", Vec::new());
    let transient = track("radio-only", Vec::new());
    db.save_queue(&db::QueueSnapshot {
        version: 1,
        queue: vec![stale.clone(), transient.clone()],
        ..Default::default()
    })
    .await
    .unwrap();
    let mut fresh = stale;
    fresh.title = "Renamed by a sync".into();
    db.upsert_tracks(&source, &[fresh]).await.unwrap();

    let restored = db.load_queue().await.unwrap().queue;

    assert_eq!(restored[0].title, "Renamed by a sync");
    assert_eq!(restored[1], transient);
}

#[tokio::test]
async fn a_remote_track_is_dated_once_and_keeps_its_date() {
    use sqlx::{ConnectOptions, sqlite::SqliteConnectOptions};
    let path = unique_db();
    let db = db::init(&path).await.unwrap();
    let source = Source::Server("jf".into());
    let mut conn = SqliteConnectOptions::new()
        .filename(&path)
        .connect()
        .await
        .unwrap();
    let added_at = async |conn: &mut sqlx::SqliteConnection| -> i64 {
        sqlx::query_scalar("SELECT added_at FROM tracks WHERE track_key = 't1'")
            .fetch_one(conn)
            .await
            .unwrap()
    };

    db.upsert_tracks(&source, &[track("t1", Vec::new())])
        .await
        .unwrap();
    assert!(added_at(&mut conn).await > 0, "first seen is its date");

    sqlx::query("UPDATE tracks SET added_at = 1 WHERE track_key = 't1'")
        .execute(&mut conn)
        .await
        .unwrap();
    db.upsert_tracks(&source, &[track("t1", Vec::new())])
        .await
        .unwrap();
    assert_eq!(added_at(&mut conn).await, 1, "a re-sync keeps the date");
}

#[tokio::test]
async fn only_a_real_album_credits_its_tracks_to_its_artist() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("jf".into());
    let on_album = |key: &str, album: &str, credits: Vec<ArtistCredit>| Track {
        album_id: album.into(),
        ..track(key, credits)
    };
    db.upsert_tracks(
        &source,
        &[
            on_album("t1", "derived", vec![ArtistCredit::linked("Ada", "ar-1")]),
            on_album("t2", "derived", vec![ArtistCredit::linked("Boris", "ar-2")]),
            on_album("t3", "real", vec![ArtistCredit::linked("Boris", "ar-2")]),
        ],
    )
    .await
    .unwrap();
    db.upsert_albums(
        &source,
        &[reader::Album {
            id: "real".into(),
            title: "Real".into(),
            artist: "Ada".into(),
            genre: String::new(),
            year: 0,
            cover_path: None,
            manual_cover: false,
            artist_id: Some("ar-1".into()),
        }],
    )
    .await
    .unwrap();

    let ada = db
        .artist_tracks(&source, &ArtistCredit::linked("Ada", "ar-1"), None)
        .await
        .unwrap();

    let keys: Vec<String> = ada.iter().map(|t| t.id.key().into_owned()).collect();
    assert_eq!(
        keys,
        ["t1", "t3"],
        "t2 sits on a guessed album and stays Boris's"
    );
}
