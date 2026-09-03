//! Contract tests: the same assertions run through `LocalApi` (in-process)
//! and `GrpcApi` (over a real tonic server and its Attach stream), proving
//! the two transports cannot drift. This is the parity mechanism the split
//! relies on.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, ErrorCode, Intent, KopuzApi, LoopMode, Page, Phase, PlayerCommand,
    PlayerState, QueueContext, QueueEdit, QueueMode, SetQueueRequest, TrackFilter,
};
use daemon::session::FactoryOverride;
use daemon::{
    ConfigService, DownloadsService, FavoritesService, FrontendService, JobRunner, LibraryService,
    LocalApi, PlaybackServices, QueueMaterializer, SessionHandle,
};
use player::engine::{NullSink, SourceFactory};
use player::player::Player;
use reader::Track;

struct StubLibrary;

#[async_trait::async_trait]
impl QueueMaterializer for StubLibrary {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        match context {
            // Keys containing "nope" stay unresolved, standing in for a track
            // that neither the DB, the transient cache, nor disk can produce;
            // the favorites contract test uses one to assert NotFound mapping.
            QueueContext::Tracks { keys } => Ok(keys
                .iter()
                .filter(|key| !key.contains("nope"))
                .map(|key| track(key))
                .collect()),
            QueueContext::InlineTracks { tracks } => Ok(tracks
                .iter()
                .map(|value| Track {
                    id: reader::TrackId::Server {
                        service: config::MusicService::YtMusic,
                        item_id: value.key.clone(),
                    },
                    cover: None,
                    album_id: value.album_id.clone(),
                    title: value.title.clone(),
                    artist: value.artist.clone(),
                    album: value.album.clone(),
                    duration: value.duration_ms.unwrap_or_default() / 1000,
                    khz: value.khz,
                    bitrate: value.bitrate,
                    track_number: value.track_number,
                    disc_number: value.disc_number,
                    musicbrainz_release_id: value.musicbrainz_release_id.clone(),
                    musicbrainz_recording_id: value.musicbrainz_recording_id.clone(),
                    musicbrainz_track_id: value.musicbrainz_track_id.clone(),
                    playlist_item_id: value.playlist_item_id.clone(),
                    artists: value.artists.clone(),
                })
                .collect()),
            _ => Err(ApiError::unsupported("stub resolves raw tracks only")),
        }
    }
}

fn track(key: &str) -> Track {
    Track {
        id: reader::models::TrackId::Local(std::path::PathBuf::from(key)),
        cover: None,
        album_id: String::new(),
        title: key.to_string(),
        artist: String::new(),
        album: String::new(),
        duration: 6,
        khz: 44,
        bitrate: 320,
        track_number: None,
        disc_number: None,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec![],
    }
}

fn wav_bytes(seconds: u64) -> Vec<u8> {
    let sample_rate: u32 = 44_100;
    let channels: usize = 2;
    let frames = seconds as usize * sample_rate as usize;
    let data_len = frames * channels * 2;
    let mut bytes = Vec::with_capacity(44 + data_len);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&(channels as u16).to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&((channels * 2) as u16).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
    bytes.resize(44 + data_len, 0);
    bytes
}

fn wav_factory(seconds: u64) -> SourceFactory {
    let bytes = wav_bytes(seconds);
    Box::new(move || Ok(player::decoder::from_stream(Cursor::new(bytes))))
}

struct Pair {
    local: LocalApi,
    wire: client::GrpcApi,
    _dir: tempfile::TempDir,
}

