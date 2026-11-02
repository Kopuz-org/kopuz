//! Artist photos are filed under the artist's identity, `(source, artist key)`.

use std::path::PathBuf;

use db::{ArtistRow, Source};
use reader::ArtistCredit;
use reader::models::{Track, TrackId};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, Executor, Row};

fn unique_db() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-ai-{}-{nanos}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

fn track(path: &str, artist: &str, credits: Vec<ArtistCredit>) -> Track {
    Track {
        id: TrackId::Local(PathBuf::from(path)),
        cover: None,
        album_id: format!("alb-{path}"),
        title: path.into(),
        artist: artist.into(),
        album: "Album".into(),
        duration: 1,
        khz: 44100,
        bitrate: 900,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec![artist.into()],
        replay_gain: config::ReplayGainInfo::default(),
        counterpart: None,
        credits,
        explicit: false,
        plays: None,
    }
}

async fn artist_named(db: &db::Db, source: &Source, name: &str) -> ArtistRow {
    db.artists(source)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.name == name && row.source_id.is_none())
        .expect("an unlinked row")
}

fn id(source: &str, key: &str) -> (String, String) {
    (source.to_string(), key.to_string())
}

#[tokio::test]
async fn unlinked_homonyms_in_two_sources_keep_their_own_photos() {
    let db = db::init(&unique_db()).await.unwrap();
    let (local, srv) = (Source::default(), Source::Server("s1".into()));
    db.upsert_tracks(&local, &[track("/a.flac", "Ada", Vec::new())])
        .await
        .unwrap();
    db.upsert_tracks(&srv, &[track("b", "Ada", Vec::new())])
        .await
        .unwrap();
    let on_local = artist_named(&db, &local, "Ada").await;
    let on_srv = artist_named(&db, &srv, "Ada").await;
    assert_ne!(on_local.key, on_srv.key);

    db.set_artist_image(&local, &on_local.key, "server", Some("https://p/local.jpg"))
        .await
        .unwrap();
    db.set_artist_image(&srv, &on_srv.key, "server", Some("https://p/srv.jpg"))
        .await
        .unwrap();

    let (_, photos) = db.artist_images().await.unwrap();
    assert_eq!(photos.len(), 2);
    let remote = |url: &str| reader::ArtistImageRef::Remote(url.into());
    assert_eq!(
        photos.get(&id("local", &on_local.key)),
        Some(&remote("https://p/local.jpg"))
    );
    assert_eq!(
        photos.get(&id("s1", &on_srv.key)),
        Some(&remote("https://p/srv.jpg"))
    );
}

#[tokio::test]
async fn a_custom_upload_leaves_a_homonym_alone() {
    let db = db::init(&unique_db()).await.unwrap();
    let (local, srv) = (Source::default(), Source::Server("s1".into()));
    db.upsert_tracks(&local, &[track("/a.flac", "Ada", Vec::new())])
        .await
        .unwrap();
    let linked = ArtistCredit {
        source: Some(srv.clone()),
        ..ArtistCredit::linked("Ada", "ar-1")
    };
    db.upsert_tracks(
        &srv,
        &[
            track("b", "Ada", vec![linked]),
            track("c", "Ada", Vec::new()),
        ],
    )
    .await
    .unwrap();
    let on_local = artist_named(&db, &local, "Ada").await;
    let unlinked = artist_named(&db, &srv, "Ada").await;

    db.set_artist_image(&srv, &unlinked.key, "custom", Some("/up/ada.png"))
        .await
        .unwrap();

    let (overrides, _) = db.artist_images().await.unwrap();
    assert_eq!(overrides.len(), 1);
    assert!(overrides.contains_key(&id("s1", &unlinked.key)));
    assert!(!overrides.contains_key(&id("s1", "ar-1")));
    assert!(!overrides.contains_key(&id("local", &on_local.key)));

    db.set_artist_image(&srv, &unlinked.key, "custom", None)
        .await
        .unwrap();
    assert!(db.artist_images().await.unwrap().0.is_empty());
}

