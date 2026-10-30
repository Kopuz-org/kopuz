//! Local media-source operations tested through the public trait against SQLite.

use std::path::PathBuf;

use db::Source;
use reader::{Track, TrackId};
use server::source;

fn track(id: TrackId) -> Track {
    Track {
        id,
        cover: None,
        album_id: String::new(),
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        duration: 0,
        khz: 0,
        bitrate: 0,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        credits: Vec::new(),
        artists: Vec::new(),
        replay_gain: config::ReplayGainInfo::default(),
        explicit: false,
        plays: None,
        counterpart: None,
    }
}

fn unique_db() -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("kopuz-source-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

#[tokio::test]
async fn local_create_then_add_playlist_round_trips() {
    let db = db::init(&unique_db()).await.unwrap();
    let src = source::local(db.clone(), Source::default());

    let id = src
        .create_playlist("Road Trip", &["/music/a.flac".into()])
        .await
        .unwrap();

    let store = db.load_playlists(&Source::default()).await.unwrap();
    let pl = store
        .playlists
        .iter()
        .find(|p| p.id == id)
        .expect("created playlist present");
    assert_eq!(pl.name, "Road Trip");
    assert_eq!(pl.tracks, vec!["/music/a.flac".to_string()]);

    let landed = src
        .add_to_playlist(&id, &["/music/b.flac".into(), "/music/a.flac".into()])
        .await
        .unwrap();
    assert_eq!(landed.len(), 2);

    let store = db.load_playlists(&Source::default()).await.unwrap();
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
    let src = source::local(db.clone(), Source::default());

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
async fn record_favorite_writes_a_clean_local_row_and_reverts() {
    let db = db::init(&unique_db()).await.unwrap();
    let src = source::local(db.clone(), Source::default());
    let t = track(TrackId::Local("/music/x.flac".into()));

    src.record_favorite(&t, true).await.unwrap();
    assert!(
        db.favorites("local")
            .await
            .unwrap()
            .contains(&"/music/x.flac".to_string())
    );
    assert!(db.dirty_favorites("local").await.unwrap().is_empty());

    src.record_favorite(&t, false).await.unwrap();
    assert!(!src.is_favorite("/music/x.flac").await);
    assert!(db.dirty_favorites("local").await.unwrap().is_empty());
    assert!(db.dirty_unlikes("local").await.unwrap().is_empty());
}

#[tokio::test]
async fn audio_cd_source_and_track_identity_round_trip_without_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let db = db::init(&directory.path().join("test.db")).await.unwrap();
    let mut server = config::MusicServer::new_with_service(
        "CD drive".into(),
        "/dev/sr0".into(),
        config::MusicService::AudioCd,
    );
    server.id = Some("cd-drive".into());
    let config = server::sources::Sources {
        active: Source::Server("cd-drive".into()),
        server: Some(server),
        ..Default::default()
    };
    let source = source::active(db.clone(), &config);
    assert_eq!(source.rip_audio(), server::audio_cd::supported());
    assert!(!source.capabilities().sync);
    assert!(!source.capabilities().downloads);
    assert!(!source.capabilities().delete_from_disk);
    let expected = track(TrackId::Server {
        service: config::MusicService::AudioCd,
        item_id: format!("{}-01", "a".repeat(64)),
    });
    source
        .upsert_tracks(std::slice::from_ref(&expected))
        .await
        .unwrap();
    let tracks = db
        .tracks_by_keys(&config.active, &[expected.id.key().into_owned()])
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, expected.id);
    assert_eq!(
        reader::TrackId::from_legacy_path(&tracks[0].id.uid()),
        expected.id
    );
    assert!(matches!(
        server::playback_ref::PlaybackItemRef::parse(&tracks[0].id.uid()),
        server::playback_ref::PlaybackItemRef::Server { item_id } if item_id == expected.id.key()
    ));
}