async fn spawn_pair() -> Pair {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("contract.db"))
        .await
        .expect("db init");
    let mut seeded: Vec<Track> = ["/lib/seed-0.flac", "/lib/seed-1.flac"]
        .iter()
        .map(|key| track(key))
        .collect();
    seeded[0].album_id = "album-1".into();
    seeded[0].album = "Alpha".into();
    seeded[0].artist = "Ada".into();
    seeded[0].artists = vec!["Ada".into()];
    seeded[0].track_number = Some(1);
    seeded[1].album_id = "album-2".into();
    seeded[1].album = "Beta".into();
    seeded[1].artist = "Bob".into();
    seeded[1].artists = vec!["Bob".into()];
    seeded[1].track_number = Some(1);
    database
        .upsert_tracks(&config::Source::Local, &seeded)
        .await
        .expect("seed tracks");
    database
        .upsert_albums(
            &config::Source::Local,
            &[
                reader::Album {
                    id: "album-1".into(),
                    title: "Alpha".into(),
                    artist: "Ada".into(),
                    genre: "Jazz".into(),
                    year: 2024,
                    cover_path: None,
                    manual_cover: false,
                },
                reader::Album {
                    id: "album-2".into(),
                    title: "Beta".into(),
                    artist: "Bob".into(),
                    genre: "Rock".into(),
                    year: 2023,
                    cover_path: None,
                    manual_cover: false,
                },
            ],
        )
        .await
        .expect("seed albums");
    database
        .upsert_playlist_meta(&config::Source::Local, "playlist-1", "Seeds", None, None)
        .await
        .expect("seed playlist");
    database
        .set_playlist_tracks(
            &config::Source::Local,
            "playlist-1",
            &["/lib/seed-1.flac".into(), "/lib/seed-0.flac".into()],
        )
        .await
        .expect("seed playlist tracks");
    database
        .push_recent(&config::Source::Local, "/lib/seed-0.flac")
        .await
        .expect("seed recent");
    database
        .bump_listen_count(&config::Source::Local, "/lib/seed-0.flac")
        .await
        .expect("seed listen count");
    let mut seeded_config = config::AppConfig::default();
    seeded_config.radio_registries.clear();
    let config_service = Arc::new(ConfigService::new(
        database.clone(),
        dir.path().join("settings.toml"),
        seeded_config,
    ));
    let library = Arc::new(LibraryService::new(
        database.clone(),
        config::Source::Local,
        Arc::new(radio::registry::StationRegistry::default()),
        dir.path().join("covers"),
    ));
    let player = Player::try_with_sink(Box::new(NullSink::new())).expect("headless player starts");
    let provider: FactoryOverride = Arc::new(|_| Some(wav_factory(6)));
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices::default(),
        provider,
    );
    library.attach_session(session.clone());
    let jobs = Arc::new(JobRunner::new(session.clone()));
    let favorites = FavoritesService::new(database.clone(), session.clone());
    let downloads = DownloadsService::new(
        database.clone(),
        session.clone(),
        config_service.clone(),
        dir.path().join("offline"),
    );
    let frontend = FrontendService::new(
        database.clone(),
        config_service.clone(),
        library.clone(),
        session.clone(),
        dir.path().join("uploaded-artwork"),
    );
    let artwork =
        daemon::ArtworkService::new(database, session.clone(), dir.path().join("artwork-cache"));
    let build_api = |session: SessionHandle| {
        LocalApi::new(session)
            .with_config(config_service.clone())
            .with_library(library.clone())
            .with_jobs(jobs.clone())
            .with_favorites(favorites.clone())
            .with_downloads(downloads.clone())
            .with_frontend(frontend.clone())
            .with_artwork(artwork.clone())
    };
    let token = "contract-token".to_string();
    let state = Arc::new(daemon::grpc::GrpcState {
        api: Arc::new(build_api(session.clone())),
        session: session.clone(),
        token: token.clone(),
        started: Instant::now(),
        shutdown: None,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(daemon::grpc::serve(listener, state));
    Pair {
        local: build_api(session),
        wire: client::GrpcApi::new(addr.to_string(), token).expect("wire client"),
        _dir: dir,
    }
}

async fn activate_spotify(pair: &Pair) -> String {
    let source = pair
        .local
        .upsert_server(api::ServerDraft {
            name: "Spotify".into(),
            url: "test-client-id".into(),
            service: api::MusicService::Spotify,
            ..Default::default()
        })
        .await
        .expect("create Spotify source");
    pair.local
        .provision_credentials(api::CredentialProvision {
            server_id: source.id.clone(),
            secret: "test-access-token".into(),
            user_id: Some("spotify-user".into()),
            browser: None,
        })
        .await
        .expect("provision Spotify source");
    pair.local
        .switch_source(source.id.clone())
        .await
        .expect("activate Spotify source");
    source.id
}

fn replace(keys: &[&str]) -> SetQueueRequest {
    SetQueueRequest {
        mode: QueueMode::Replace,
        context: QueueContext::Tracks {
            keys: keys.iter().map(|key| (*key).to_string()).collect(),
        },
        start_index: Some(0),
        shuffle: None,
        insert_index: None,
    }
}

async fn wait_state(
    api: &dyn KopuzApi,
    description: &str,
    predicate: impl Fn(&PlayerState) -> bool,
) -> PlayerState {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = api.player_state().await.expect("player state");
        if predicate(&state) {
            return state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {description}: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Wall-clock and anchor fields differ between the two reads by nature;
/// everything else must match bit for bit.
fn normalized(mut state: PlayerState) -> PlayerState {
    state.now_ms = 0;
    state.position = None;
    state
}

#[tokio::test]
async fn reads_agree_between_local_and_wire() {
    let pair = spawn_pair().await;
    let ack = pair
        .wire
        .set_queue(replace(&["/a.wav", "/b.wav", "/c.wav"]))
        .await
        .expect("set queue over gRPC");
    assert!(ack.rev > 0);
    wait_state(&pair.local, "committed", |state| {
        state.phase == Phase::Playing && matches!(state.intent, Intent::Committed { .. })
    })
    .await;

    let local_state = normalized(pair.local.player_state().await.expect("local state"));
    let wire_state = normalized(pair.wire.player_state().await.expect("wire state"));
    assert_eq!(local_state, wire_state);

    let local_window = pair
        .local
        .queue_window(Page::default())
        .await
        .expect("local window");
    let wire_window = pair
        .wire
        .queue_window(Page::default())
        .await
        .expect("wire window");
    assert_eq!(local_window, wire_window);
    assert_eq!(wire_window.total, 3);
    assert_eq!(
        pair.local.live_queue().await.expect("local live queue"),
        pair.wire.live_queue().await.expect("wire live queue")
    );
}

#[tokio::test]
async fn queue_persistence_agrees_between_local_and_wire() {
    let pair = spawn_pair().await;
    let tracks = pair
        .wire
        .tracks_by_keys(vec!["/lib/seed-0.flac".into(), "/lib/seed-1.flac".into()])
        .await
        .expect("seed tracks over the wire");
    let snapshot = api::QueuePersistenceSnapshot {
        tracks,
        current_index: 1,
        progress_ms: 4_000,
        shuffle_order: vec![1, 0],
        shuffle_enabled: true,
    };
    pair.wire
        .save_queue_snapshot(snapshot.clone())
        .await
        .expect("save snapshot over the wire");
    assert_eq!(
        snapshot,
        pair.local.queue_snapshot().await.expect("read locally")
    );

    let invalid = api::QueuePersistenceSnapshot {
        tracks: snapshot.tracks,
        shuffle_order: vec![0, 0],
        ..Default::default()
    };
    let local_error = pair
        .local
        .save_queue_snapshot(invalid.clone())
        .await
        .expect_err("invalid local snapshot");
    let wire_error = pair
        .wire
        .save_queue_snapshot(invalid)
        .await
        .expect_err("invalid wire snapshot");
    assert_eq!(local_error.code, wire_error.code);
    assert_eq!(local_error.message, wire_error.message);
}

#[tokio::test]
async fn equalizer_preview_agrees_between_local_and_wire() {
    let pair = spawn_pair().await;
    let equalizer = serde_json::json!({
        "enabled": true,
        "preset": "Custom",
        "bands": [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 4.0, 3.0, 2.0, 1.0],
        "preamp_db": -2.0,
    });
    pair.local
        .preview_equalizer(equalizer.clone())
        .await
        .expect("local equalizer preview");
    pair.wire
        .preview_equalizer(equalizer)
        .await
        .expect("wire equalizer preview");

    let invalid = serde_json::json!({ "bands": "not a band array" });
    let local_error = pair
        .local
        .preview_equalizer(invalid.clone())
        .await
        .expect_err("invalid local equalizer");
    let wire_error = pair
        .wire
        .preview_equalizer(invalid)
        .await
        .expect_err("invalid wire equalizer");
    assert_eq!(local_error.code, wire_error.code);
    assert_eq!(local_error.message, wire_error.message);
}

#[tokio::test]
async fn commands_and_errors_map_identically() {
    let pair = spawn_pair().await;
    pair.wire
        .set_queue(replace(&["/a.wav", "/b.wav"]))
        .await
        .expect("set queue");
    wait_state(&pair.local, "committed", |state| {
        matches!(state.intent, Intent::Committed { .. })
    })
    .await;

    pair.wire
        .player_command(PlayerCommand::SetMode {
            shuffle: None,
            loop_mode: Some(LoopMode::Queue),
        })
        .await
        .expect("set mode over the wire");
    let state = pair.local.player_state().await.expect("state");
    assert_eq!(state.queue.loop_mode, LoopMode::Queue);

    let local_err = pair
        .local
        .queue_edit(QueueEdit::Remove { index: 0 })
        .await
        .expect_err("guarded locally");
    let wire_err = pair
        .wire
        .queue_edit(QueueEdit::Remove { index: 0 })
        .await
        .expect_err("guarded over gRPC");
    assert_eq!(local_err.code, ErrorCode::InvalidInput);
    assert_eq!(wire_err.code, local_err.code);
    assert_eq!(wire_err.message, local_err.message);

    let local_page = pair
        .local
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("tracks locally");
    let wire_page = pair
        .wire
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("tracks over the wire");
    assert_eq!(local_page, wire_page);
    assert_eq!(wire_page.total, 2);
}

#[tokio::test]
async fn attach_stream_delivers_typed_events() {
    use futures_util::StreamExt;
    let pair = spawn_pair().await;
    let mut events = pair.wire.events();

    // The stream connects asynchronously and the first subscription has no
    // Last-Event-ID to replay from, so keep nudging until events flow.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut saw_queue_changed = false;
    let mut saw_player_state = false;
    while !(saw_queue_changed && saw_player_state) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for Attach events"
        );
        pair.wire
            .player_command(PlayerCommand::SetMode {
                shuffle: Some(true),
                loop_mode: None,
            })
            .await
            .expect("set mode");
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_millis(500), events.next()).await
        {
            match event {
                ApiEvent::QueueChanged { .. } => saw_queue_changed = true,
                ApiEvent::PlayerState(state) if state.queue.shuffle => saw_player_state = true,
                _ => {}
            }
            if saw_queue_changed && saw_player_state {
                break;
            }
        }
    }
}