#[tokio::test]
async fn unlinked_artist_keys_answer_by_folded_name() {
    let db = db::init(&unique_db()).await.unwrap();
    db.upsert_tracks(&Source::default(), &[track("/a.flac", "Ada ", Vec::new())])
        .await
        .unwrap();
    let row = artist_named(&db, &Source::default(), "Ada").await;
    let keys = db.unlinked_artist_keys(&Source::default()).await.unwrap();
    assert_eq!(keys.get("ada"), Some(&row.key));
    assert!(
        db.unlinked_artist_keys(&Source::Server("s1".into()))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn linked_artist_keys_answer_by_source_id() {
    let db = db::init(&unique_db()).await.unwrap();
    let srv = Source::Server("s1".into());
    let linked = ArtistCredit {
        source: Some(srv.clone()),
        ..ArtistCredit::linked("Ada", "ar-1")
    };
    db.upsert_tracks(
        &srv,
        &[
            track("b", "Ada", vec![linked]),
            track("c", "Bob", Vec::new()),
        ],
    )
    .await
    .unwrap();
    let keys = db.linked_artist_keys(&srv).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert!(keys.contains_key("ar-1"));
    assert!(
        db.linked_artist_keys(&Source::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn migration_converts_both_legacy_key_shapes() {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&migrations)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    files.sort();
    let target = "20260930000012_artist_image_identity.sql";
    let (before, after): (Vec<_>, Vec<_>) = files
        .iter()
        .partition(|path| path.file_name().unwrap().to_str().unwrap() < target);

    let mut conn = SqliteConnectOptions::new()
        .in_memory(true)
        .connect()
        .await
        .unwrap();
    for file in before {
        conn.execute(std::fs::read_to_string(file).unwrap().as_str())
            .await
            .unwrap();
    }
    conn.execute(
        "INSERT INTO artists (source, source_artist_id, name, name_key, key) VALUES \
           ('local', NULL, 'Ada', 'ada', 'k-local'), \
           ('s1', NULL, 'Ada', 'ada', 'k-s1'), \
           ('s1', 'ar-1', 'Ada Lovelace', 'ada lovelace', 'ar-1'), \
           ('s2', 'ar-1', 'Someone', 'someone', 'ar-1'); \
         INSERT INTO artist_images (artist_norm, kind, image_ref) VALUES \
           ('id:s1:ar-1', 'server', 'https://p/ar1'), \
           ('ada', 'server', 'https://p/ada'), \
           ('ada', 'custom', '/pics/ada.png'), \
           ('ada lovelace', 'custom', '/pics/lovelace.png'), \
           ('ada lovelace', 'server', 'https://p/lovelace'), \
           ('nobody', 'server', 'https://p/nobody'), \
           ('id:s9:zz', 'server', 'https://p/gone'); \
         INSERT INTO kv (name, kind, value) VALUES ('ada', 'artist_photo_miss', '');",
    )
    .await
    .unwrap();
    for file in after {
        conn.execute(std::fs::read_to_string(file).unwrap().as_str())
            .await
            .unwrap();
    }

    let rows = sqlx::query(
        "SELECT source, artist_key, kind, image_ref FROM artist_images \
         ORDER BY source, artist_key, kind",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    let got: Vec<(String, String, String, String)> = rows
        .iter()
        .map(|row| {
            (
                row.get("source"),
                row.get("artist_key"),
                row.get("kind"),
                row.get("image_ref"),
            )
        })
        .collect();
    let row = |a: &str, b: &str, c: &str, d: &str| (a.into(), b.into(), c.into(), d.into());
    assert_eq!(
        got,
        vec![
            row("local", "k-local", "custom", "/pics/ada.png"),
            row("local", "k-local", "server", "https://p/ada"),
            row("s1", "ar-1", "custom", "/pics/lovelace.png"),
            row("s1", "ar-1", "server", "https://p/ar1"),
            row("s1", "k-s1", "custom", "/pics/ada.png"),
            row("s1", "k-s1", "server", "https://p/ada"),
        ]
    );
    let misses: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM kv WHERE kind = 'artist_photo_miss'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(misses, 0);
    conn.close().await.unwrap();
}
