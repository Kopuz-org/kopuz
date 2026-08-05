//! The `MediaSource` facade (issue #347, Phase 2) over a real temp DB. Exercises
//! the local impl end-to-end through the public trait — `create_playlist` /
//! `add_to_playlist` / `set_favorite` route to the DB and read back — so the
//! facade's wiring is covered without a GUI. The remote impl needs a live
//! server and is verified against real accounts instead.

use std::path::PathBuf;

use db::Source;
use reader::{Track, TrackId};
use server::source;

fn track(id: TrackId) -> Track {
    Track {
        id,
        cover: None,
        album_id: String::new(),
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        duration: 0,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: Vec::new(),
    }
}

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
    let dir = std::env::temp_dir().join(format!("kopuz-source-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

#[tokio::test]
async fn local_create_then_add_playlist_round_trips() {
    let db = db::init(&unique_db()).await.unwrap();
    let src = source::local(db.clone(), Source::Local);

    let id = src
        .create_playlist("Road Trip", &["/music/a.flac".into()])
        .await
        .unwrap();

    // The created playlist is readable with its seed track.
    let store = db.load_playlists(&Source::Local).await.unwrap();
    let pl = store
        .playlists
        .iter()
        .find(|p| p.id == id)
        .expect("created playlist present");
    assert_eq!(pl.name, "Road Trip");
    assert_eq!(pl.tracks, vec!["/music/a.flac".to_string()]);

    // Appending dedups and preserves order.
    let landed = src
        .add_to_playlist(&id, &["/music/b.flac".into(), "/music/a.flac".into()])
        .await
        .unwrap();
    assert_eq!(landed.len(), 2);

    let store = db.load_playlists(&Source::Local).await.unwrap();
    let pl = store.playlists.iter().find(|p| p.id == id).unwrap();
    assert_eq!(
        pl.tracks,
        vec!["/music/a.flac".to_string(), "/music/b.flac".to_string()],
        "existing track not duplicated, new one appended"
    );
}

#[tokio::test]
async fn local_favorite_round_trips() {
    let db = db::init(&unique_db()).await.unwrap();
    let src = source::local(db.clone(), Source::Local);

    assert!(!src.is_favorite("/music/x.flac").await);

    src.set_favorite("/music/x.flac", true).await.unwrap();
    assert!(src.is_favorite("/music/x.flac").await);
    assert!(
        db.favorites("local")
            .await
            .unwrap()
            .contains(&"/music/x.flac".to_string())
    );

    src.set_favorite("/music/x.flac", false).await.unwrap();
    assert!(!src.is_favorite("/music/x.flac").await);
}

#[tokio::test]
async fn record_favorite_writes_a_clean_local_row_and_reverts() {
    let db = db::init(&unique_db()).await.unwrap();
    let src = source::local(db.clone(), Source::Local);
    let t = track(TrackId::Local("/music/x.flac".into()));

    // record_favorite writes the local state as a CLEAN row (no dirty/pending) —
    // the optimistic half of a toggle.
    src.record_favorite(&t, true).await.unwrap();
    assert!(
        db.favorites("local")
            .await
            .unwrap()
            .contains(&"/music/x.flac".to_string())
    );
    assert!(db.dirty_favorites("local").await.unwrap().is_empty());

    // Calling it with the opposite `on` reverts cleanly (the revert-on-push-fail
    // path) — no favorite, no lingering row.
    src.record_favorite(&t, false).await.unwrap();
    assert!(!src.is_favorite("/music/x.flac").await);
    assert!(db.dirty_favorites("local").await.unwrap().is_empty());
    assert!(db.dirty_unlikes("local").await.unwrap().is_empty());
}

#[tokio::test]
async fn portable_local_metadata_survives_a_different_mount_path() {
    let global_a_path = unique_db();
    let test_dir = global_a_path.parent().unwrap();
    let root_a = test_dir.join("computer-a").join("Music");
    let root_b = test_dir.join("computer-b").join("Shared Music");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();

    let source_a = Source::LocalLibrary("local:computer-a".into());
    let first_a = root_a.join("album").join("first.flac");
    let second_a = root_a.join("album").join("second.flac");
    let db_a = db::init(&global_a_path).await.unwrap();

    // Existing app-local metadata is imported the first time this folder gets
    // a portable database.
    db_a.set_favorite(source_a.as_str(), &first_a.to_string_lossy(), true)
        .await
        .unwrap();
    db_a.clear_favorite_dirty(source_a.as_str(), &first_a.to_string_lossy())
        .await
        .unwrap();
    db_a.upsert_playlist_meta(&source_a, "road-trip", "Road Trip", None, None)
        .await
        .unwrap();
    db_a.set_playlist_tracks(
        &source_a,
        "road-trip",
        &[first_a.to_string_lossy().into_owned()],
    )
    .await
    .unwrap();

    let src_a = source::local_with_directories(db_a, source_a, vec![root_a.clone()]);
    assert_eq!(
        src_a.favorites().await.unwrap(),
        vec![first_a.to_string_lossy().into_owned()]
    );
    src_a
        .record_favorite(&track(TrackId::Local(second_a.clone())), true)
        .await
        .unwrap();
    src_a
        .add_to_playlist("road-trip", &[second_a.to_string_lossy().into_owned()])
        .await
        .unwrap();

    let portable_a = root_a.join(source::PORTABLE_LIBRARY_DB_FILENAME);
    assert!(portable_a.is_file());
    drop(src_a);

    // Simulate the same shared folder appearing under another mount path.
    let portable_b = root_b.join(source::PORTABLE_LIBRARY_DB_FILENAME);
    std::fs::copy(&portable_a, &portable_b).unwrap();
    let db_b = db::init(&test_dir.join("computer-b.db")).await.unwrap();
    let src_b = source::local_with_directories(
        db_b,
        Source::LocalLibrary("local:computer-b".into()),
        vec![root_b.clone()],
    );

    let favorites = src_b.favorites().await.unwrap();
    assert_eq!(
        favorites,
        vec![
            root_b
                .join("album")
                .join("second.flac")
                .to_string_lossy()
                .into_owned(),
            root_b
                .join("album")
                .join("first.flac")
                .to_string_lossy()
                .into_owned(),
        ]
    );
    let store = src_b.load_playlists().await.unwrap();
    assert_eq!(store.playlists.len(), 1);
    assert_eq!(
        store.playlists[0].tracks,
        vec![
            root_b
                .join("album")
                .join("first.flac")
                .to_string_lossy()
                .into_owned(),
            root_b
                .join("album")
                .join("second.flac")
                .to_string_lossy()
                .into_owned(),
        ]
    );

    // The shared DB itself contains portable refs, never computer A's mount.
    let raw = db::init_portable(&portable_b).await.unwrap();
    assert_eq!(
        raw.favorites("local").await.unwrap(),
        vec![
            "kopuz-root-v1:0:album/second.flac",
            "kopuz-root-v1:0:album/first.flac",
        ]
    );
}
