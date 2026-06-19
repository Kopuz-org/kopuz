//! Targeted persistence ops (issue #347): playlists, favorites, and the queue
//! are written through scoped ops and read back, and active-server writes
//! never touch another server's rows.

use std::path::PathBuf;

use config::{AppConfig, MusicServer, MusicService, SavedServer};
use db::{QueueSnapshot, Source, TrackFilter};
use reader::models::{Track, TrackId};

fn unique_db() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-persist-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

fn server_track(id: &str, title: &str) -> Track {
    Track {
        id: TrackId::Server {
            service: MusicService::YtMusic,
            item_id: id.into(),
        },
        cover: Some("https://img/x.jpg".into()),
        album_id: "ytmusic:album:A".into(),
        title: title.into(),
        artist: "Art".into(),
        album: "YA".into(),
        duration: 200,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec!["Art".into()],
    }
}

async fn seed_active_server(db: &db::Db, id: &str) {
    let cfg = AppConfig {
        servers: vec![SavedServer {
            id: id.into(),
            name: "yt".into(),
            url: "https://music.youtube.com".into(),
            service: MusicService::YtMusic,
            yt_browser: None,
            yt_anonymous: false,
        }],
        server: Some(MusicServer {
            name: "yt".into(),
            url: "https://music.youtube.com".into(),
            service: MusicService::YtMusic,
            access_token: Some("cookie".into()),
            user_id: None,
            id: Some(id.into()),
            yt_browser: None,
            yt_anonymous: false,
        }),
        active_source: config::Source::Server(id.into()),
        ..Default::default()
    };
    db.save_config(&cfg).await.unwrap();
}

#[tokio::test]
async fn recently_played_round_trip() {
    let db = db::init(&unique_db()).await.unwrap();
    let local = Source::Local;

    for k in ["a", "b", "c"] {
        db.push_recent(&local, k).await.unwrap();
    }
    // Re-playing "a" moves it back to the front (monotonic rank, no tie).
    db.push_recent(&local, "a").await.unwrap();
    assert_eq!(
        db.recently_played(&local, 50).await.unwrap(),
        vec!["a", "c", "b"]
    );
    // The limit caps the result, newest first.
    assert_eq!(db.recently_played(&local, 1).await.unwrap(), vec!["a"]);

    // Per-source isolation: a server keeps its own history.
    let srv = Source::Server("srv-1".into());
    db.push_recent(&srv, "VID1").await.unwrap();
    assert_eq!(db.recently_played(&srv, 50).await.unwrap(), vec!["VID1"]);
    assert_eq!(
        db.recently_played(&local, 50).await.unwrap(),
        vec!["a", "c", "b"]
    );
}

#[tokio::test]
async fn playlists_round_trip() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db, "srv-1").await;

    db.upsert_playlist_meta(&Source::Local, "pl-1", "Mine", None, None)
        .await
        .unwrap();
    db.set_playlist_tracks(
        &Source::Local,
        "pl-1",
        &["/music/a.flac".into(), "/music/b.flac".into()],
    )
    .await
    .unwrap();

    let srv = Source::Server("srv-1".into());
    db.upsert_playlist_meta(&srv, "PL1", "Liked", None, Some("urlhex_ab"))
        .await
        .unwrap();
    db.set_playlist_tracks(&srv, "PL1", &["VID1".into(), "VID2".into()])
        .await
        .unwrap();

    db.create_folder("f1", "Folder").await.unwrap();
    db.set_playlist_folder("pl-1", Some("f1")).await.unwrap();

    let store = db.load_playlists(&Source::Local).await.unwrap();
    assert_eq!(store.playlists.len(), 1);
    assert_eq!(store.playlists[0].id, "pl-1");
    assert_eq!(store.playlists[0].name, "Mine");
    assert_eq!(
        store.playlists[0].tracks,
        vec!["/music/a.flac", "/music/b.flac"]
    );
    assert_eq!(store.folders.len(), 1);
    assert_eq!(store.folders[0].playlist_ids, vec!["pl-1"]);

    let store = db
        .load_playlists(&Source::Server("srv-1".into()))
        .await
        .unwrap();
    assert_eq!(store.playlists.len(), 1);
    assert_eq!(store.playlists[0].id, "PL1");
    assert_eq!(store.playlists[0].name, "Liked");
    assert_eq!(store.playlists[0].tracks, vec!["VID1", "VID2"]);
    assert_eq!(store.playlists[0].image_tag.as_deref(), Some("urlhex_ab"));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// Helper: a local playlist's track refs from the loaded store.
