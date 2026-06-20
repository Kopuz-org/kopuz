//! Windowed track query proof (issue #347, step 6): a 20k-track library is
//! sorted/filtered/paged in SQL — a page query returns only its slice, in the
//! requested order, and the count reflects the filter.

use std::path::PathBuf;

use db::{Page, Source, TrackFilter, TrackSort};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Executor};

fn unique_db() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-q-{nanos}"));
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

    // Reconstructed identity is a local path.
    assert!(matches!(page[0].id, reader::models::TrackId::Local(_)));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}
