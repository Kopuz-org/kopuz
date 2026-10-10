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
        replay_gain: config::ReplayGainInfo::default(),
        credits,
    }
}

/// A credit as a source wrote it, without the library row a read files it under.
fn named(credits: &[ArtistCredit]) -> Vec<(&str, Option<&str>)> {
    credits
        .iter()
        .map(|credit| (credit.name.as_str(), credit.id.as_deref()))
        .collect()
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

    assert_eq!(named(&stored.credits), named(&linked.credits));
    assert_eq!(stored.artists, ["Ada", "Boris"]);
}

#[tokio::test]
async fn a_row_without_credits_stores_its_names_and_follows_a_retag() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::default();
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

    assert_eq!(named(&stored.credits), [("Cyd", None)]);
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
    cfg.active_source = config::Source::default();
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
            artist_key: None,
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
    db.save_queue(
        &source,
        &db::QueueSnapshot {
            version: 1,
            queue: vec![stale.clone(), transient.clone()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut fresh = stale;
    fresh.title = "Renamed by a sync".into();
    db.upsert_tracks(&source, &[fresh]).await.unwrap();

    let restored = db.load_queue(&source).await.unwrap().queue;

    assert_eq!(restored[0].title, "Renamed by a sync");
    let named = Track {
        credits: vec![ArtistCredit::unlinked("Ada")],
        ..transient
    };
    assert_eq!(
        restored[1], named,
        "its names come back as unlinked credits"
    );
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
            artist_key: Some("ar-1".into()),
        }],
    )
    .await
    .unwrap();

    let ada = db.artist_tracks(&source, "ar-1", None).await.unwrap();

    let keys: Vec<String> = ada.iter().map(|t| t.id.key().into_owned()).collect();
    assert_eq!(
        keys,
        ["t1", "t3"],
        "t2 sits on a guessed album and stays Boris's"
    );
}

#[tokio::test]
async fn a_renamed_artist_keeps_its_row_and_a_prune_drops_what_nothing_credits() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("jf".into());
    db.upsert_tracks(
        &source,
        &[
            track("t1", vec![ArtistCredit::linked("Ada", "ar-1")]),
            track("t2", vec![ArtistCredit::unlinked("Boris")]),
        ],
    )
    .await
    .unwrap();
    let boris = db
        .artists(&source)
        .await
        .unwrap()
        .into_iter()
        .find(|artist| artist.name == "Boris")
        .unwrap()
        .key;

    db.upsert_tracks(
        &source,
        &[track(
            "t1",
            vec![ArtistCredit::linked("Ada Lovelace", "ar-1")],
        )],
    )
    .await
    .unwrap();
    db.prune_source(&source, &["t1".into()], &[]).await.unwrap();

    let renamed = db.artist(&source, "ar-1").await.unwrap().unwrap();
    assert_eq!((renamed.name.as_str(), renamed.tracks), ("Ada Lovelace", 1));
    assert_eq!(db.artist(&source, &boris).await.unwrap(), None);
}

#[tokio::test]
async fn an_artist_is_keyed_by_its_source_id_or_by_one_key_minted_when_filed() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("jf".into());
    let file = || async {
        db.upsert_tracks(
            &source,
            &[
                track("t1", vec![ArtistCredit::linked("Ada", "ar-1")]),
                track("t2", vec![ArtistCredit::unlinked("Boris")]),
                track("t3", vec![ArtistCredit::unlinked("Cleo")]),
            ],
        )
        .await
        .unwrap();
        db.artists(&source).await.unwrap()
    };

    let first = file().await;
    let again = file().await;

    let key = |listed: &[db::ArtistRow], name: &str| {
        listed
            .iter()
            .find(|artist| artist.name == name)
            .unwrap()
            .key
            .clone()
    };
    assert_eq!(key(&first, "Ada"), "ar-1");
    assert_ne!(key(&first, "Boris"), key(&first, "Cleo"));
    assert_eq!(key(&first, "Boris").len(), 32, "a minted key, not a name");
    assert_eq!(first, again, "filing again keeps every key");
    let boris = db.artist(&source, &key(&first, "Boris")).await.unwrap();
    assert_eq!(boris.map(|artist| artist.name).as_deref(), Some("Boris"));
}