#[tokio::test]
async fn config_view_and_patch_agree_across_transports() {
    let pair = spawn_pair().await;

    let local_view = pair.local.config().await.expect("local view");
    let wire_view = pair.wire.config().await.expect("wire view");
    assert_eq!(local_view, wire_view);
    assert!(local_view.config.get("lastfm_session_key").is_none());
    assert!(local_view.config.get("server").is_none());

    let patched = pair
        .wire
        .patch_config(serde_json::json!({"crossfade_seconds": 7}))
        .await
        .expect("patch over the wire");
    assert_eq!(patched.config["crossfade_seconds"], 7);
    let local_view = pair.local.config().await.expect("local view after patch");
    assert_eq!(local_view.config["crossfade_seconds"], 7);

    let local_err = pair
        .local
        .patch_config(serde_json::json!({"servers": []}))
        .await
        .expect_err("credential key locally");
    let wire_err = pair
        .wire
        .patch_config(serde_json::json!({"servers": []}))
        .await
        .expect_err("credential key over the wire");
    assert_eq!(local_err.code, ErrorCode::InvalidInput);
    assert_eq!(wire_err.code, local_err.code);
    assert_eq!(wire_err.message, local_err.message);
}

#[tokio::test]
async fn favorites_round_trip_across_transports() {
    let pair = spawn_pair().await;
    pair.wire
        .set_favorite("/lib/seed-0.flac".into(), true)
        .await
        .expect("set over the wire");
    let local_view = pair.local.favorites().await.expect("local list");
    let wire_view = pair.wire.favorites().await.expect("wire list");
    assert_eq!(local_view.refs, wire_view.refs);
    assert!(local_view.refs.contains(&"/lib/seed-0.flac".to_string()));

    pair.local
        .set_favorite("/lib/seed-0.flac".into(), false)
        .await
        .expect("unset locally");
    let wire_view = pair.wire.favorites().await.expect("wire list");
    assert!(wire_view.refs.is_empty());

    let err = pair
        .wire
        .set_favorite("/nope.flac".into(), true)
        .await
        .expect_err("unknown key");
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[tokio::test]
async fn scan_job_indexes_local_files_over_the_wire() {
    let pair = spawn_pair().await;
    let music = pair._dir.path().join("music");
    std::fs::create_dir_all(&music).expect("music dir");
    std::fs::write(music.join("one.wav"), wav_bytes(1)).expect("write wav");
    std::fs::write(music.join("two.wav"), wav_bytes(1)).expect("write wav");

    pair.wire
        .patch_config(serde_json::json!({
            "music_directory": [music.to_string_lossy()],
        }))
        .await
        .expect("point the library at the temp dir");

    let job = pair
        .wire
        .start_job(api::JobKind::Scan)
        .await
        .expect("start scan");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let jobs = pair.local.jobs().await.expect("jobs");
        let status = jobs
            .iter()
            .find(|status| status.id == job.job_id)
            .expect("job listed");
        match status.state {
            api::JobState::Running => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "scan timed out: {status:?}"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            api::JobState::Finished => break,
            other => panic!("scan ended as {other:?}: {status:?}"),
        }
    }

    let page = pair
        .wire
        .tracks(TrackFilter::default(), Page::default())
        .await
        .expect("tracks over the wire");
    assert!(
        page.items
            .iter()
            .filter(|track| track.title.contains("one") || track.title.contains("two"))
            .count()
            >= 2,
        "scanned tracks visible: {:?}",
        page.items
            .iter()
            .map(|t| t.title.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn folders_and_stats_agree_across_transports() {
    let pair = spawn_pair().await;
    let local_page = pair
        .local
        .folder_tracks("/lib/".into(), Page::default())
        .await
        .expect("folders locally");
    let wire_page = pair
        .wire
        .folder_tracks("/lib/".into(), Page::default())
        .await
        .expect("folders over the wire");
    assert_eq!(local_page, wire_page);
    assert_eq!(wire_page.total, 2);
    let row = &wire_page.items[0];
    assert_eq!(row.key, "/lib/seed-0.flac");
    assert_eq!(row.artwork.as_deref(), Some(row.key.as_str()));
    assert!(!row.offline);

    let local_stats = pair.local.stats().await.expect("stats locally");
    let wire_stats = pair.wire.stats().await.expect("stats over the wire");
    assert_eq!(local_stats, wire_stats);
}

#[tokio::test]
async fn library_catalog_agrees_across_transports() {
    let pair = spawn_pair().await;
    let page = Page {
        offset: 0,
        limit: 1,
    };

    assert_eq!(
        pair.local
            .albums(api::AlbumFilter::default(), page)
            .await
            .expect("local albums"),
        pair.wire
            .albums(api::AlbumFilter::default(), page)
            .await
            .expect("wire albums")
    );
    assert_eq!(
        pair.local
            .album("album-1".into())
            .await
            .expect("local album"),
        pair.wire.album("album-1".into()).await.expect("wire album")
    );
    assert_eq!(
        pair.local.artists(page).await.expect("local artists"),
        pair.wire.artists(page).await.expect("wire artists")
    );
    assert_eq!(
        pair.local.genres().await.expect("local genres"),
        pair.wire.genres().await.expect("wire genres")
    );
    assert_eq!(
        pair.local.recent_tracks(page).await.expect("local recents"),
        pair.wire.recent_tracks(page).await.expect("wire recents")
    );
    assert_eq!(
        pair.local
            .album_tracks("album-1".into(), Page::default())
            .await
            .expect("local album tracks"),
        pair.wire
            .album_tracks("album-1".into(), Page::default())
            .await
            .expect("wire album tracks")
    );
    assert_eq!(
        pair.local
            .artist_tracks("Ada".into(), Page::default())
            .await
            .expect("local artist tracks"),
        pair.wire
            .artist_tracks("Ada".into(), Page::default())
            .await
            .expect("wire artist tracks")
    );
    assert_eq!(
        pair.local
            .genre_tracks("Jazz".into(), Page::default())
            .await
            .expect("local genre tracks"),
        pair.wire
            .genre_tracks("Jazz".into(), Page::default())
            .await
            .expect("wire genre tracks")
    );
    assert_eq!(
        pair.local
            .artist_sample_tracks(Page::default())
            .await
            .expect("local artist samples"),
        pair.wire
            .artist_sample_tracks(Page::default())
            .await
            .expect("wire artist samples")
    );
    let keys = vec!["/lib/seed-1.flac".into(), "/lib/seed-0.flac".into()];
    let local_tracks = pair
        .local
        .tracks_by_keys(keys.clone())
        .await
        .expect("local keyed tracks");
    let wire_tracks = pair
        .wire
        .tracks_by_keys(keys)
        .await
        .expect("wire keyed tracks");
    assert_eq!(local_tracks, wire_tracks);
    assert_eq!(wire_tracks[0].key, "/lib/seed-1.flac");
    assert_eq!(
        pair.local
            .track_web_url("/lib/seed-0.flac".into())
            .await
            .expect("local web URL"),
        pair.wire
            .track_web_url("/lib/seed-0.flac".into())
            .await
            .expect("wire web URL")
    );
    assert_eq!(
        pair.local
            .refresh_artist_artwork(vec!["Ada".into()])
            .await
            .expect("local artist artwork refresh"),
        pair.wire
            .refresh_artist_artwork(vec!["Ada".into()])
            .await
            .expect("wire artist artwork refresh")
    );
    assert_eq!(
        pair.local.top_genre().await.expect("local top genre"),
        pair.wire.top_genre().await.expect("wire top genre")
    );
    assert_eq!(
        pair.local.search("Ada".into()).await.expect("local search"),
        pair.wire.search("Ada".into()).await.expect("wire search")
    );
}

#[tokio::test]
async fn playlist_and_folder_mutations_cross_transports() {
    let pair = spawn_pair().await;
    assert_eq!(
        pair.local.playlists().await.expect("local catalog"),
        pair.wire.playlists().await.expect("wire catalog")
    );

    let playlist_id = pair
        .wire
        .create_playlist("Cross transport".into(), vec!["/lib/seed-0.flac".into()])
        .await
        .expect("create playlist");
    pair.local
        .rename_playlist(playlist_id.clone(), "Renamed".into())
        .await
        .expect("rename playlist");
    pair.wire
        .add_playlist_tracks(playlist_id.clone(), vec!["/lib/seed-1.flac".into()])
        .await
        .expect("add track");
    pair.local
        .reorder_playlist_tracks(
            playlist_id.clone(),
            vec!["/lib/seed-1.flac".into(), "/lib/seed-0.flac".into()],
        )
        .await
        .expect("reorder tracks");

    let request = api::PlaylistTracksRequest {
        id: playlist_id.clone(),
        page: Page::default(),
    };
    let local_tracks = pair
        .local
        .playlist_tracks(request.clone())
        .await
        .expect("local playlist tracks");
    let wire_tracks = pair
        .wire
        .playlist_tracks(request)
        .await
        .expect("wire playlist tracks");
    assert_eq!(local_tracks, wire_tracks);
    assert_eq!(wire_tracks.items[0].key, "/lib/seed-1.flac");
    let refresh = api::PlaylistTracksRequest {
        id: playlist_id.clone(),
        page: Page::default(),
    };
    assert_eq!(
        pair.local
            .refresh_playlist(refresh.clone())
            .await
            .expect("refresh local playlist"),
        pair.wire
            .refresh_playlist(refresh)
            .await
            .expect("refresh wire playlist")
    );

    let folder_id = pair
        .wire
        .create_playlist_folder("Folder".into())
        .await
        .expect("create folder");
    pair.local
        .move_playlist(playlist_id.clone(), Some(folder_id.clone()))
        .await
        .expect("move playlist");
    pair.wire
        .rename_playlist_folder(folder_id.clone(), "Renamed folder".into())
        .await
        .expect("rename folder");
    assert_eq!(
        pair.local.playlists().await.expect("local changed catalog"),
        pair.wire.playlists().await.expect("wire changed catalog")
    );

    let local_error = pair
        .local
        .reorder_playlist_tracks(
            playlist_id.clone(),
            vec!["/lib/seed-0.flac".into(), "/lib/seed-0.flac".into()],
        )
        .await
        .expect_err("invalid local reorder");
    let wire_error = pair
        .wire
        .reorder_playlist_tracks(
            playlist_id.clone(),
            vec!["/lib/seed-0.flac".into(), "/lib/seed-0.flac".into()],
        )
        .await
        .expect_err("invalid wire reorder");
    assert_eq!(local_error, wire_error);

    pair.wire
        .remove_playlist_tracks(playlist_id.clone(), vec!["/lib/seed-1.flac".into()])
        .await
        .expect("remove track");
    pair.local
        .delete_playlist_folder(folder_id)
        .await
        .expect("delete folder");
    pair.wire
        .delete_playlist(playlist_id)
        .await
        .expect("delete playlist");
    assert_eq!(
        pair.local.playlists().await.expect("local final catalog"),
        pair.wire.playlists().await.expect("wire final catalog")
    );
}

#[tokio::test]
async fn server_credentials_are_secure_and_do_not_switch_sources() {
    let pair = spawn_pair().await;
    let created = pair
        .wire
        .upsert_server(api::ServerDraft {
            name: "Test server".into(),
            url: "https://music.example".into(),
            service: api::MusicService::Jellyfin,
            ..Default::default()
        })
        .await
        .expect("create server");
    assert!(!created.active);

    let provisioned = pair
        .wire
        .provision_credentials(api::CredentialProvision {
            server_id: created.id.clone(),
            secret: "top-secret-token".into(),
            user_id: Some("user-1".into()),
            browser: None,
        })
        .await
        .expect("provision credentials");
    assert!(provisioned.authenticated);
    assert!(!provisioned.active);
    assert!(
        pair.local
            .sources()
            .await
            .expect("local sources")
            .iter()
            .any(|source| source.id == created.id && source.authenticated && !source.active)
    );

    let view = pair.wire.config().await.expect("wire config");
    let serialized = serde_json::to_string(&view).expect("serialize config view");
    assert!(!serialized.contains("top-secret-token"));
    assert!(view.config.get("server").is_none());
    assert!(view.config.get("servers").is_none());

    pair.local
        .clear_credentials(created.id.clone())
        .await
        .expect("clear credentials");
    let local_sources = pair
        .local
        .sources()
        .await
        .expect("local sources after clear");
    let wire_sources = pair.wire.sources().await.expect("wire sources after clear");
    assert_eq!(local_sources, wire_sources);
    assert!(
        wire_sources
            .iter()
            .any(|source| source.id == created.id && !source.authenticated && !source.active)
    );
    assert_eq!(
        pair.local
            .validate_source("local".into())
            .await
            .expect("validate local source"),
        pair.wire
            .validate_source("local".into())
            .await
            .expect("validate local source over wire")
    );
    pair.wire
        .delete_server(created.id)
        .await
        .expect("delete server");
}

#[tokio::test]
async fn source_management_agrees_between_local_and_wire() {
    let pair = spawn_pair().await;
    let local = pair
        .wire
        .set_source_directories("local".into(), vec!["/music".into()])
        .await
        .expect("set default local directories");
    assert_eq!(local.directories, vec!["/music"]);

    let created = pair
        .local
        .upsert_local_source(api::LocalSourceDraft {
            id: None,
            name: "Archive".into(),
            directories: vec!["/archive".into()],
        })
        .await
        .expect("create local source");
    assert_eq!(created.kind, api::SourceKind::LocalLibrary);
    assert_eq!(created.directories, vec!["/archive"]);
    assert_eq!(
        pair.local.sources().await.expect("local sources"),
        pair.wire.sources().await.expect("wire sources")
    );

    let updated = pair
        .wire
        .set_source_directories(created.id.clone(), vec!["/archive".into(), "/vault".into()])
        .await
        .expect("update local source directories");
    assert_eq!(updated.directories, vec!["/archive", "/vault"]);
    assert!(
        pair.local
            .switch_source(created.id.clone())
            .await
            .expect("switch local source")
            .active
    );
    pair.wire
        .delete_local_source(created.id)
        .await
        .expect("delete local source");
    assert!(
        pair.local
            .sources()
            .await
            .expect("sources after delete")
            .iter()
            .any(|source| source.id == "local" && source.active)
    );

    let local_error = pair
        .local
        .delete_local_source("local".into())
        .await
        .expect_err("local default delete must fail");
    let wire_error = pair
        .wire
        .delete_local_source("local".into())
        .await
        .expect_err("wire default delete must fail");
    assert_eq!(local_error, wire_error);
}

#[tokio::test]
async fn switching_sources_stops_playback_and_clears_the_queue() {
    let pair = spawn_pair().await;
    let server_id = activate_spotify(&pair).await;
    pair.local
        .switch_source("local".into())
        .await
        .expect("return to local before test");

    pair.local
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks {
                keys: vec!["first".into(), "second".into()],
            },
            start_index: Some(1),
            shuffle: Some(false),
            insert_index: None,
        })
        .await
        .expect("seed local queue");
    pair.local
        .switch_source(server_id.clone())
        .await
        .expect("switch local transport to server");
    let local_state = pair.local.player_state().await.expect("local state");
    assert_eq!(local_state.intent, Intent::Stopped);
    assert_eq!(local_state.queue.length, 0);

    pair.wire
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks {
                keys: vec!["third".into()],
            },
            start_index: Some(0),
            shuffle: Some(false),
            insert_index: None,
        })
        .await
        .expect("seed wire queue");
    let _lease = pair
        .wire
        .claim_external_playback(api::ExternalPlayback {
            kind: "spotify".into(),
            device: Some("test-device".into()),
        })
        .await
        .expect("mark wire playback external");
    pair.wire
        .switch_source("local".into())
        .await
        .expect("switch wire transport to local");
    let wire_state = pair.wire.player_state().await.expect("wire state");
    assert_eq!(wire_state.intent, Intent::Stopped);
    assert_eq!(wire_state.queue.length, 0);
    assert_eq!(wire_state.external, None);

    pair.local
        .switch_source(server_id)
        .await
        .expect("reactivate Spotify before config patch");
    pair.wire
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks {
                keys: vec!["fourth".into()],
            },
            start_index: Some(0),
            shuffle: Some(false),
            insert_index: None,
        })
        .await
        .expect("seed queue before config patch");
    let _lease = pair
        .wire
        .claim_external_playback(api::ExternalPlayback {
            kind: "spotify".into(),
            device: Some("test-device".into()),
        })
        .await
        .expect("claim external before config patch");
    pair.wire
        .patch_config(serde_json::json!({
            "active_source": "Local"
        }))
        .await
        .expect("switch source through config patch");
    let patched_state = pair.wire.player_state().await.expect("patched state");
    assert_eq!(patched_state.intent, Intent::Stopped);
    assert_eq!(patched_state.queue.length, 0);
    assert_eq!(patched_state.external, None);
}

