//! Session behavior tests: engine races, queue edits, persistence,
//! recording, and the replay ring, all over a fake sink.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use api::{ErrorCode, KopuzApi, LoopMode};
use futures_util::StreamExt;
use player::engine::{AudioSink, DataCallback, DataCallbackFactory, SinkConfig};

use super::state::now_playing_from;
use super::*;

const TEST_CONFIG: SinkConfig = SinkConfig {
    channels: 2,
    sample_rate: 44_100,
};

#[derive(Default)]
struct FakeSinkState {
    callback: Option<DataCallback>,
    config: Option<SinkConfig>,
    playing: bool,
    pause_calls: usize,
}

#[derive(Clone, Default)]
struct FakeSinkHandle(Arc<Mutex<FakeSinkState>>);

impl FakeSinkHandle {
    fn pull(&self, samples: usize) -> Vec<f32> {
        let mut output = vec![0.0; samples];
        let mut state = self.0.lock().expect("sink lock");
        if state.playing
            && let Some(callback) = state.callback.as_mut()
        {
            callback(&mut output);
        }
        output
    }

    fn pause_calls(&self) -> usize {
        self.0.lock().expect("sink lock").pause_calls
    }
}

struct FakeSink(FakeSinkHandle);

impl AudioSink for FakeSink {
    fn probe_config(&mut self, desired_sample_rate: Option<u32>) -> Result<SinkConfig, String> {
        Ok(SinkConfig {
            channels: TEST_CONFIG.channels,
            sample_rate: desired_sample_rate.unwrap_or(TEST_CONFIG.sample_rate),
        })
    }

    fn open(
        &mut self,
        _desired_sample_rate: Option<u32>,
        make_callback: DataCallbackFactory,
    ) -> Result<SinkConfig, String> {
        let callback = make_callback(TEST_CONFIG);
        let mut state = self.0.0.lock().expect("sink lock");
        state.callback = Some(callback);
        state.config = Some(TEST_CONFIG);
        state.playing = true;
        Ok(TEST_CONFIG)
    }

    fn config(&self) -> Option<SinkConfig> {
        self.0.0.lock().expect("sink lock").config
    }

    fn play(&mut self) -> Result<(), String> {
        self.0.0.lock().expect("sink lock").playing = true;
        Ok(())
    }

    fn pause(&mut self) {
        let mut state = self.0.0.lock().expect("sink lock");
        state.playing = false;
        state.pause_calls += 1;
    }

    fn close(&mut self) {
        let mut state = self.0.0.lock().expect("sink lock");
        state.callback = None;
        state.config = None;
        state.playing = false;
    }
}

struct StubLibrary;

#[async_trait::async_trait]
impl QueueMaterializer for StubLibrary {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        match context {
            QueueContext::Tracks { keys } => Ok(keys.iter().map(test_track).collect()),
            _ => Err(ApiError::unsupported("stub resolves raw tracks only")),
        }
    }
}

