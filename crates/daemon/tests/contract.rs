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
    ConfigService, FavoritesService, JobRunner, LibraryService, LocalApi, PlaybackServices,
    QueueMaterializer, SessionHandle,
};
use player::engine::{NullSink, SourceFactory};
use player::player::Player;
use reader::Track;

struct StubLibrary;

#[async_trait::async_trait]
impl QueueMaterializer for StubLibrary {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        match context {
            QueueContext::Tracks { keys } => Ok(keys.iter().map(|key| track(key)).collect()),
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
    let seeded: Vec<Track> = ["/lib/seed-0.flac", "/lib/seed-1.flac"]
        .iter()
        .map(|key| track(key))
        .collect();
    database
        .upsert_tracks(&config::Source::Local, &seeded)
        .await
        .expect("seed tracks");
    let config_service = Arc::new(ConfigService::new(
        database.clone(),
        dir.path().join("settings.toml"),
        config::AppConfig::default(),
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
    let favorites = FavoritesService::new(database, session.clone());
    let build_api = |session: SessionHandle| {
        LocalApi::new(session)
            .with_config(config_service.clone())
            .with_library(library.clone())
            .with_jobs(jobs.clone())
            .with_favorites(favorites.clone())
    };
    let token = "contract-token".to_string();
    let state = Arc::new(daemon::grpc::GrpcState {
        api: Arc::new(build_api(session.clone())),
        artwork: None,
        session: session.clone(),
        token: token.clone(),
        started: Instant::now(),
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

fn replace(keys: &[&str]) -> SetQueueRequest {
    SetQueueRequest {
        mode: QueueMode::Replace,
        context: QueueContext::Tracks {
            keys: keys.iter().map(|key| (*key).to_string()).collect(),
        },
        start_index: Some(0),
        shuffle: None,
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
        .expect("set queue over http");
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
        .expect_err("guarded over http");
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
async fn wrong_token_is_rejected() {
    let pair = spawn_pair().await;
    let probe = pair.wire.player_state().await;
    assert!(probe.is_ok(), "control: correct token works");
    let bad =
        client::GrpcApi::new(pair.wire.addr().to_string(), "wrong-token").expect("client builds");
    let err = bad.player_state().await.expect_err("rejected");
    assert_eq!(err.code, ErrorCode::Unauthorized);
}
