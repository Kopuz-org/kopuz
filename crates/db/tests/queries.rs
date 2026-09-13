//! Windowed track query proof (issue #347, step 6): a 20k-track library is
//! sorted/filtered/paged in SQL — a page query returns only its slice, in the
//! requested order, and the count reflects the filter.

use std::path::PathBuf;

use config::{SortCriterion, SortDirection, TrackSortField};
use db::{Page, Source, TrackFilter, TrackSort};
use reader::models::{Album, Track, TrackId};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Executor};

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
    let dir = std::env::temp_dir().join(format!("kopuz-q-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

const N: usize = 20_000;

async fn seed(db_path: &std::path::Path) {
    let mut conn = SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap();
    conn.execute("BEGIN").await.unwrap();
    for i in 0..N {
        // Artist/album buckets give the sort something to order within; titles
        // are zero-padded so lexical order matches numeric.
        let key = format!("/music/{i:05}.flac");
        let title = format!("Track {i:05}");
        let artist = format!("Artist {:03}", i % 50);
        let album = format!("Album {:03}", i % 200);
        sqlx::query(
            "INSERT INTO tracks (source, track_key, title, artist, album, artists_json) \
             VALUES ('local', ?1, ?2, ?3, ?4, '[]')",
        )
        .bind(&key)
        .bind(&title)
        .bind(&artist)
        .bind(&album)
        .execute(&mut conn)
        .await
        .unwrap();
    }
    conn.execute("COMMIT").await.unwrap();
}

#[tokio::test]
async fn windowed_queries_over_20k_tracks() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed(&db_path).await;

    let local = TrackFilter::new(Source::Local);

    // Count reflects the whole library.
    assert_eq!(db.tracks_count(&local).await.unwrap(), N as u32);

    // A page returns exactly its slice, in Title order.
    let by_title = TrackFilter {
        sort: TrackSort::Title,
        ..local.clone()
    };
    let page = db
        .tracks_page(
            &by_title,
            Page {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.len(), 100);
    assert_eq!(page[0].title, "Track 00000");
    assert_eq!(page[99].title, "Track 00099");

    // A deeper window starts where it should — only that slice is materialized.
    let mid = db
        .tracks_page(
            &by_title,
            Page {
                offset: 12_345,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(mid.len(), 10);
    assert_eq!(mid[0].title, "Track 12345");
    assert_eq!(mid[9].title, "Track 12354");

    // Search narrows both the page and the count. "Artist 007" tags every 50th
    // track → 400 of them.
    let search = TrackFilter {
        search: "Artist 007".into(),
        sort: TrackSort::Title,
        ..local.clone()
    };
    assert_eq!(db.tracks_count(&search).await.unwrap(), (N / 50) as u32);
    let hits = db
        .tracks_page(
            &search,
            Page {
                offset: 0,
                limit: 5,
            },
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 5);
    assert!(hits.iter().all(|t| t.artist == "Artist 007"));

    // Sort actually orders: first row by Artist differs from first by Title.
    let by_artist = db
        .tracks_page(
            &TrackFilter {
                sort: TrackSort::Artist,
                ..local.clone()
            },
            Page {
                offset: 0,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(by_artist[0].artist, "Artist 000");

    let stacked = db
        .tracks_page(
            &TrackFilter {
                sort: TrackSort::Fields(vec![
                    SortCriterion::new(TrackSortField::Artist, SortDirection::Asc),
                    SortCriterion::new(TrackSortField::Title, SortDirection::Desc),
                ]),
                ..local.clone()
            },
            Page {
                offset: 0,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(stacked[0].artist, "Artist 000");
    assert_eq!(stacked[0].title, "Track 19950");

    let fallback = db
        .tracks_page(
            &TrackFilter {
                sort: TrackSort::Fields(Vec::new()),
                ..local.clone()
            },
            Page {
                offset: 0,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(fallback[0].artist, "Artist 000");

    // Reconstructed identity is a local path.
    assert!(matches!(page[0].id, reader::models::TrackId::Local(_)));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

fn album(id: &str, title: &str, artist: &str) -> Album {
    Album {
        id: id.into(),
        title: title.into(),
        artist: artist.into(),
        genre: String::new(),
        year: 2000,
        cover_path: None,
        manual_cover: false,
    }
}

fn track(path: &str, album_id: &str) -> Track {
    Track {
        id: TrackId::Local(PathBuf::from(path)),
        cover: None,
        album_id: album_id.into(),
        title: path.into(),
        artist: "Artist".into(),
        album: album_id.into(),
        duration: 1,
        khz: 44100,
        bitrate: 900,
        track_number: Some(1),
        disc_number: Some(1),
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: Vec::new(),
    }
}

/// Recently-added is its own ordering, not the album listing read backwards:
/// the listing is alphabetical, so reversing it only ever surfaces the end of
/// the alphabet (issue #691, where a CJK-heavy library saw the same albums no
/// matter what was scanned).
#[tokio::test]
async fn recently_added_albums_order_by_date_added() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    // Written oldest-first, and deliberately neither alphabetical nor its
    // reverse, so neither ordering can pass by accident.
    for (id, artist) in [("bee", "Bea"), ("cee", "Cara"), ("ann", "Ann")] {
        db.upsert_albums(&Source::Local, &[album(id, id, artist)])
            .await
            .unwrap();
        db.upsert_tracks(&Source::Local, &[track(&format!("/music/{id}.flac"), id)])
            .await
            .unwrap();
    }

    let ids =
        |albums: Vec<reader::Album>| -> Vec<String> { albums.into_iter().map(|a| a.id).collect() };

    // Unstamped rows (a library not rescanned since the added_at migration, or
    // any server source) still fall back to insertion order.
    assert_eq!(
        ids(db.albums_recently_added(&Source::Local, 10).await.unwrap()),
        ["ann", "cee", "bee"]
    );
    assert_eq!(
        ids(db.albums_recently_added(&Source::Local, 2).await.unwrap()),
        ["ann", "cee"]
    );

    // A stamp outranks insertion order, and it is the album's newest track that
    // decides: stamping the oldest album's track pulls that album to the front.
    db.stamp_added_at(&Source::Local, &[("/music/bee.flac".into(), 1_700_000_000)])
        .await
        .unwrap();
    assert_eq!(
        ids(db.albums_recently_added(&Source::Local, 10).await.unwrap()),
        ["bee", "ann", "cee"]
    );

    // Stamps are written once: a rescan after a tag edit bumped the file's
    // mtime must not make old music look new.
    db.stamp_added_at(&Source::Local, &[("/music/cee.flac".into(), 1_800_000_000)])
        .await
        .unwrap();
    db.stamp_added_at(&Source::Local, &[("/music/cee.flac".into(), 1_900_000_000)])
        .await
        .unwrap();
    let filter = TrackFilter {
        sort: TrackSort::DateAdded,
        ..TrackFilter::new(Source::Local)
    };
    let by_date = db
        .tracks_page(
            &filter,
            Page {
                offset: 0,
                limit: 3,
            },
        )
        .await
        .unwrap();
    assert_eq!(by_date[0].title, "/music/cee.flac");
    assert_eq!(by_date[1].title, "/music/bee.flac");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}
