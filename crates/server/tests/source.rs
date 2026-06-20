//! The `MediaSource` facade (issue #347, Phase 2) over a real temp DB. Exercises
//! the local impl end-to-end through the public trait — `create_playlist` /
//! `add_to_playlist` / `set_favorite` route to the DB and read back — so the
//! facade's wiring is covered without a GUI. The remote impl needs a live
//! server and is verified against real accounts instead.

use std::path::PathBuf;

use db::Source;
use server::source;

fn unique_db() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-source-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

#[tokio::test]
async fn local_create_then_add_playlist_round_trips() {
    let db = db::init(&unique_db()).await.unwrap();
    let src = source::local(db.clone());

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
    let src = source::local(db.clone());

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