#[tokio::test]
async fn radio_and_external_playback_agree_across_transports() {
    let pair = spawn_pair().await;
    let station = api::RadioStationInfo {
        id: "contract-radio".into(),
        name: "Contract Radio".into(),
        description: "Contract fixture".into(),
        icon: "fa-solid fa-radio".into(),
        artwork: None,
        tags: vec!["test".into()],
        streams: vec![api::RadioStreamInfo {
            id: "main".into(),
            name: "Main".into(),
            url: "https://radio.example/stream".into(),
            icon: None,
        }],
        pinned: false,
    };
    pair.wire
        .pin_radio_station(station.clone(), true)
        .await
        .expect("pin station");
    let local_stations = pair.local.radio_stations().await.expect("local stations");
    let wire_stations = pair.wire.radio_stations().await.expect("wire stations");
    assert_eq!(local_stations, wire_stations);
    assert_eq!(wire_stations.len(), 1);
    assert!(wire_stations[0].pinned);

    let local_error = pair
        .local
        .track_radio("/lib/seed-0.flac".into())
        .await
        .expect_err("local source has no radio");
    let wire_error = pair
        .wire
        .track_radio("/lib/seed-0.flac".into())
        .await
        .expect_err("wire source has no radio");
    assert_eq!(local_error, wire_error);
    let local_error = pair
        .local
        .playlist_radio("playlist-1".into())
        .await
        .expect_err("local source has no playlist radio");
    let wire_error = pair
        .wire
        .playlist_radio("playlist-1".into())
        .await
        .expect_err("wire source has no playlist radio");
    assert_eq!(local_error, wire_error);
    let request = api::CatalogDetailRequest {
        kind: api::CatalogItemKind::Album,
        id: "album-1".into(),
        continuation: None,
    };
    let local_error = pair
        .local
        .catalog_detail(request.clone())
        .await
        .expect_err("local source has no remote catalog");
    let wire_error = pair
        .wire
        .catalog_detail(request)
        .await
        .expect_err("wire source has no remote catalog");
    assert_eq!(local_error, wire_error);
    let local_error = pair
        .local
        .external_access("spotify".into())
        .await
        .expect_err("local source has no Spotify access");
    let wire_error = pair
        .wire
        .external_access("spotify".into())
        .await
        .expect_err("wire source has no Spotify access");
    assert_eq!(local_error, wire_error);

    activate_spotify(&pair).await;

    let lease = pair
        .wire
        .claim_external_playback(api::ExternalPlayback {
            kind: "spotify".into(),
            device: Some("browser".into()),
        })
        .await
        .expect("set external playback");
    let local_state = wait_state(&pair.local, "external playback", |state| {
        state.external.is_some()
    })
    .await;
    let wire_state = pair.wire.player_state().await.expect("wire external state");
    assert_eq!(normalized(local_state), normalized(wire_state));
    pair.local
        .release_external_playback(lease.lease_id)
        .await
        .expect("clear external playback");
    wait_state(&pair.wire, "external playback cleared", |state| {
        state.external.is_none()
    })
    .await;
}

