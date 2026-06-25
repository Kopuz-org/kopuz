//! The `MediaSource` facade (issue #347, Phase 2) over a real temp DB. Exercises
//! the local impl end-to-end through the public trait — `create_playlist` /
//! `add_to_playlist` / `set_favorite` route to the DB and read back — so the
//! facade's wiring is covered without a GUI. The remote impl needs a live
//! server and is verified against real accounts instead.

use std::path::PathBuf;

use config::MusicService;
use db::Source;
use server::source;

fn unique_suffix() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos}-{n}")
}

fn unique_db() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kopuz-source-{}", unique_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

fn unique_server_id(prefix: &str) -> String {
    format!("{prefix}-{}", unique_suffix())
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

#[tokio::test]
async fn signin_cleanup_jellyfin_is_noop() {
    let db = db::init(&unique_db()).await.unwrap();
    let id = unique_server_id("jellyfin-cleanup");
    let yt_profile = server::ytmusic::isolated_profile::profile_dir(&id);
    let sc_profile = server::soundcloud::signin::profile_dir(&id);
    assert!(!yt_profile.exists());
    assert!(!sc_profile.exists());

    let src = source::signin_cleanup(db, &id, MusicService::Jellyfin);

    assert!(src.cleanup_signin().await.is_ok());
    assert!(!yt_profile.exists());
    assert!(!sc_profile.exists());
}

#[tokio::test]
async fn signin_cleanup_ytmusic_removes_profile_dir() {
    let db = db::init(&unique_db()).await.unwrap();
    let id = unique_server_id("yt-cleanup");
    let profile = server::ytmusic::isolated_profile::profile_dir(&id);
    let _ = std::fs::remove_dir_all(&profile);

    let src = source::signin_cleanup(db.clone(), &id, MusicService::YtMusic);
    assert!(src.cleanup_signin().await.is_ok());
    assert!(!profile.exists());

    std::fs::create_dir_all(&profile).unwrap();
    assert!(profile.is_dir());

    let src = source::signin_cleanup(db, &id, MusicService::YtMusic);
    assert!(src.cleanup_signin().await.is_ok());
    assert!(!profile.exists());
}

#[tokio::test]
async fn signin_cleanup_soundcloud_removes_profile_dir() {
    let db = db::init(&unique_db()).await.unwrap();
    let id = unique_server_id("sc-cleanup");
    let profile = server::soundcloud::signin::profile_dir(&id);
    let _ = std::fs::remove_dir_all(&profile);

    let src = source::signin_cleanup(db.clone(), &id, MusicService::SoundCloud);
    assert!(src.cleanup_signin().await.is_ok());
    assert!(!profile.exists());

    std::fs::create_dir_all(&profile).unwrap();
    assert!(profile.is_dir());

    let src = source::signin_cleanup(db, &id, MusicService::SoundCloud);
    assert!(src.cleanup_signin().await.is_ok());
    assert!(!profile.exists());
}