async fn local_playlist_tracks(db: &db::Db, id: &str) -> Vec<String> {
    db.load_playlists(&Source::Local)
        .await
        .unwrap()
        .playlists
        .into_iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("playlist {id} missing"))
        .tracks
}

#[tokio::test]
async fn playlist_add_appends_and_dedups() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    db.upsert_playlist_meta(&Source::Local, "pl", "Mine", None, None)
        .await
        .unwrap();
    db.set_playlist_tracks(&Source::Local, "pl", &["a".into(), "b".into()])
        .await
        .unwrap();

    // New tracks append at the end, in order.
    db.add_playlist_tracks(&Source::Local, "pl", &["c".into(), "d".into()])
        .await
        .unwrap();
    assert_eq!(local_playlist_tracks(&db, "pl").await, ["a", "b", "c", "d"]);

    // Already-present refs are skipped; only the genuinely new one is appended.
    db.add_playlist_tracks(&Source::Local, "pl", &["b".into(), "e".into()])
        .await
        .unwrap();
    assert_eq!(
        local_playlist_tracks(&db, "pl").await,
        ["a", "b", "c", "d", "e"]
    );

    // A batch with an internal duplicate adds that ref only once.
    db.add_playlist_tracks(&Source::Local, "pl", &["f".into(), "f".into()])
        .await
        .unwrap();
    assert_eq!(
        local_playlist_tracks(&db, "pl").await,
        ["a", "b", "c", "d", "e", "f"]
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn playlist_add_creates_playlist_if_absent() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    db.add_playlist_tracks(&Source::Local, "fresh", &["x".into()])
        .await
        .unwrap();
    assert_eq!(local_playlist_tracks(&db, "fresh").await, ["x"]);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn playlist_remove_keeps_remaining_order() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    db.upsert_playlist_meta(&Source::Local, "pl", "Mine", None, None)
        .await
        .unwrap();
    db.set_playlist_tracks(
        &Source::Local,
        "pl",
        &["a".into(), "b".into(), "c".into(), "d".into()],
    )
    .await
    .unwrap();

    db.remove_playlist_tracks(&Source::Local, "pl", &["b".into(), "d".into()])
        .await
        .unwrap();
    assert_eq!(local_playlist_tracks(&db, "pl").await, ["a", "c"]);

    // Removing a non-member is a no-op.
    db.remove_playlist_tracks(&Source::Local, "pl", &["z".into()])
        .await
        .unwrap();
    assert_eq!(local_playlist_tracks(&db, "pl").await, ["a", "c"]);

    // A later add still appends after the survivors (no position collision).
    db.add_playlist_tracks(&Source::Local, "pl", &["e".into()])
        .await
        .unwrap();
    assert_eq!(local_playlist_tracks(&db, "pl").await, ["a", "c", "e"]);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// Helper: a folder's playlist_ids from the loaded store, or panic if absent.
async fn folder_members(db: &db::Db, id: &str) -> Vec<String> {
    db.load_playlists(&Source::Local)
        .await
        .unwrap()
        .folders
        .into_iter()
        .find(|f| f.id == id)
        .unwrap_or_else(|| panic!("folder {id} missing"))
        .playlist_ids
}

#[tokio::test]
async fn folder_create_rename_delete() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    db.create_folder("f1", "Rock").await.unwrap();
    let store = db.load_playlists(&Source::Local).await.unwrap();
    assert_eq!(store.folders.len(), 1);
    assert_eq!(store.folders[0].name, "Rock");

    // create on the same id is an upsert of the name (idempotent on id).
    db.create_folder("f1", "Metal").await.unwrap();
    let store = db.load_playlists(&Source::Local).await.unwrap();
    assert_eq!(store.folders.len(), 1, "no duplicate folder row");
    assert_eq!(store.folders[0].name, "Metal");

    db.rename_folder("f1", "Jazz").await.unwrap();
    let store = db.load_playlists(&Source::Local).await.unwrap();
    assert_eq!(store.folders[0].name, "Jazz");

    db.delete_folder("f1").await.unwrap();
    assert!(
        db.load_playlists(&Source::Local)
            .await
            .unwrap()
            .folders
            .is_empty()
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn folder_move_is_not_duplicate() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    db.create_folder("f1", "A").await.unwrap();
    db.create_folder("f2", "B").await.unwrap();

    // Put a playlist in f1, then move it to f2: it must leave f1, not be in both.
    db.set_playlist_folder("p1", Some("f1")).await.unwrap();
    assert_eq!(folder_members(&db, "f1").await, vec!["p1"]);

    db.set_playlist_folder("p1", Some("f2")).await.unwrap();
    assert!(
        folder_members(&db, "f1").await.is_empty(),
        "moving out of f1 clears the old membership"
    );
    assert_eq!(folder_members(&db, "f2").await, vec!["p1"]);

    // None removes it from every folder.
    db.set_playlist_folder("p1", None).await.unwrap();
    assert!(folder_members(&db, "f2").await.is_empty());

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn folder_membership_appends_in_order() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    db.create_folder("f1", "A").await.unwrap();

    db.set_playlist_folder("pA", Some("f1")).await.unwrap();
    db.set_playlist_folder("pB", Some("f1")).await.unwrap();
    db.set_playlist_folder("pC", Some("f1")).await.unwrap();
    assert_eq!(folder_members(&db, "f1").await, vec!["pA", "pB", "pC"]);

    // Re-adding an existing member is idempotent (no duplicate, position kept).
    db.set_playlist_folder("pB", Some("f1")).await.unwrap();
    assert_eq!(folder_members(&db, "f1").await, vec!["pA", "pC", "pB"]);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn deleting_folder_cascades_membership() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    db.create_folder("f1", "A").await.unwrap();
    db.set_playlist_folder("p1", Some("f1")).await.unwrap();

    db.delete_folder("f1").await.unwrap();
    // Folder gone; re-creating it must come back empty (no orphaned membership).
    db.create_folder("f1", "A").await.unwrap();
    assert!(
        folder_members(&db, "f1").await.is_empty(),
        "membership did not survive the folder delete"
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn favorites_round_trip() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db, "srv-1").await;

    db.set_favorite("local", "/music/a.flac", true)
        .await
        .unwrap();
    db.set_favorite("srv-1", "VID1", true).await.unwrap();

    assert_eq!(db.favorites("local").await.unwrap(), vec!["/music/a.flac"]);
    assert_eq!(db.favorites("srv-1").await.unwrap(), vec!["VID1"]);
    assert!(db.is_favorite("local", "/music/a.flac").await.unwrap());
    assert!(db.is_favorite("srv-1", "VID1").await.unwrap());
    assert!(!db.is_favorite("srv-1", "VID2").await.unwrap());

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn fresh_like_sorts_to_top() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db, "srv-1").await;

    db.set_favorite("srv-1", "A", true).await.unwrap();
    db.set_favorite("srv-1", "B", true).await.unwrap();
    db.set_favorite("srv-1", "C", true).await.unwrap();

    // Each new like surfaces at the top, newest first (matches YT's ordering).
    assert_eq!(db.favorites("srv-1").await.unwrap(), vec!["C", "B", "A"]);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn pull_applies_remote_order_and_fresh_like_tops_it() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db, "srv-1").await;

    // A pull stores the remote's order (newest first).
    db.replace_favorites_clean("srv-1", &["X".into(), "Y".into(), "Z".into()])
        .await
        .unwrap();
    assert_eq!(db.favorites("srv-1").await.unwrap(), vec!["X", "Y", "Z"]);

    // A fresh local like lands above the pulled set.
    db.set_favorite("srv-1", "NEW", true).await.unwrap();
    assert_eq!(
        db.favorites("srv-1").await.unwrap(),
        vec!["NEW", "X", "Y", "Z"]
    );

    // A re-pull that reorders the remote set is reflected on existing rows; the
    // still-pending local like (not yet in the remote set) stays on top.
    db.replace_favorites_clean("srv-1", &["Z".into(), "X".into(), "Y".into()])
        .await
        .unwrap();
    assert_eq!(
        db.favorites("srv-1").await.unwrap(),
        vec!["NEW", "Z", "X", "Y"]
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn streaming_favorites_upsert_then_sweep() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db, "srv-1").await;

    // First sync (epoch 1), streamed in two pages — order accumulates.
    db.upsert_favorites_page("srv-1", &["A".into(), "B".into()], 0, 1)
        .await
        .unwrap();
    db.upsert_favorites_page("srv-1", &["C".into()], 2, 1)
        .await
        .unwrap();
    assert_eq!(db.favorites("srv-1").await.unwrap(), vec!["A", "B", "C"]);

    // A pending local like must survive sweeps (push-before-pull).
    db.set_favorite("srv-1", "LOCAL", true).await.unwrap();

    // Second sync (epoch 2): B was unliked remotely (only C, A re-seen, reordered).
    db.upsert_favorites_page("srv-1", &["C".into(), "A".into()], 0, 2)
        .await
        .unwrap();
    db.sweep_favorites("srv-1", 2).await.unwrap();

    // B swept (stale epoch); the dirty local like stays on top; C/A in new order.
    assert_eq!(
        db.favorites("srv-1").await.unwrap(),
        vec!["LOCAL", "C", "A"]
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn liked_music_playlist_is_hidden_from_the_grid() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db, "srv-1").await;
    let srv = Source::Server("srv-1".into());

    // A real playlist surfaces; the reserved "LM" (YT Liked Music) one never
    // does — likes belong to the favorites view, not the playlists grid.
    db.upsert_playlist_meta(&srv, "PLX", "Mix", None, None)
        .await
        .unwrap();
    db.upsert_playlist_meta(&srv, "LM", "Liked Songs", None, None)
        .await
        .unwrap();
    db.set_playlist_tracks(&srv, "LM", &["VID1".into()])
        .await
        .unwrap();

    let store = db.load_playlists(&srv).await.unwrap();
    let ids: Vec<&str> = store.playlists.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["PLX"], "LM must not surface as a playlist");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn queue_round_trips() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let snap = QueueSnapshot {
        version: 1,
        queue: vec![server_track("VID1", "Yt One")],
        current_queue_index: 0,
        progress_secs: 42,
        shuffle_order: vec![0],
        shuffle_enabled: true,
    };
    db.save_queue(&snap).await.unwrap();
    let q = db.load_queue().await.unwrap();
    assert_eq!(q.queue.len(), 1);
    assert_eq!(q.queue[0].title, "Yt One");
    assert_eq!(q.progress_secs, 42);
    assert!(q.shuffle_enabled);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn active_server_writes_never_touch_other_servers_rows() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db, "srv-1").await;

    // Seed ANOTHER server's cache directly.
    let other = Source::Server("srv-other".into());
    db.upsert_tracks(&other, &[server_track("OV1", "Other One")])
        .await
        .unwrap();
    db.upsert_playlist_meta(&other, "OPL", "Other List", None, None)
        .await
        .unwrap();
    db.set_playlist_tracks(&other, "OPL", &["OV1".into()])
        .await
        .unwrap();
    db.set_favorite("srv-other", "OV1", true).await.unwrap();

    // A full sync-style write cycle for the ACTIVE server (srv-1)...
    let active = Source::Server("srv-1".into());
    db.upsert_tracks(
        &active,
        &[
            server_track("VID1", "Yt One"),
            server_track("VID2", "Yt Two"),
        ],
    )
    .await
    .unwrap();
    db.prune_source(&active, &["VID1".into(), "VID2".into()], &[])
        .await
        .unwrap();
    db.upsert_playlist_meta(&active, "PL1", "Liked", None, None)
        .await
        .unwrap();
    db.set_playlist_tracks(&active, "PL1", &["VID1".into()])
        .await
        .unwrap();
    db.upsert_playlist_meta(&active, "TMP", "Scratch", None, None)
        .await
        .unwrap();
    db.delete_playlist(&active, "TMP").await.unwrap();

    // ...must leave srv-other's rows completely intact.
    let other_count = db
        .tracks_count(&TrackFilter::new(Source::Server("srv-other".into())))
        .await
        .unwrap();
    assert_eq!(other_count, 1, "other server's tracks survived");
    assert_eq!(
        db.favorites("srv-other").await.unwrap(),
        vec!["OV1"],
        "other server's favorites survived"
    );

    // Each server's playlists are scoped to that source — srv-1 first...
    let store = db
        .load_playlists(&Source::Server("srv-1".into()))
        .await
        .unwrap();
    assert_eq!(store.playlists.len(), 1);
    assert_eq!(store.playlists[0].id, "PL1");
    assert_eq!(store.playlists[0].tracks, vec!["VID1"]);

    // ...and srv-other's playlist survived untouched.
    let store = db
        .load_playlists(&Source::Server("srv-other".into()))
        .await
        .unwrap();
    assert_eq!(
        store.playlists.len(),
        1,
        "other server's playlists survived"
    );
    assert_eq!(store.playlists[0].id, "OPL");
    assert_eq!(store.playlists[0].tracks, vec!["OV1"]);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}