#[tokio::test]
async fn extended_frontend_operations_match_local_and_grpc() {
    use futures_util::StreamExt as _;

    let pair = spawn_pair().await;
    let remote = api::TrackInfo {
        key: "remote-1".into(),
        uid: "ytmusic:remote-1".into(),
        title: "Remote One".into(),
        artist: "Remote Artist".into(),
        album: "Remote Album".into(),
        duration_ms: Some(180_000),
        service: Some(api::MusicService::YtMusic),
        source: "remote".into(),
        ..Default::default()
    };
    pair.wire
        .set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::InlineTracks {
                tracks: vec![remote.clone()],
            },
            start_index: Some(0),
            shuffle: Some(false),
            insert_index: None,
        })
        .await
        .expect("queue inline remote track");
    pair.local
        .set_queue(SetQueueRequest {
            mode: QueueMode::Insert,
            context: QueueContext::InlineTracks {
                tracks: vec![api::TrackInfo {
                    key: "remote-2".into(),
                    uid: "ytmusic:remote-2".into(),
                    title: "Remote Two".into(),
                    duration_ms: Some(200_000),
                    service: Some(api::MusicService::YtMusic),
                    ..Default::default()
                }],
            },
            start_index: None,
            shuffle: None,
            insert_index: Some(1),
        })
        .await
        .expect("insert inline remote track");
    assert_eq!(
        pair.local
            .queue_window(Page::default())
            .await
            .expect("local queue"),
        pair.wire
            .queue_window(Page::default())
            .await
            .expect("wire queue")
    );

    let initial_statuses = pair
        .local
        .download_statuses()
        .await
        .expect("local statuses");
    assert_eq!(
        initial_statuses,
        pair.wire.download_statuses().await.expect("wire statuses")
    );
    let local_error = pair
        .local
        .cancel_download_item("missing".into())
        .await
        .expect_err("local missing download");
    let wire_error = pair
        .wire
        .cancel_download_item("missing".into())
        .await
        .expect_err("wire missing download");
    assert_eq!(local_error, wire_error);

    let provisioned = pair
        .local
        .provision_integration_credentials(api::IntegrationCredentialProvision {
            kind: api::IntegrationKind::ListenBrainz,
            token: Some("listen-token".into()),
            ..Default::default()
        })
        .await
        .expect("provision ListenBrainz");
    assert!(provisioned.configured);
    assert_eq!(
        pair.local
            .integration_credentials()
            .await
            .expect("local integration statuses"),
        pair.wire
            .integration_credentials()
            .await
            .expect("wire integration statuses")
    );
    pair.wire
        .clear_integration_credentials(api::IntegrationKind::ListenBrainz)
        .await
        .expect("clear ListenBrainz");
    assert!(
        pair.local
            .integration_credentials()
            .await
            .expect("cleared statuses")
            .iter()
            .find(|status| status.kind == api::IntegrationKind::ListenBrainz)
            .is_some_and(|status| !status.configured)
    );

    let local_error = pair
        .local
        .browse_source("local".into(), "/".into())
        .await
        .expect_err("local browse rejects non-server");
    let wire_error = pair
        .wire
        .browse_source("local".into(), "/".into())
        .await
        .expect_err("wire browse rejects non-server");
    assert_eq!(local_error, wire_error);
    let local_error = pair
        .local
        .authenticate_source("local".into())
        .await
        .expect_err("local source cannot authenticate");
    let wire_error = pair
        .wire
        .authenticate_source("local".into())
        .await
        .expect_err("wire source cannot authenticate");
    assert_eq!(local_error, wire_error);

    let request = api::YtdlpRequest::default();
    let local_error = pair
        .local
        .start_ytdlp(request.clone())
        .await
        .expect_err("empty local yt-dlp request");
    let wire_error = pair
        .wire
        .start_ytdlp(request)
        .await
        .expect_err("empty wire yt-dlp request");
    assert_eq!(local_error, wire_error);

    let mut local_lyrics = pair.local.lyrics_stream("missing".into());
    let mut wire_lyrics = pair.wire.lyrics_stream("missing".into());
    let local_error = local_lyrics
        .next()
        .await
        .expect("local lyrics response")
        .expect_err("local lyrics error");
    let wire_error = wire_lyrics
        .next()
        .await
        .expect("wire lyrics response")
        .expect_err("wire lyrics error");
    assert_eq!(local_error, wire_error);
}

