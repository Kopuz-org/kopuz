//! Per-server favorites with optimistic dirty tracking (issue #347, step 8).

use std::path::PathBuf;

fn unique_db() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("kopuz-fav-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

#[tokio::test]
async fn favorites_dirty_and_reconcile() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    db.set_favorite("local", "/music/a.flac", true)
        .await
        .unwrap();
    assert!(db.is_favorite("local", "/music/a.flac").await.unwrap());
    assert_eq!(
        db.dirty_favorites("local").await.unwrap(),
        vec!["/music/a.flac".to_string()]
    );

    db.set_favorite("local", "/music/a.flac", true)
        .await
        .unwrap();
    assert_eq!(db.favorites("local").await.unwrap().len(), 1);

    db.clear_favorite_dirty("local", "/music/a.flac")
        .await
        .unwrap();
    assert!(db.dirty_favorites("local").await.unwrap().is_empty());
    assert!(db.is_favorite("local", "/music/a.flac").await.unwrap());

    db.set_favorite("local", "/music/a.flac", false)
        .await
        .unwrap();
    assert!(!db.is_favorite("local", "/music/a.flac").await.unwrap());
    assert!(
        !db.favorites("local")
            .await
            .unwrap()
            .contains(&"/music/a.flac".to_string())
    );
    assert_eq!(
        db.dirty_unlikes("local").await.unwrap(),
        vec!["/music/a.flac".to_string()]
    );

    db.clear_favorite_dirty("local", "/music/a.flac")
        .await
        .unwrap();
    assert!(db.dirty_unlikes("local").await.unwrap().is_empty());

    db.set_favorite("local", "/music/x.flac", true)
        .await
        .unwrap();
    db.set_favorite("local", "/music/x.flac", false)
        .await
        .unwrap();
    assert!(db.dirty_unlikes("local").await.unwrap().is_empty());
    assert!(db.dirty_favorites("local").await.unwrap().is_empty());

    db.set_favorite("local", "/music/y.flac", true)
        .await
        .unwrap();
    db.clear_favorite_dirty("local", "/music/y.flac")
        .await
        .unwrap();
    db.set_favorite("local", "/music/y.flac", false)
        .await
        .unwrap();
    db.set_favorite("local", "/music/y.flac", true)
        .await
        .unwrap();
    assert!(db.is_favorite("local", "/music/y.flac").await.unwrap());
    assert_eq!(
        db.dirty_favorites("local").await.unwrap(),
        vec!["/music/y.flac".to_string()]
    );
    db.set_favorite("local", "/music/y.flac", false)
        .await
        .unwrap();

    db.set_favorite("srv-1", "VID9", true).await.unwrap();
    assert!(db.is_favorite("srv-1", "VID9").await.unwrap());
    assert!(!db.is_favorite("local", "VID9").await.unwrap());

    db.set_favorite("srv-1", "VID_dirty", true).await.unwrap();
    db.clear_favorite_dirty("srv-1", "VID9").await.unwrap();
    db.replace_favorites_clean("srv-1", &["VID9".into(), "VID_new".into()])
        .await
        .unwrap();
    let mut favs = db.favorites("srv-1").await.unwrap();
    favs.sort();
    assert_eq!(favs, vec!["VID9", "VID_dirty", "VID_new"]);
    assert_eq!(
        db.dirty_favorites("srv-1").await.unwrap(),
        vec!["VID_dirty".to_string()]
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}