fn test_track(key: &String) -> Track {
    let duration = if key.starts_with("radio:") {
        u64::MAX
    } else if key.contains("short") {
        1
    } else {
        6
    };
    Track {
        id: reader::models::TrackId::Local(PathBuf::from(key)),
        cover: None,
        album_id: String::new(),
        title: key.clone(),
        artist: String::new(),
        album: String::new(),
        duration,
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
    let frames = seconds as usize * TEST_CONFIG.sample_rate as usize;
    let data_len = frames * TEST_CONFIG.channels * 2;
    let mut bytes = Vec::with_capacity(44 + data_len);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&(TEST_CONFIG.channels as u16).to_le_bytes());
    bytes.extend_from_slice(&TEST_CONFIG.sample_rate.to_le_bytes());
    bytes.extend_from_slice(
        &(TEST_CONFIG.sample_rate * TEST_CONFIG.channels as u32 * 2).to_le_bytes(),
    );
    bytes.extend_from_slice(&((TEST_CONFIG.channels * 2) as u16).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
    for frame in 0..frames {
        let sample = (((frame % 100) as i16) + 1) * 100;
        for _ in 0..TEST_CONFIG.channels {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    bytes
}

fn wav_factory(seconds: u64) -> SourceFactory {
    let bytes = wav_bytes(seconds);
    Box::new(move || Ok(player::decoder::from_stream(Cursor::new(bytes))))
}

fn gated_factory(seconds: u64, gate: Arc<(Mutex<bool>, Condvar)>) -> SourceFactory {
    let bytes = wav_bytes(seconds);
    Box::new(move || {
        let (lock, ready) = &*gate;
        let mut blocked = lock.lock().expect("gate lock");
        while *blocked {
            blocked = ready.wait(blocked).expect("gate wait");
        }
        drop(blocked);
        Ok(player::decoder::from_stream(Cursor::new(bytes)))
    })
}

struct Harness {
    api: LocalApi,
    sink: FakeSinkHandle,
}

fn harness(configure: impl FnOnce(&mut config::AppConfig)) -> Harness {
    harness_with_provider(
        configure,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    )
}

fn harness_with_provider(
    configure: impl FnOnce(&mut config::AppConfig),
    provider: FactoryOverride,
) -> Harness {
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut services = PlaybackServices::default();
    services.config.crossfade_seconds = 0;
    configure(&mut services.config);
    let session =
        SessionHandle::spawn_with_factory(Arc::new(StubLibrary), player, services, provider);
    Harness {
        api: LocalApi::new(session),
        sink,
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
    api: &LocalApi,
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

async fn drive_until(
    harness: &Harness,
    description: &str,
    predicate: impl Fn(&PlayerState) -> bool,
) -> PlayerState {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        // Keep the fake callback close enough to wall-clock pacing that
        // the actor can observe crossfade arming before the synthetic
        // decoder reaches EOF under parallel test load.
        harness.sink.pull(2048);
        let state = harness.api.player_state().await.expect("player state");
        if predicate(&state) {
            return state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out driving audio until {description}: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_committed(api: &LocalApi) -> PlayerState {
    wait_state(api, "committed playback", |state| {
        state.phase == ApiPhase::Playing && matches!(state.intent, Intent::Committed { .. })
    })
    .await
}

#[tokio::test]
async fn set_queue_then_window_round_trips() {
    let harness = harness(|_| {});
    let ack = harness
        .api
        .set_queue(replace(&["track-0", "track-1", "track-2"]))
        .await
        .expect("set queue");
    assert!(ack.rev > 0);

    let window = harness
        .api
        .queue_window(Page::default())
        .await
        .expect("window");
    assert_eq!(window.total, 3);
    assert_eq!(window.items[0].track.title, "track-0");
    assert_eq!(window.rev, ack.rev);
}

#[tokio::test]
async fn next_and_previous_load_the_selected_track() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["track-0", "track-1", "track-2"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    harness
        .api
        .player_command(PlayerCommand::Next)
        .await
        .expect("next");
    let state = wait_state(&harness.api, "second track", |state| {
        state.queue.index == Some(1) && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("track-1")
    );

    harness
        .api
        .player_command(PlayerCommand::Previous)
        .await
        .expect("previous");
    let state = wait_state(&harness.api, "first track", |state| {
        state.queue.index == Some(0) && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("track-0")
    );
}

#[tokio::test]
async fn set_mode_and_events_flow_through() {
    let harness = harness(|_| {});
    let mut events = harness.api.events();
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    assert!(matches!(
        events.next().await,
        Some(ApiEvent::QueueChanged { length: 2, .. })
    ));

    harness
        .api
        .player_command(PlayerCommand::SetMode {
            shuffle: Some(true),
            loop_mode: Some(LoopMode::Queue),
        })
        .await
        .expect("set mode");
    let state = harness.api.player_state().await.expect("state");
    assert!(state.queue.shuffle);
    assert_eq!(state.queue.loop_mode, LoopMode::Queue);
}

#[tokio::test]
async fn transport_commands_drive_the_engine_and_position_anchors() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["track-0"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    harness
        .api
        .player_command(PlayerCommand::Pause)
        .await
        .expect("pause");
    let paused = wait_state(&harness.api, "paused engine", |state| {
        state.phase == ApiPhase::Paused
    })
    .await;
    assert_eq!(paused.position.map(|anchor| anchor.playing), Some(false));

    harness
        .api
        .player_command(PlayerCommand::Play)
        .await
        .expect("play");
    let playing = wait_state(&harness.api, "resumed engine", |state| {
        state.phase == ApiPhase::Playing
    })
    .await;
    assert_eq!(playing.position.map(|anchor| anchor.playing), Some(true));

    harness
        .api
        .player_command(PlayerCommand::Toggle)
        .await
        .expect("toggle");
    wait_state(&harness.api, "toggle paused", |state| {
        state.phase == ApiPhase::Paused
    })
    .await;

    harness
        .api
        .player_command(PlayerCommand::Stop)
        .await
        .expect("stop");
    let stopped = harness.api.player_state().await.expect("state");
    assert_eq!(stopped.intent, Intent::Stopped);
    assert_eq!(stopped.phase, ApiPhase::Idle);
    assert_eq!(stopped.position.map(|anchor| anchor.ms), Some(0));
}

#[tokio::test]
async fn engine_position_ticks_do_not_become_one_hz_api_events() {
    let harness = harness(|_| {});
    let mut events = harness.api.session.subscribe();
    harness
        .api
        .set_queue(replace(&["track-0"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    for _ in 0..25 {
        harness.sink.pull(8192);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut anchors = 0;
    while let Ok((_, event)) = events.try_recv() {
        if matches!(event, ApiEvent::PlayerPosition { .. }) {
            anchors += 1;
        }
    }
    assert_eq!(anchors, 1, "only the initial play anchor is emitted");
}

#[tokio::test]
async fn pause_mid_load_cannot_restart_a_cancelled_session() {
    let gate = Arc::new((Mutex::new(true), Condvar::new()));
    let provider_gate = gate.clone();
    let provider: FactoryOverride = Arc::new(move |track| {
        Some(if track.title == "slow" {
            gated_factory(6, provider_gate.clone())
        } else {
            wav_factory(6)
        })
    });
    let harness = harness_with_provider(|_| {}, provider);
    harness
        .api
        .set_queue(replace(&["slow"]))
        .await
        .expect("set queue");
    wait_state(&harness.api, "loading intent", |state| {
        matches!(state.intent, Intent::Loading { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    harness
        .api
        .player_command(PlayerCommand::Pause)
        .await
        .expect("pause");
    {
        *gate.0.lock().expect("gate lock") = false;
        gate.1.notify_all();
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    let state = harness.api.player_state().await.expect("state");
    assert_eq!(state.intent, Intent::Stopped);
    assert_ne!(state.phase, ApiPhase::Playing);
}

#[tokio::test]
async fn resume_re_adopts_the_live_engine_token_after_mid_load_pause() {
    let gate = Arc::new((Mutex::new(true), Condvar::new()));
    let provider_gate = gate.clone();
    let provider: FactoryOverride = Arc::new(move |track| {
        Some(if track.title == "slow" {
            gated_factory(6, provider_gate.clone())
        } else {
            wav_factory(6)
        })
    });
    let harness = harness_with_provider(|_| {}, provider);
    harness
        .api
        .set_queue(replace(&["fast", "slow"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    harness
        .api
        .player_command(PlayerCommand::Next)
        .await
        .expect("next");
    wait_state(&harness.api, "second load resolving", |state| {
        matches!(state.intent, Intent::Loading { token: 2, .. })
    })
    .await;

    harness
        .api
        .player_command(PlayerCommand::Pause)
        .await
        .expect("pause");
    harness
        .api
        .player_command(PlayerCommand::Play)
        .await
        .expect("resume");
    let state = wait_state(&harness.api, "live token re-adopted", |state| {
        state.phase == ApiPhase::Playing && matches!(state.intent, Intent::Committed { token: 1 })
    })
    .await;
    assert_eq!(state.queue.index, Some(1));

    {
        *gate.0.lock().expect("gate lock") = false;
        gate.1.notify_all();
    }
}

#[tokio::test]
async fn newer_load_wins_when_a_cancelled_decode_finishes_late() {
    let gate = Arc::new((Mutex::new(true), Condvar::new()));
    let provider_gate = gate.clone();
    let provider: FactoryOverride = Arc::new(move |track| {
        Some(if track.title == "slow" {
            gated_factory(6, provider_gate.clone())
        } else {
            wav_factory(6)
        })
    });
    let harness = harness_with_provider(|_| {}, provider);
    harness
        .api
        .set_queue(replace(&["slow", "fast"]))
        .await
        .expect("set queue");
    tokio::time::sleep(Duration::from_millis(50)).await;
    harness
        .api
        .player_command(PlayerCommand::Next)
        .await
        .expect("next");
    let state = wait_state(&harness.api, "newer load committed", |state| {
        state.queue.index == Some(1) && matches!(state.intent, Intent::Committed { token: 2 })
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("fast")
    );

    {
        *gate.0.lock().expect("gate lock") = false;
        gate.1.notify_all();
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    let state = harness.api.player_state().await.expect("state");
    assert!(matches!(state.intent, Intent::Committed { token: 2 }));
    assert_eq!(state.queue.index, Some(1));
}

#[tokio::test]
async fn crossfade_load_can_be_superseded_without_a_stale_switch() {
    let gate = Arc::new((Mutex::new(true), Condvar::new()));
    let calls = Arc::new(Mutex::new(HashMap::<String, Arc<AtomicUsize>>::new()));
    let provider_gate = gate.clone();
    let provider_calls = calls.clone();
    let provider: FactoryOverride = Arc::new(move |track| {
        let counter = provider_calls
            .lock()
            .expect("calls lock")
            .entry(track.title.clone())
            .or_default()
            .clone();
        let call = counter.fetch_add(1, Ordering::Relaxed);
        Some(if track.title == "track-1" && call == 0 {
            gated_factory(6, provider_gate.clone())
        } else {
            wav_factory(6)
        })
    });
    let harness = harness_with_provider(|config| config.crossfade_seconds = 1, provider);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    drive_until(&harness, "crossfade resolving", |state| {
        matches!(
            state.intent,
            Intent::Loading {
                token: 2,
                from_token: Some(1)
            }
        )
    })
    .await;
    harness
        .api
        .player_command(PlayerCommand::Next)
        .await
        .expect("manual next supersedes fade");
    let state = wait_state(&harness.api, "replacement load committed", |state| {
        state.queue.index == Some(1) && matches!(state.intent, Intent::Committed { token: 3 })
    })
    .await;
    assert!(state.fading.is_none());

    {
        *gate.0.lock().expect("gate lock") = false;
        gate.1.notify_all();
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    let state = harness.api.player_state().await.expect("state");
    assert!(matches!(state.intent, Intent::Committed { token: 3 }));
    assert_eq!(state.queue.index, Some(1));
}

#[tokio::test]
async fn end_of_queue_pauses_the_live_engine_session() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["short-track"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    let pauses_before = harness.sink.pause_calls();
    let state = drive_until(&harness, "end-of-queue stop", |state| {
        state.intent == Intent::Stopped && state.phase == ApiPhase::Ended
    })
    .await;
    assert_eq!(state.queue.index, Some(0));
    assert!(harness.sink.pause_calls() > pauses_before);
}

#[tokio::test]
async fn seek_during_crossfade_is_guarded_to_the_visible_token() {
    let harness = harness(|config| config.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    drive_until(&harness, "running crossfade", |state| {
        state.fading.is_some() && matches!(state.intent, Intent::Committed { token: 2 })
    })
    .await;

    harness
        .api
        .player_command(PlayerCommand::Seek { position_ms: 1_500 })
        .await
        .expect("seek visible track");
    let state = wait_state(&harness.api, "outgoing session restored", |state| {
        state.fading.is_none()
            && state.queue.index == Some(0)
            && matches!(state.intent, Intent::Committed { token: 1 })
    })
    .await;
    assert_eq!(state.position.map(|position| position.ms), Some(1_500));
}

#[tokio::test]
async fn radio_tracks_reject_seek_commands() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["radio:station:main"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    let error = harness
        .api
        .player_command(PlayerCommand::Seek { position_ms: 1_000 })
        .await
        .expect_err("radio seek rejected");
    assert_eq!(error.code, ErrorCode::InvalidInput);

    let pauses_before = harness.sink.pause_calls();
    harness
        .api
        .player_command(PlayerCommand::Pause)
        .await
        .expect("pause radio");
    let state = wait_state(&harness.api, "radio stopped", |state| {
        state.phase == ApiPhase::Idle
    })
    .await;
    assert_eq!(state.intent, Intent::Committed { token: 1 });
    assert_eq!(harness.sink.pause_calls(), pauses_before);
}

#[test]
fn radio_sentinel_becomes_wire_kind() {
    let track = test_track(&"radio:station:main".to_string());
    let now = now_playing_from(&track);
    assert_eq!(now.kind, TrackKind::Radio);
    assert_eq!(now.duration_ms, None);
    assert!(!now.seekable);
}

struct MemoryStore {
    saved: Mutex<Vec<db::QueueSnapshot>>,
}

#[async_trait::async_trait]
impl crate::persistence::QueueStore for MemoryStore {
    async fn load(&self) -> Option<db::QueueSnapshot> {
        None
    }

    async fn save(&self, snapshot: db::QueueSnapshot) {
        self.saved.lock().expect("store lock").push(snapshot);
    }
}

struct MemoryRecorder {
    recents: Mutex<Vec<String>>,
    listens: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl PlaybackRecorder for MemoryRecorder {
    async fn record_recent(&self, track: &Track) {
        self.recents
            .lock()
            .expect("recorder lock")
            .push(track.title.clone());
    }

    async fn bump_listen_count(&self, track: &Track) {
        self.listens
            .lock()
            .expect("recorder lock")
            .push(track.title.clone());
    }
}

#[tokio::test]
async fn recents_record_once_and_completion_bumps_listens() {
    let recorder = Arc::new(MemoryRecorder {
        recents: Mutex::new(Vec::new()),
        listens: Mutex::new(Vec::new()),
    });
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let services = PlaybackServices {
        recorder: Some(recorder.clone()),
        ..Default::default()
    };
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        services,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    let api = LocalApi::new(session);
    let harness = Harness { api, sink };

    harness
        .api
        .set_queue(replace(&["short-a", "short-b"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    drive_until(&harness, "auto-advance to second track", |state| {
        state.queue.index == Some(1) && matches!(state.intent, Intent::Committed { .. })
    })
    .await;

    tokio::time::sleep(Duration::from_millis(50)).await;
    let recents = recorder.recents.lock().expect("lock").clone();
    assert_eq!(recents, vec!["short-a".to_string(), "short-b".to_string()]);
    let listens = recorder.listens.lock().expect("lock").clone();
    assert_eq!(listens, vec!["short-a".to_string()]);

    harness
        .api
        .player_command(PlayerCommand::Pause)
        .await
        .expect("pause");
    harness
        .api
        .player_command(PlayerCommand::Play)
        .await
        .expect("resume");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let recents = recorder.recents.lock().expect("lock").clone();
    assert_eq!(recents.len(), 2, "resume must not re-record the same track");
}

#[tokio::test]
async fn restore_seeds_a_paused_resume_point_and_play_continues_there() {
    let harness = harness(|_| {});
    let snapshot = db::QueueSnapshot {
        version: 1,
        queue: ["track-0", "track-1", "track-2"]
            .iter()
            .map(|key| test_track(&(*key).to_string()))
            .collect(),
        current_queue_index: 1,
        progress_secs: 2,
        shuffle_order: Vec::new(),
        shuffle_enabled: false,
    };
    harness
        .api
        .session
        .restore_queue(snapshot)
        .await
        .expect("restore");

    let state = harness.api.player_state().await.expect("state");
    assert_eq!(state.phase, ApiPhase::Idle);
    assert_eq!(state.queue.index, Some(1));
    assert_eq!(
        state.track.as_ref().map(|t| t.title.as_str()),
        Some("track-1")
    );
    let anchor = state.position.expect("restored anchor");
    assert_eq!(anchor.ms, 2000);
    assert!(!anchor.playing);

    harness
        .api
        .player_command(PlayerCommand::Play)
        .await
        .expect("play");
    let state = wait_committed(&harness.api).await;
    let anchor = state.position.expect("anchor");
    assert!(anchor.ms >= 2000, "resumed at {}ms", anchor.ms);
}

#[tokio::test]
async fn persist_now_writes_the_current_snapshot() {
    let store = Arc::new(MemoryStore {
        saved: Mutex::new(Vec::new()),
    });
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let services = PlaybackServices {
        queue_store: Some(store.clone()),
        ..Default::default()
    };
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        services,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    let api = LocalApi::new(session.clone());

    api.set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&api).await;
    session.persist_now().await;

    let saved = store.saved.lock().expect("store lock");
    let last = saved.last().expect("at least one snapshot");
    assert_eq!(last.version, 1);
    assert_eq!(last.queue.len(), 2);
    assert_eq!(last.current_queue_index, 0);
    assert!(!last.shuffle_enabled);
}

#[tokio::test]
async fn queue_edit_moves_removes_and_guards_the_playing_row() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["/a.wav", "/b.wav", "/c.wav"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    let err = harness
        .api
        .queue_edit(QueueEdit::Remove { index: 0 })
        .await
        .expect_err("removing the playing row is refused");
    assert_eq!(err.code, ErrorCode::InvalidInput);

    harness
        .api
        .queue_edit(QueueEdit::Move { from: 1, to: 2 })
        .await
        .expect("move");
    let window = harness
        .api
        .queue_window(Page::default())
        .await
        .expect("window");
    assert_eq!(window.items[1].track.title, "/c.wav");
    assert_eq!(window.items[2].track.title, "/b.wav");

    harness
        .api
        .queue_edit(QueueEdit::Remove { index: 2 })
        .await
        .expect("remove tail");
    let window = harness
        .api
        .queue_window(Page::default())
        .await
        .expect("window");
    assert_eq!(window.total, 2);

    let err = harness
        .api
        .queue_edit(QueueEdit::Jump { index: 9 })
        .await
        .expect_err("out of range jump");
    assert_eq!(err.code, ErrorCode::InvalidInput);

    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 1 })
        .await
        .expect("jump");
    let state = wait_state(&harness.api, "jump target playing", |state| {
        state.queue.index == Some(1) && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|t| t.title.as_str()),
        Some("/c.wav")
    );
}

#[tokio::test]
async fn replay_ring_serves_gaps_and_flags_overflow() {
    let harness = harness(|_| {});
    harness
        .api
        .player_command(PlayerCommand::SetVolume { volume: 0.5 })
        .await
        .expect("volume");

    let (resync, replayed) = harness.api.session.replay_since(0);
    assert!(!resync);
    assert!(!replayed.is_empty());
    let ids: Vec<u64> = replayed.iter().map(|(sequence, _)| *sequence).collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));

    let newest = *ids.last().expect("ids");
    let (resync, tail) = harness.api.session.replay_since(newest);
    assert!(!resync);
    assert!(tail.is_empty());

    for step in 0..(EVENT_BUFFER as u64 + 40) {
        harness
            .api
            .player_command(PlayerCommand::SetVolume {
                volume: (step % 100) as f32 / 100.0,
            })
            .await
            .expect("volume");
    }
    let (resync, dropped) = harness.api.session.replay_since(1);
    assert!(resync);
    assert!(dropped.is_empty());
}

#[tokio::test]
async fn radio_metadata_updates_the_displayed_track() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["radio:station:stream"]))
        .await
        .expect("set queue");
    let state = wait_state(&harness.api, "radio committed", |state| {
        matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    let token = match state.intent {
        Intent::Committed { token } => token,
        _ => unreachable!(),
    };

    harness
        .api
        .session
        .cmd_tx
        .send(SessionCmd::RadioMetadata {
            token,
            title: "Song Title".into(),
            artist: Some("Some Artist".into()),
        })
        .expect("send metadata");
    let state = wait_state(&harness.api, "metadata applied", |state| {
        state
            .track
            .as_ref()
            .is_some_and(|t| t.title == "Song Title")
    })
    .await;
    let track = state.track.expect("track");
    assert_eq!(track.artist, "Some Artist");
    assert_eq!(track.kind, TrackKind::Radio);

    harness
        .api
        .session
        .cmd_tx
        .send(SessionCmd::RadioMetadata {
            token: token + 999,
            title: "Stale".into(),
            artist: None,
        })
        .expect("send stale");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let state = harness.api.player_state().await.expect("state");
    assert_eq!(
        state.track.as_ref().map(|t| t.title.as_str()),
        Some("Song Title")
    );
}
