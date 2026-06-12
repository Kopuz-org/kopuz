//! Batch upsert + scan-reconcile prune (issue #347, step 7).

use std::path::PathBuf;

use db::{Page, Source, TrackFilter};
use reader::models::{Track, TrackId};

fn unique_db() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-w-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

fn local(path: &str, title: &str) -> Track {
    Track {
        id: TrackId::Local(PathBuf::from(path)),
        cover: None,
        album_id: "alb".into(),
        title: title.into(),
        artist: "Artist".into(),
        album: "Album".into(),
        duration: 123,
        khz: 44100,
        bitrate: 900,
        track_number: Some(2),
        disc_number: Some(1),
        musicbrainz_release_id: Some("mbr".into()),
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec!["Artist".into(), "Feat".into()],
    }
}

#[tokio::test]
async fn upsert_then_prune() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let a = local("/music/a.flac", "A");
    let b = local("/music/b.flac", "B");
    let c = local("/other/c.flac", "C");
    db.upsert_tracks(&Source::Local, &[a.clone(), b.clone(), c.clone()])
        .await
        .unwrap();

    let filter = TrackFilter::new(Source::Local);
    assert_eq!(db.tracks_count(&filter).await.unwrap(), 3);

    // Upsert is idempotent on identity: re-inserting "A" with a new title updates
    // the existing row rather than adding one.
    let mut a2 = a.clone();
    a2.title = "A (remastered)".into();
    db.upsert_tracks(&Source::Local, &[a2]).await.unwrap();
    assert_eq!(db.tracks_count(&filter).await.unwrap(), 3);

    // Round-trip preserves the typed fields.
    let page = db
        .tracks_page(&filter, Page { offset: 0, limit: 10 })
        .await
        .unwrap();
    let got = page.iter().find(|t| t.title.starts_with("A")).unwrap();
    assert_eq!(got.title, "A (remastered)");
    assert_eq!(got.track_number, Some(2));
    assert_eq!(got.musicbrainz_release_id.as_deref(), Some("mbr"));
    assert_eq!(got.artists, vec!["Artist".to_string(), "Feat".to_string()]);
    assert!(matches!(got.id, TrackId::Local(_)));

    // Prune the local source keeping "a.flac" + "c.flac" → "b.flac" goes (the
    // scan-reconcile step: anything not in the last scan's keep-set).
    let keep = vec!["/music/a.flac".to_string(), "/other/c.flac".to_string()];
    db.prune_source(&Source::Local, &keep, &[]).await.unwrap();
    assert_eq!(db.tracks_count(&filter).await.unwrap(), 2);
    let remaining: Vec<String> = db
        .tracks_page(&filter, Page { offset: 0, limit: 10 })
        .await
        .unwrap()
        .iter()
        .filter_map(|t| t.id.local_path().map(|p| p.to_string_lossy().into_owned()))
        .collect();
    assert!(remaining.contains(&"/music/a.flac".to_string()));
    assert!(remaining.contains(&"/other/c.flac".to_string()));
    assert!(!remaining.contains(&"/music/b.flac".to_string()));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}
