//! Per-server favorites with optimistic dirty tracking (issue #347, step 8).

use std::path::PathBuf;

fn unique_db() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-fav-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

#[tokio::test]
async fn favorites_dirty_and_reconcile() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    // A local like writes a dirty row, visible immediately.
    db.set_favorite("local", "/music/a.flac", true)
        .await
        .unwrap();
    assert!(db.is_favorite("local", "/music/a.flac").await.unwrap());
    assert_eq!(
        db.dirty_favorites("local").await.unwrap(),
        vec!["/music/a.flac".to_string()]
    );

    // Idempotent re-like.
    db.set_favorite("local", "/music/a.flac", true)
        .await
        .unwrap();
    assert_eq!(db.favorites("local").await.unwrap().len(), 1);

    // Pushed to remote → no longer dirty, still a favorite.
    db.clear_favorite_dirty("local", "/music/a.flac")
        .await
        .unwrap();
    assert!(db.dirty_favorites("local").await.unwrap().is_empty());
    assert!(db.is_favorite("local", "/music/a.flac").await.unwrap());

    // Unlike of a SYNCED favorite hides it immediately but leaves a
    // pending-unlike tombstone for the reconciler to push.
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
    // Pushing the unlike resolves the tombstone away.
    db.clear_favorite_dirty("local", "/music/a.flac")
        .await
        .unwrap();
    assert!(db.dirty_unlikes("local").await.unwrap().is_empty());

    // Unlike of a NEVER-PUSHED like just disappears (nothing to push).
    db.set_favorite("local", "/music/x.flac", true)
        .await
        .unwrap();
    db.set_favorite("local", "/music/x.flac", false)
        .await
        .unwrap();
    assert!(db.dirty_unlikes("local").await.unwrap().is_empty());
    assert!(db.dirty_favorites("local").await.unwrap().is_empty());

    // Re-like of a tombstone resurrects it as a pending-like.
    db.set_favorite("local", "/music/y.flac", true)
        .await
        .unwrap();
    db.clear_favorite_dirty("local", "/music/y.flac")
        .await
        .unwrap(); // synced
    db.set_favorite("local", "/music/y.flac", false)
        .await
        .unwrap(); // tombstone
    db.set_favorite("local", "/music/y.flac", true)
        .await
        .unwrap(); // re-like
    assert!(db.is_favorite("local", "/music/y.flac").await.unwrap());
    assert_eq!(
        db.dirty_favorites("local").await.unwrap(),
        vec!["/music/y.flac".to_string()]
    );
    db.set_favorite("local", "/music/y.flac", false)
        .await
        .unwrap();

    // Per-server isolation: a YT like doesn't touch local.
    db.set_favorite("srv-1", "VID9", true).await.unwrap();
    assert!(db.is_favorite("srv-1", "VID9").await.unwrap());
    assert!(!db.is_favorite("local", "VID9").await.unwrap());

    // Reconcile pull: clean rows absent remotely go; dirty rows survive (not
    // pushed yet); the remote set is added clean.
    db.set_favorite("srv-1", "VID_dirty", true).await.unwrap(); // dirty, not in remote
    db.clear_favorite_dirty("srv-1", "VID9").await.unwrap(); // VID9 now clean
    db.replace_favorites_clean("srv-1", &["VID9".into(), "VID_new".into()])
        .await
        .unwrap();
    let mut favs = db.favorites("srv-1").await.unwrap();
    favs.sort();
    assert_eq!(favs, vec!["VID9", "VID_dirty", "VID_new"]); // dirty kept, new added, none clean-dropped
    assert_eq!(
        db.dirty_favorites("srv-1").await.unwrap(),
        vec!["VID_dirty".to_string()]
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}