#[tokio::test]
async fn external_playback_leases_report_identical_state() {
    let pair = spawn_pair().await;
    activate_spotify(&pair).await;
    let external = api::ExternalPlayback {
        kind: "spotify".into(),
        device: Some("browser".into()),
    };
    let local_lease = pair
        .local
        .claim_external_playback(external.clone())
        .await
        .expect("local external claim");
    let local_error = pair
        .local
        .report_external_playback(api::ExternalPlaybackReport {
            lease_id: "wrong".into(),
            ..Default::default()
        })
        .await
        .expect_err("local rejects wrong lease");
    let wire_error = pair
        .wire
        .report_external_playback(api::ExternalPlaybackReport {
            lease_id: "wrong".into(),
            ..Default::default()
        })
        .await
        .expect_err("wire rejects wrong lease");
    assert_eq!(local_error, wire_error);
    pair.local
        .release_external_playback(local_lease.lease_id)
        .await
        .expect("release local claim");

    let lease = pair
        .wire
        .claim_external_playback(external)
        .await
        .expect("wire external claim");
    pair.wire
        .report_external_playback(api::ExternalPlaybackReport {
            lease_id: lease.lease_id.clone(),
            track: Some(api::TrackInfo {
                key: "spotify-track".into(),
                uid: "spotify:spotify-track".into(),
                title: "Spotify Track".into(),
                artist: "Artist".into(),
                album: "Album".into(),
                duration_ms: Some(210_000),
                service: Some(api::MusicService::Spotify),
                ..Default::default()
            }),
            position_ms: 42_000,
            playing: true,
            completed: false,
            device: Some("browser".into()),
        })
        .await
        .expect("wire external report");
    let local_state = pair
        .local
        .player_state()
        .await
        .expect("local external state");
    let wire_state = pair.wire.player_state().await.expect("wire external state");
    assert_eq!(normalized(local_state.clone()), normalized(wire_state));
    assert_eq!(local_state.phase, Phase::Playing);
    assert_eq!(
        local_state.track.as_ref().map(|track| track.title.as_str()),
        Some("Spotify Track")
    );
    pair.wire
        .release_external_playback(lease.lease_id)
        .await
        .expect("release wire claim");
}

