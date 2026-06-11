//! Full in-memory ↔ DB round-trips (issue #347): the runtime now loads/saves
//! Library, PlaylistStore, FavoritesStore, and the queue through the DB instead
//! of JSON. A save of a freshly-loaded value must not lose or prune anything.

use std::collections::HashMap;
use std::path::PathBuf;

use config::{AppConfig, MusicServer, MusicService, SavedServer};
use db::QueueSnapshot;
use reader::models::{
    Album, FavoritesStore, JellyfinPlaylist, Library, Playlist, PlaylistFolder, PlaylistStore,
    Track, TrackId,
};

fn unique_db() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("kopuz-persist-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

fn local_track(path: &str, title: &str) -> Track {
    Track {
        id: TrackId::Local(PathBuf::from(path)),
        cover: None,
        album_id: "alb-local".into(),
        title: title.into(),
        artist: "Loc".into(),
        album: "L".into(),
        duration: 100,
        khz: 44100,
        bitrate: 900,
        track_number: Some(1),
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec!["Loc".into()],
    }
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

async fn seed_active_server(db: &db::Db) {
    let mut cfg = AppConfig::default();
    cfg.servers = vec![SavedServer {
        id: "srv-1".into(),
        name: "yt".into(),
        url: "https://music.youtube.com".into(),
        service: MusicService::YtMusic,
        yt_browser: None,
        yt_anonymous: false,
    }];
    cfg.server = Some(MusicServer {
        name: "yt".into(),
        url: "https://music.youtube.com".into(),
        service: MusicService::YtMusic,
        access_token: Some("cookie".into()),
        user_id: None,
        id: Some("srv-1".into()),
        yt_browser: None,
        yt_anonymous: false,
    });
    cfg.active_server_id = Some("srv-1".into());
    db.save_config(&cfg).await.unwrap();
}

fn sample_library() -> Library {
    let mut server_artist_images = HashMap::new();
    server_artist_images.insert("art".to_string(), "https://img/art.jpg".to_string());
    Library {
        root_paths: vec![PathBuf::from("/music")],
        tracks: vec![local_track("/music/a.flac", "A"), local_track("/music/b.flac", "B")],
        albums: vec![Album {
            id: "alb-local".into(),
            title: "L".into(),
            artist: "Loc".into(),
            genre: "Rock".into(),
            year: 2020,
            cover_path: Some(PathBuf::from("/cache/l.png")),
            manual_cover: false,
        }],
        jellyfin_tracks: vec![server_track("VID1", "Yt One"), server_track("VID2", "Yt Two")],
        jellyfin_albums: vec![Album {
            id: "ytmusic:album:A".into(),
            title: "YA".into(),
            artist: "Art".into(),
            genre: String::new(),
            year: 0,
            cover_path: None,
            manual_cover: false,
        }],
        jellyfin_genres: Vec::new(),
        last_yt_sync_at: Some(1_700_000_000),
        last_yt_playlists_sync_at: Some(1_700_000_500),
        server_artist_images,
        local_artist_images: HashMap::new(),
        custom_artist_images: HashMap::new(),
    }
}

#[tokio::test]
async fn library_round_trips_without_loss() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db).await;

    let lib = sample_library();
    db.save_library(&lib).await.unwrap();

    let loaded = db.load_library().await.unwrap();
    assert_eq!(loaded.tracks.len(), 2, "local tracks");
    assert_eq!(loaded.jellyfin_tracks.len(), 2, "server tracks");
    assert_eq!(loaded.albums.len(), 1);
    assert_eq!(loaded.jellyfin_albums.len(), 1);
    assert_eq!(loaded.last_yt_sync_at, Some(1_700_000_000));
    assert_eq!(loaded.last_yt_playlists_sync_at, Some(1_700_000_500));
    assert_eq!(
        loaded.server_artist_images.get("art").map(String::as_str),
        Some("https://img/art.jpg")
    );
    // Server track kept its cover + typed identity.
    let yt = loaded
        .jellyfin_tracks
        .iter()
        .find(|t| t.title == "Yt One")
        .unwrap();
    assert_eq!(yt.cover.as_deref(), Some("https://img/x.jpg"));
    assert!(matches!(yt.id, TrackId::Server { .. }));

    // Saving the freshly-loaded library prunes nothing.
    db.save_library(&loaded).await.unwrap();
    let again = db.load_library().await.unwrap();
    assert_eq!(again.tracks.len(), 2);
    assert_eq!(again.jellyfin_tracks.len(), 2);

    // Dropping a track then saving prunes exactly it.
    let mut shrunk = again;
    shrunk.tracks.retain(|t| t.title == "A");
    db.save_library(&shrunk).await.unwrap();
    assert_eq!(db.load_library().await.unwrap().tracks.len(), 1);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn playlists_favorites_queue_round_trip() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed_active_server(&db).await;

    let store = PlaylistStore {
        playlists: vec![Playlist {
            id: "pl-1".into(),
            name: "Mine".into(),
            tracks: vec![PathBuf::from("/music/a.flac"), PathBuf::from("/music/b.flac")],
            cover_path: None,
        }],
        jellyfin_playlists: vec![JellyfinPlaylist {
            id: "LM".into(),
            name: "Liked".into(),
            tracks: vec!["VID1".into(), "VID2".into()],
            image_tag: Some("urlhex_ab".into()),
            cover_path: None,
        }],
        folders: vec![PlaylistFolder {
            id: "f1".into(),
            name: "Folder".into(),
            playlist_ids: vec!["pl-1".into()],
        }],
    };
    db.save_playlists(&store).await.unwrap();
    let pl = db.load_playlists().await.unwrap();
    assert_eq!(pl.playlists.len(), 1);
    assert_eq!(pl.playlists[0].tracks.len(), 2);
    assert_eq!(pl.jellyfin_playlists.len(), 1);
    assert_eq!(pl.jellyfin_playlists[0].tracks, vec!["VID1", "VID2"]);
    assert_eq!(pl.folders.len(), 1);
    assert_eq!(pl.folders[0].playlist_ids, vec!["pl-1"]);

    let favs = FavoritesStore {
        local_favorites: vec![PathBuf::from("/music/a.flac")],
        jellyfin_favorites: vec!["VID1".into(), "VID2".into()],
    };
    db.save_favorites_store(&favs).await.unwrap();
    let loaded = db.load_favorites_store().await.unwrap();
    assert_eq!(loaded.local_favorites, vec![PathBuf::from("/music/a.flac")]);
    let mut jf = loaded.jellyfin_favorites;
    jf.sort();
    assert_eq!(jf, vec!["VID1", "VID2"]);

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