#[tokio::test]
async fn once_the_source_names_an_artist_a_credit_no_longer_renames_it() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("yt".into());
    let billed = |text: &str| track("t1", vec![ArtistCredit::linked(text, "UC-4lat")]);
    db.upsert_tracks(&source, &[billed("4LAT feat. Kasane Teto")])
        .await
        .unwrap();
    assert!(
        db.artist_keys_unnamed_by_source(&source)
            .await
            .unwrap()
            .contains("UC-4lat")
    );

    assert!(db.name_artist(&source, "UC-4lat", "4LAT").await.unwrap());
    assert!(
        !db.name_artist(&source, "UC-4lat", "4LAT").await.unwrap(),
        "already so"
    );
    db.upsert_tracks(&source, &[billed("4LAT feat. Someone Else")])
        .await
        .unwrap();

    let row = db.artist(&source, "UC-4lat").await.unwrap().unwrap();
    assert_eq!(row.name, "4LAT");
    assert!(
        db.artist_keys_unnamed_by_source(&source)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_queue_round_trips_through_its_rows() {
    let db = db::init(&unique_db()).await.unwrap();
    let mut listed = track("t1", Vec::new());
    listed.credits = vec![ArtistCredit {
        source: Some(Source::Server("yt".into())),
        key: Some("UC-ada".into()),
        ..ArtistCredit::linked("Ada", "UC-ada")
    }];
    listed.playlist_item_id = Some("entry-1".into());
    let mut stream = track("station", vec![ArtistCredit::unlinked("Radio")]);
    stream.duration = u64::MAX;
    stream.artists = vec!["Radio".into()];
    let saved = db::QueueSnapshot {
        version: 1,
        queue: vec![listed, stream],
        current_queue_index: 1,
        progress_secs: 40,
        shuffle_order: vec![1, 0, 9],
        shuffle_enabled: true,
        radio_cursor: None,
    };

    db.save_queue(&Source::default(), &saved).await.unwrap();
    let restored = db.load_queue(&Source::default()).await.unwrap();

    assert_eq!(restored.queue, saved.queue, "a stream keeps having no end");
    assert_eq!(
        restored.shuffle_order,
        [1, 0],
        "a slot past the end is dropped"
    );
    assert_eq!(
        (
            restored.current_queue_index,
            restored.progress_secs,
            restored.shuffle_enabled
        ),
        (1, 40, true)
    );
}

#[tokio::test]
async fn a_position_save_leaves_the_rows_alone() {
    let db = db::init(&unique_db()).await.unwrap();
    let queued = db::QueueSnapshot {
        version: 1,
        queue: vec![track("t1", Vec::new()), track("t2", Vec::new())],
        ..Default::default()
    };
    db.save_queue(&Source::default(), &queued).await.unwrap();

    db.save_queue_position(
        &Source::default(),
        &db::QueueSnapshot {
            current_queue_index: 1,
            progress_secs: 12,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let restored = db.load_queue(&Source::default()).await.unwrap();
    assert_eq!(restored.queue.len(), 2);
    assert_eq!(
        (restored.current_queue_index, restored.progress_secs),
        (1, 12)
    );
}

/// Downloads and pins are written a row at a time, so a whole-config save must neither store nor erase them.
#[tokio::test]
async fn offline_copies_and_pins_live_in_their_own_tables() {
    let path = unique_db();
    let db = db::init(&path).await.unwrap();
    db.save_config(&AppConfig::default()).await.unwrap();
    db.set_offline_track("t1", Some("/cache/t1.flac"))
        .await
        .unwrap();
    db.set_pinned_station("moe", Some(r#"{"id":"moe"}"#))
        .await
        .unwrap();
    db.set_pinned_station("jazz", Some(r#"{"id":"jazz"}"#))
        .await
        .unwrap();
    db.set_pinned_station("moe", Some(r#"{"id":"moe","name":"LISTEN.moe"}"#))
        .await
        .unwrap();

    let mut stale = db.load_config().await.unwrap().unwrap();
    stale.offline_tracks.clear();
    stale.pinned_stations.clear();
    db.save_config(&stale).await.unwrap();

    let loaded = db.load_config().await.unwrap().unwrap();
    assert_eq!(
        loaded.offline_tracks.get("t1").map(String::as_str),
        Some("/cache/t1.flac")
    );
    assert_eq!(
        loaded.pinned_stations,
        [r#"{"id":"moe","name":"LISTEN.moe"}"#, r#"{"id":"jazz"}"#],
        "a re-pin keeps its place"
    );
    let settings =
        std::fs::read_to_string(config::store::settings_path_for(path.parent().unwrap())).unwrap();
    assert!(!settings.contains("offline_tracks") && !settings.contains("pinned_stations"));

    db.set_offline_track("t1", None).await.unwrap();
    db.set_pinned_station("moe", None).await.unwrap();
    let loaded = db.load_config().await.unwrap().unwrap();
    assert!(loaded.offline_tracks.is_empty());
    assert_eq!(loaded.pinned_stations, [r#"{"id":"jazz"}"#]);
}

#[tokio::test]
async fn lyrics_round_trip_through_their_rows() {
    use utils::lyrics::{LyricChunk, LyricLine, Lyrics};

    let db = db::init(&unique_db()).await.unwrap();
    let synced = Lyrics::Synced(vec![
        LyricLine {
            start_time: 1.5,
            end_time: Some(3.0),
            text: "first".into(),
            chunks: vec![
                LyricChunk {
                    start_time: 1.5,
                    text: "fi".into(),
                },
                LyricChunk {
                    start_time: 2.0,
                    text: "rst".into(),
                },
            ],
            parent_line_index: None,
            background: false,
            opposite_turn: true,
        },
        LyricLine {
            start_time: 3.0,
            end_time: None,
            text: "echo".into(),
            chunks: Vec::new(),
            parent_line_index: Some(0),
            background: true,
            opposite_turn: false,
        },
    ]);
    let plain = Lyrics::Plain("words".into());

    db.cache_lyrics("a", Some(&synced)).await.unwrap();
    db.cache_lyrics("b", Some(&plain)).await.unwrap();
    db.cache_lyrics("c", None).await.unwrap();
    db.cache_lyrics("b", None).await.unwrap();

    assert_eq!(
        db.cached_lyrics("a").await.unwrap(),
        Some(db::CachedLyrics::Found(synced))
    );
    assert!(matches!(
        db.cached_lyrics("b").await.unwrap(),
        Some(db::CachedLyrics::Missing { .. })
    ));
    assert!(matches!(
        db.cached_lyrics("c").await.unwrap(),
        Some(db::CachedLyrics::Missing { .. })
    ));
    assert_eq!(db.cached_lyrics("d").await.unwrap(), None);
}

#[tokio::test]
async fn the_single_and_bulk_artist_covers_agree_for_every_artist() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("yt".into());
    let on = |key: &str, album: &str, credits: Vec<ArtistCredit>| {
        let mut row = track(key, credits);
        row.album_id = album.into();
        row
    };
    db.upsert_tracks(
        &source,
        &[
            on("t1", "A1", vec![ArtistCredit::linked("Ada", "UC-ada")]),
            on(
                "t2",
                "A2",
                vec![
                    ArtistCredit::linked("Ada", "UC-ada"),
                    ArtistCredit::linked("Boris", "UC-boris"),
                ],
            ),
            on("t3", "A3", vec![ArtistCredit::unlinked("Cleo")]),
        ],
    )
    .await
    .unwrap();
    for (album, cover) in [("A2", "/a2.jpg"), ("A1", "/a1.jpg")] {
        db.update_album_cover(&source, album, Some(cover), false)
            .await
            .unwrap();
    }

    let bulk = db.artist_album_covers(&source).await.unwrap();
    let artists = db.artists(&source).await.unwrap();
    assert_eq!(artists.len(), 3);
    for artist in &artists {
        let single = db.artist_album_cover(&source, &artist.key).await.unwrap();
        assert_eq!(single, bulk.get(&artist.key).cloned(), "{}", artist.name);
    }
    assert_eq!(bulk.get("UC-ada").map(String::as_str), Some("/a1.jpg"));
    assert_eq!(bulk.get("UC-boris").map(String::as_str), Some("/a2.jpg"));
    assert_eq!(bulk.len(), 2);
}
