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
