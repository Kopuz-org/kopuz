//! A server's play counts and ratings beside kopuz's own counts.

use std::path::PathBuf;

use db::{ServerTrackStats, Source};

fn unique_db() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("kopuz-server-stats-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

fn stat(key: &str, plays: Option<u32>, last: Option<i64>, rating: Option<u8>) -> ServerTrackStats {
    ServerTrackStats {
        track_key: key.into(),
        play_count: plays,
        last_played_at: last,
        rating,
    }
}

async fn bump(db: &db::Db, source: &Source, key: &str, times: usize) {
    for _ in 0..times {
        db.bump_listen_count(source, key).await.unwrap();
    }
}

#[tokio::test]
async fn the_higher_count_wins_and_counts_are_never_summed() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("nav".into());

    // `a`: the server saw more (other clients). `b`: kopuz played it since the last sync.
    bump(&db, &source, "a", 2).await;
    bump(&db, &source, "b", 4).await;
    db.replace_server_track_stats(
        &source,
        &[
            stat("a", Some(9), Some(100), None),
            stat("b", Some(3), Some(50), None),
            stat("c", Some(5), Some(70), None),
        ],
    )
    .await
    .unwrap();

    let history = db.play_history(&source).await.unwrap();
    let count = |key: &str| history.iter().find(|r| r.track_key == key).map(|r| r.count);
    assert_eq!(count("a"), Some(9));
    assert_eq!(count("b"), Some(4));
    assert_eq!(
        count("c"),
        Some(5),
        "a track only the server played counts too"
    );
    assert_eq!(history.first().map(|r| r.track_key.as_str()), Some("a"));
}

#[tokio::test]
async fn ratings_are_replaced_by_a_sync_and_set_one_by_one() {
    let db = db::init(&unique_db()).await.unwrap();
    let source = Source::Server("nav".into());

    db.replace_server_track_stats(
        &source,
        &[
            stat("a", None, None, Some(4)),
            stat("b", Some(1), None, Some(2)),
        ],
    )
    .await
    .unwrap();
    db.set_server_rating(&source, "c", Some(5)).await.unwrap();
    db.set_server_rating(&source, "b", None).await.unwrap();

    let ratings = db.server_ratings(&source).await.unwrap();
    assert_eq!(ratings.get("a"), Some(&4));
    assert_eq!(ratings.get("b"), None, "cleared");
    assert_eq!(ratings.get("c"), Some(&5));

    db.replace_server_track_stats(&source, &[stat("a", None, None, Some(1))])
        .await
        .unwrap();
    let ratings = db.server_ratings(&source).await.unwrap();
    assert_eq!(ratings.len(), 1, "a sync's report replaces the last one");
    assert_eq!(ratings.get("a"), Some(&1));
    assert!(
        db.server_ratings(&Source::Server("other".into()))
            .await
            .unwrap()
            .is_empty()
    );
}