#[tokio::test]
async fn artwork_metadata_and_deletes_cross_transports() {
    let pair = spawn_pair().await;
    let image = image::DynamicImage::new_rgb8(2, 2);
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("encode png");
    pair.wire
        .upload_artwork(api::ArtworkUpload {
            target: Some(api::ArtworkTarget::Album {
                id: "album-1".into(),
            }),
            content_type: "image/png".into(),
            data: png.into_inner(),
        })
        .await
        .expect("upload artwork");
    let request = api::ArtworkRequest {
        entity: Some(api::ArtworkEntity::Album {
            id: "album-1".into(),
        }),
        hq: false,
    };
    let local_art = pair
        .local
        .artwork(request.clone())
        .await
        .expect("local artwork");
    let wire_art = pair.wire.artwork(request).await.expect("wire artwork");
    assert_eq!(local_art, wire_art);
    assert!(!wire_art.data.is_empty());
    assert!(
        pair.wire
            .album("album-1".into())
            .await
            .expect("album after artwork")
            .manual_artwork
    );
    pair.local
        .remove_artwork(api::ArtworkTarget::Album {
            id: "album-1".into(),
        })
        .await
        .expect("remove artwork");

    let local_error = pair
        .local
        .update_track_metadata(api::TrackMetadataPatch {
            key: "/missing.flac".into(),
            ..Default::default()
        })
        .await
        .expect_err("missing metadata target locally");
    let wire_error = pair
        .wire
        .update_track_metadata(api::TrackMetadataPatch {
            key: "/missing.flac".into(),
            ..Default::default()
        })
        .await
        .expect_err("missing metadata target over wire");
    assert_eq!(local_error, wire_error);

    pair.wire
        .delete_tracks(vec!["/lib/seed-1.flac".into()], false)
        .await
        .expect("delete track row");
    assert_eq!(
        pair.local
            .tracks(TrackFilter::default(), Page::default())
            .await
            .expect("local tracks after delete"),
        pair.wire
            .tracks(TrackFilter::default(), Page::default())
            .await
            .expect("wire tracks after delete")
    );
    pair.local
        .delete_album("album-1".into(), false)
        .await
        .expect("delete album");
    let local_error = pair
        .local
        .album("album-1".into())
        .await
        .expect_err("album deleted locally");
    let wire_error = pair
        .wire
        .album("album-1".into())
        .await
        .expect_err("album deleted over wire");
    assert_eq!(local_error, wire_error);
}

#[tokio::test]
async fn wrong_token_is_rejected() {
    let pair = spawn_pair().await;
    let probe = pair.wire.player_state().await;
    assert!(probe.is_ok(), "control: correct token works");
    let bad =
        client::GrpcApi::new(pair.wire.addr().to_string(), "wrong-token").expect("client builds");
    let err = bad.player_state().await.expect_err("rejected");
    assert_eq!(err.code, ErrorCode::Unauthorized);
}
