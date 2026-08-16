//! Batch upsert + scan-reconcile prune (issue #347, step 7).

use std::path::PathBuf;

use db::{Page, Source, TrackFilter};
use reader::models::{Album, Track, TrackId};

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
    let dir = std::env::temp_dir().join(format!("kopuz-w-{pid}-{nanos}-{seq}"));
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
        .tracks_page(
            &filter,
            Page {
                offset: 0,
                limit: 10,
            },
        )
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
        .tracks_page(
            &filter,
            Page {
                offset: 0,
                limit: 10,
            },
        )
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

#[tokio::test]
async fn automatic_cover_update_preserves_concurrent_manual_cover() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let album = Album {
        id: "album".into(),
        title: "Album".into(),
        artist: "Artist".into(),
        genre: "Unknown".into(),
        year: 0,
        cover_path: None,
        manual_cover: false,
    };
    db.upsert_albums(&Source::Local, &[album]).await.unwrap();

    assert!(
        db.update_album_cover_if_not_manual(&Source::Local, "album", "/auto.jpg")
            .await
            .unwrap()
    );
    db.update_album_cover(&Source::Local, "album", Some("/manual.jpg"), true)
        .await
        .unwrap();
    assert!(
        !db.update_album_cover_if_not_manual(&Source::Local, "album", "/late-auto.jpg")
            .await
            .unwrap()
    );

    let stored = db.album(&Source::Local, "album").await.unwrap().unwrap();
    assert_eq!(stored.cover_path, Some(PathBuf::from("/manual.jpg")));
    assert!(stored.manual_cover);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// First coverage of the metadata-cache API: `meta_keys_since` returns keys of
/// the requested kind written within the window, and a re-put refreshes the
/// `fetched_at` stamp (the artist-photo-miss TTL relies on both).
#[tokio::test]
async fn meta_keys_since_windows_by_kind_and_age() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    db.meta_put("artist a", "artist_photo_miss", "")
        .await
        .unwrap();
    db.meta_put("artist b", "other_kind", "").await.unwrap();

    let fresh = db
        .meta_keys_since("artist_photo_miss", 86_400)
        .await
        .unwrap();
    assert_eq!(fresh, vec!["artist a".to_string()], "same kind, in window");

    // `fetched_at` can't be backdated through the public API, so expiry is
    // simulated with a negative window: `fetched_at >= unixepoch() + 1` never
    // matches a just-written row.
    let expired = db.meta_keys_since("artist_photo_miss", -1).await.unwrap();
    assert!(expired.is_empty(), "an aged-out row stops matching");

    // Re-putting refreshes the stamp — the row is fresh again by upsert.
    db.meta_put("artist a", "artist_photo_miss", "")
        .await
        .unwrap();
    let fresh = db.meta_keys_since("artist_photo_miss", 1).await.unwrap();
    assert_eq!(fresh, vec!["artist a".to_string()]);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}
