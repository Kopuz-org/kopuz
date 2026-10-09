//! Session behavior tests: engine races, queue edits, persistence,
//! recording, and the replay ring, all over a fake sink.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

use api::prelude::*;
use api::{ErrorCode, LoopMode, TrackKind};
use futures_util::StreamExt;
use player::engine::{AudioSink, DataCallback, DataCallbackFactory, SinkConfig};

use super::*;
use crate::wire::track_info;

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
            // The way a source answers: the seed among its mix.
            QueueContext::TrackRadio { key } => Ok([
                key.as_str(),
                "radio-short-1",
                "radio-short-explicit-2",
                "radio-short-3",
            ]
            .iter()
            .map(|key| test_track(&key.to_string()))
            .collect()),
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
        id: if key.starts_with("unavailable-server:") {
            reader::models::TrackId::Server {
                service: config::MusicService::Jellyfin,
                item_id: key.clone(),
            }
        } else if let Some(item_id) = key.strip_prefix("spotify:") {
            reader::models::TrackId::Server {
                service: config::MusicService::Spotify,
                item_id: item_id.to_string(),
            }
        } else {
            reader::models::TrackId::Local(PathBuf::from(key))
        },
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
        credits: if key.contains("credited") {
            vec![reader::models::ArtistCredit::linked("Ada", "ar-ada")]
        } else {
            Vec::new()
        },
        artists: vec![],
        replay_gain: config::ReplayGainInfo::default(),
        explicit: key.contains("explicit"),
        plays: None,
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

fn gated_failing_factory(gate: Arc<(Mutex<bool>, Condvar)>) -> SourceFactory {
    Box::new(move || {
        let (lock, ready) = &*gate;
        let mut blocked = lock.lock().expect("gate lock");
        while *blocked {
            blocked = ready.wait(blocked).expect("gate wait");
        }
        Err("synthetic source failure".to_string())
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
    harness_with_services(configure, provider, None)
}

fn harness_with_services(
    configure: impl FnOnce(&mut config::AppConfig),
    provider: FactoryOverride,
    recorder: Option<Arc<dyn PlaybackRecorder>>,
) -> Harness {
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut services = PlaybackServices {
        recorder,
        ..Default::default()
    };
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

fn replay_gain_harness(provider: FactoryOverride) -> Harness {
    struct GainLibrary;
    #[async_trait::async_trait]
    impl QueueMaterializer for GainLibrary {
        async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
            let mut tracks = StubLibrary.materialize(context).await?;
            for track in &mut tracks {
                track.album_id = "album".into();
                track.replay_gain = config::ReplayGainInfo {
                    track_gain_db: Some(-6.0),
                    album_gain_db: Some(-12.0),
                    ..Default::default()
                };
            }
            Ok(tracks)
        }
    }
    let sink = FakeSinkHandle::default();
    let player = Player::try_with_sink(Box::new(FakeSink(sink.clone()))).unwrap();
    let mut services = PlaybackServices::default();
    services.config.crossfade_seconds = 0;
    services.config.volume = 1.0;
    services.config.replay_gain.mode = config::ReplayGainMode::Auto;
    let session =
        SessionHandle::spawn_with_factory(Arc::new(GainLibrary), player, services, provider);
    Harness {
        api: LocalApi::new(session),
        sink,
    }
}

async fn wait_replay_gain(harness: &Harness, db: f32) {
    let expected = (10_000.0 / 32_768.0) * 10.0_f32.powf(db / 20.0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let peak = harness
            .sink
            .pull(1024)
            .into_iter()
            .fold(0.0_f32, |p, s| p.max(s.abs()));
        if (peak - expected).abs() < expected * 0.01 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "gain {db} dB: {peak} vs {expected}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn replay_gain_auto_follows_shuffle_and_queue_edits() {
    let harness = replay_gain_harness(Arc::new(|_| Some(wav_factory(6))));
    harness.api.set_queue(replace(&["a", "b"])).await.unwrap();
    wait_committed(&harness.api).await;
    wait_replay_gain(&harness, -12.0).await;
    for shuffle in [true, false] {
        harness
            .api
            .player_command(PlayerCommand::SetMode {
                shuffle: Some(shuffle),
                loop_mode: None,
            })
            .await
            .unwrap();
        wait_replay_gain(&harness, if shuffle { -6.0 } else { -12.0 }).await;
    }
    harness
        .api
        .queue_edit(QueueEdit::Remove { index: 1 })
        .await
        .unwrap();
    wait_replay_gain(&harness, -6.0).await;
    harness
        .api
        .set_queue(enqueue(QueueMode::Append, &["b"]))
        .await
        .unwrap();
    wait_replay_gain(&harness, -12.0).await;
}

#[tokio::test]
async fn replay_gain_settings_reach_playing_audio_through_config_api() {
    let mut harness = harness(|config| config.volume = 1.0);
    let dir = tempfile::tempdir().unwrap();
    let database = db::init(&dir.path().join("test.db")).await.unwrap();
    let config = harness.api.session.config_rx.borrow().clone();
    let service = Arc::new(crate::config_service::ConfigService::new(
        database,
        dir.path().join("settings.toml"),
        config,
    ));
    service.attach_session(harness.api.session.clone());
    harness.api = harness.api.with_config(service);
    harness.api.set_queue(replace(&["untagged"])).await.unwrap();
    wait_committed(&harness.api).await;
    wait_replay_gain(&harness, 0.0).await;

    for (mode, preamp, expected) in [
        (config::ReplayGainMode::Track, -12.0, -12.0),
        (config::ReplayGainMode::Album, -6.0, -6.0),
        (config::ReplayGainMode::Off, -12.0, 0.0),
        (config::ReplayGainMode::Auto, -12.0, -12.0),
    ] {
        let mut next = harness.api.config().await.unwrap().config;
        next.replay_gain.mode = mode;
        next.replay_gain.preamp_db = preamp;
        harness.api.set_config(next).await.unwrap();
        wait_replay_gain(&harness, expected).await;
    }
}

#[tokio::test]
async fn replay_gain_auto_refreshes_while_the_decoder_is_probing() {
    let gate = Arc::new((Mutex::new(true), Condvar::new()));
    let provider_gate = gate.clone();
    let harness = replay_gain_harness(Arc::new(move |_| {
        Some(gated_factory(6, provider_gate.clone()))
    }));
    harness.api.set_queue(replace(&["a", "b"])).await.unwrap();
    harness
        .api
        .player_command(PlayerCommand::SetMode {
            shuffle: Some(true),
            loop_mode: None,
        })
        .await
        .unwrap();
    *gate.0.lock().unwrap() = false;
    gate.1.notify_all();
    wait_committed(&harness.api).await;
    wait_replay_gain(&harness, -6.0).await;
}

fn enqueue(mode: QueueMode, keys: &[&str]) -> SetQueueRequest {
    SetQueueRequest {
        mode,
        context: QueueContext::Tracks {
            keys: keys.iter().map(|key| (*key).to_string()).collect(),
        },
        start_index: None,
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

/// A frontend opens the artist and album off the state's row, so it must be the queue's own.
#[tokio::test]
async fn the_playing_row_is_the_one_the_queue_holds() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["credited-0", "track-1"]))
        .await
        .expect("set queue");
    let state = wait_state(&harness.api, "a track to show", |state| {
        state.track.is_some()
    })
    .await;
    let queue = harness.api.queue_snapshot().await.expect("queue");
    let playing = state.track.expect("track");

    assert_eq!(
        playing,
        queue.items[queue.position.expect("position") as usize]
    );
    assert_eq!(playing.credits[0].name, "Ada");
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
async fn unavailable_queue_targets_do_not_replace_the_current_track() {
    let provider: FactoryOverride = Arc::new(|track| {
        (!track.title.starts_with("unavailable-server:"))
            .then(|| wav_factory(track.duration.min(6)))
    });
    let harness = harness_with_provider(|_| {}, provider);
    let mut request = replace(&[
        "unavailable-server:previous",
        "track-0",
        "unavailable-server:next",
    ]);
    request.start_index = Some(1);
    harness
        .api
        .set_queue(request)
        .await
        .expect("set initial queue");
    let before = wait_committed(&harness.api).await;

    for result in [
        harness.api.queue_edit(QueueEdit::Jump { index: 2 }).await,
        harness.api.player_command(PlayerCommand::Next).await,
        harness.api.player_command(PlayerCommand::Previous).await,
    ] {
        let error = result.expect_err("unavailable target is rejected");
        assert_eq!(error.code, ErrorCode::SourceUnreachable);
        let state = harness.api.player_state().await.expect("state");
        assert_eq!(state.queue.index, before.queue.index);
        assert_eq!(state.track, before.track);
        assert_eq!(state.intent, before.intent);
    }

    let error = harness
        .api
        .set_queue(replace(&["unavailable-server:replacement"]))
        .await
        .expect_err("unavailable replacement is rejected");
    assert_eq!(error.code, ErrorCode::SourceUnreachable);
    let state = harness.api.player_state().await.expect("state");
    assert_eq!(state.queue.length, 3);
    assert_eq!(state.queue.index, before.queue.index);
    assert_eq!(state.track, before.track);
    assert_eq!(state.intent, before.intent);
}

#[tokio::test]
async fn replacing_with_an_empty_queue_stops_playback() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["track-0"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    harness
        .api
        .set_queue(replace(&[]))
        .await
        .expect("clear queue");
    let state = harness.api.player_state().await.expect("state");
    assert_eq!(state.queue.length, 0);
    assert_eq!(state.queue.index, None);
    assert_eq!(state.track, None);
    assert_eq!(state.intent, Intent::Stopped);
    assert_eq!(state.phase, ApiPhase::Idle);
    assert_eq!(state.position.expect("position").ms, 0);
}

#[tokio::test]
async fn set_mode_and_events_flow_through() {
    let harness = harness(|_| {});
    let mut events = harness.api.events();
    // The stream greets with Resync, same as Subscribe does over the wire,
    // so a consumer knows it is attached and its mirror is empty.
    assert!(matches!(events.next().await, Some(ApiEvent::Resync)));

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
    loop {
        match events.try_recv() {
            Ok(ApiEvent::PlayerPosition { .. }) => anchors += 1,
            Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                break;
            }
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

/// Play far enough into the first track that the crossfade into the second is
/// pending, and leave it there: the fake sink only advances while `drive_until`
/// pulls, so the transition cannot commit under the assertions that follow.
async fn armed_crossfade(harness: &Harness) {
    drive_until(harness, "crossfade armed", |state| {
        state.fading.is_some()
            || matches!(
                state.intent,
                Intent::Loading {
                    from_token: Some(_),
                    ..
                }
            )
    })
    .await;
}

async fn queue_titles(api: &LocalApi) -> Vec<String> {
    api.queue_window(Page::default())
        .await
        .expect("window")
        .items
        .into_iter()
        .map(|item| item.track.title)
        .collect()
}

#[tokio::test]
async fn append_during_a_crossfade_survives_the_commit() {
    let harness = harness(|config| config.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    armed_crossfade(&harness).await;

    harness
        .api
        .set_queue(enqueue(QueueMode::Append, &["track-2"]))
        .await
        .expect("append mid-crossfade");
    assert_eq!(queue_titles(&harness.api).await.len(), 3);

    let state = drive_until(&harness, "crossfade committed", |state| {
        state.queue.index == Some(1) && state.fading.is_none()
    })
    .await;
    assert_eq!(state.queue.length, 3);
    assert_eq!(
        queue_titles(&harness.api).await,
        vec!["track-0", "track-1", "track-2"]
    );
}

#[tokio::test]
async fn play_next_during_a_crossfade_lands_after_the_incoming_track() {
    let harness = harness(|config| config.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    armed_crossfade(&harness).await;

    harness
        .api
        .set_queue(enqueue(QueueMode::PlayNext, &["track-2"]))
        .await
        .expect("play next mid-crossfade");

    let state = drive_until(&harness, "crossfade committed", |state| {
        state.queue.index == Some(1) && state.fading.is_none()
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("track-1")
    );
    assert_eq!(
        queue_titles(&harness.api).await,
        vec!["track-0", "track-1", "track-2"]
    );
}

#[tokio::test]
async fn remove_during_a_crossfade_is_not_resurrected_by_the_commit() {
    let harness = harness(|config| config.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1", "track-2"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    armed_crossfade(&harness).await;

    harness
        .api
        .queue_edit(QueueEdit::Remove { index: 2 })
        .await
        .expect("remove the tail mid-crossfade");

    let state = drive_until(&harness, "crossfade committed", |state| {
        state.queue.index == Some(1) && state.fading.is_none()
    })
    .await;
    assert_eq!(state.queue.length, 2);
    assert_eq!(queue_titles(&harness.api).await, vec!["track-0", "track-1"]);
}

#[tokio::test]
async fn removing_the_track_being_faded_into_is_refused() {
    let harness = harness(|config| config.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    armed_crossfade(&harness).await;

    let error = harness
        .api
        .queue_edit(QueueEdit::Remove { index: 1 })
        .await
        .expect_err("the incoming track cannot be removed");
    assert_eq!(error.code, ErrorCode::InvalidInput);

    let state = drive_until(&harness, "crossfade committed", |state| {
        state.queue.index == Some(1) && state.fading.is_none()
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("track-1")
    );
}

#[tokio::test]
async fn move_during_a_crossfade_follows_the_incoming_track() {
    let harness = harness(|config| config.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1", "track-2"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    armed_crossfade(&harness).await;

    harness
        .api
        .queue_edit(QueueEdit::Move { from: 2, to: 1 })
        .await
        .expect("move the tail ahead of the incoming track");
    assert_eq!(
        queue_titles(&harness.api).await,
        vec!["track-0", "track-2", "track-1"]
    );

    let state = drive_until(&harness, "crossfade committed", |state| {
        state.fading.is_none() && state.queue.index == Some(2)
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("track-1")
    );
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
    let now = track_info(&track, &config::AppConfig::default());
    assert_eq!(now.kind, TrackKind::Radio);
    assert_eq!(now.duration_ms, None);
    assert!(!now.seekable);
}

/// A server track's key and uid are different strings, and a consumer that
/// matches the wrong one silently works on local tracks (where they are
/// equal) while missing every server track.
#[test]
fn the_playing_row_carries_both_the_library_ref_and_the_source_qualified_id() {
    let server = Track {
        id: reader::models::TrackId::Server {
            service: config::MusicService::YtMusic,
            item_id: "abc123".to_string(),
        },
        ..test_track(&"abc123".to_string())
    };
    let now = track_info(&server, &config::AppConfig::default());
    assert_eq!(now.key, "abc123", "key is the bare library ref");
    assert_eq!(now.uid, "ytmusic:abc123", "uid is source-qualified");
    assert_ne!(now.key, now.uid, "the two must not be conflated");

    // Local tracks are the case that hides the mistake: both are the path.
    let local = test_track(&"/music/a.flac".to_string());
    let now = track_info(&local, &config::AppConfig::default());
    assert_eq!(now.key, now.uid);
}

struct MemoryStore {
    saved: Mutex<Vec<(config::Source, db::QueueSnapshot)>>,
}

#[async_trait::async_trait]
impl crate::persistence::QueueStore for MemoryStore {
    async fn load(&self, source: &config::Source) -> Result<db::QueueSnapshot, db::DbError> {
        let saved = self.saved.lock().expect("store lock");
        Ok(saved
            .iter()
            .rev()
            .find(|(at, _)| at == source)
            .map(|(_, snapshot)| snapshot.clone())
            .unwrap_or_default())
    }

    async fn save(&self, source: &config::Source, snapshot: db::QueueSnapshot) {
        self.saved
            .lock()
            .expect("store lock")
            .push((source.clone(), snapshot));
    }

    async fn forget(&self, source: &config::Source) {
        self.saved
            .lock()
            .expect("store lock")
            .retain(|(at, _)| at != source);
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
    let harness = harness_with_services(
        |_| {},
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
        Some(recorder.clone()),
    );

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

/// Writes play history slowly, into the same log a listening client writes what it hears.
struct SlowRecorder {
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl SlowRecorder {
    async fn write(&self) {
        tokio::time::sleep(Duration::from_millis(30)).await;
        self.log.lock().expect("log lock").push("written");
    }
}

#[async_trait::async_trait]
impl PlaybackRecorder for SlowRecorder {
    async fn record_recent(&self, _: &Track) {
        self.write().await;
    }

    async fn bump_listen_count(&self, _: &Track) {
        self.write().await;
    }
}

/// A client re-reading play history on its event must find the write that caused it.
#[tokio::test]
async fn play_history_is_announced_after_it_is_written() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let harness = harness_with_services(
        |_| {},
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
        Some(Arc::new(SlowRecorder { log: log.clone() })),
    );
    let mut events = harness.api.session.subscribe();
    let heard = log.clone();
    tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if matches!(
                event,
                ApiEvent::LibraryInvalidated {
                    table: api::Table::Recents
                }
            ) {
                heard.lock().expect("log lock").push("announced");
            }
        }
    });

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
    tokio::time::sleep(Duration::from_millis(200)).await;

    let log = log.lock().expect("log lock").clone();
    let (mut written, mut announced) = (0, 0);
    for entry in &log {
        match *entry {
            "written" => written += 1,
            _ => announced += 1,
        }
        assert!(announced <= written, "announced before written: {log:?}");
    }
    assert_eq!(
        (written, announced),
        (3, 3),
        "two recents and a listen: {log:?}"
    );
}

#[tokio::test]
async fn failed_crossfade_preparation_records_no_listen_and_can_retry() {
    let recorder = Arc::new(MemoryRecorder {
        recents: Mutex::new(Vec::new()),
        listens: Mutex::new(Vec::new()),
    });
    let failure_gate = Arc::new((Mutex::new(true), Condvar::new()));
    let target_attempts = Arc::new(AtomicUsize::new(0));
    let provider_attempts = target_attempts.clone();
    let provider_gate = failure_gate.clone();
    let provider: FactoryOverride = Arc::new(move |track| {
        Some(
            if track.title == "track-1" && provider_attempts.fetch_add(1, Ordering::Relaxed) == 0 {
                gated_failing_factory(provider_gate.clone())
            } else {
                wav_factory(track.duration.min(6))
            },
        )
    });
    let harness = harness_with_services(
        |config| config.crossfade_seconds = 1,
        provider,
        Some(recorder.clone()),
    );

    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    drive_until(&harness, "first crossfade attempt", |state| {
        matches!(
            state.intent,
            Intent::Loading {
                token: 2,
                from_token: Some(1)
            }
        )
    })
    .await;
    {
        *failure_gate.0.lock().expect("gate lock") = false;
        failure_gate.1.notify_all();
    }
    wait_state(&harness.api, "failed crossfade restored", |state| {
        matches!(state.intent, Intent::Committed { token: 1 }) && state.error.is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(recorder.listens.lock().expect("lock").is_empty());

    drive_until(&harness, "retried crossfade committed", |state| {
        state.queue.index == Some(1) && matches!(state.intent, Intent::Committed { token: 3 })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        recorder.listens.lock().expect("lock").as_slice(),
        ["track-0"]
    );
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
    let (_, last) = saved.last().expect("at least one snapshot");
    assert_eq!(last.version, 1);
    assert_eq!(last.queue.len(), 2);
    assert_eq!(last.current_queue_index, 0);
    assert!(!last.shuffle_enabled);
}

/// Each source keeps its queue: leaving one parks it, coming back resumes it, and a source never visited starts empty.
#[tokio::test]
async fn switching_sources_parks_and_resumes_each_queue() {
    let store = Arc::new(MemoryStore {
        saved: Mutex::new(Vec::new()),
    });
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut on_a = config::AppConfig::default();
    on_a.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    let mut on_b = on_a.clone();
    on_b.active_source = config::Source::LocalLibrary("local:b".into());
    let services = PlaybackServices {
        config: on_a.clone(),
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
    api.set_queue(replace(&["a-0", "a-1"]))
        .await
        .expect("set queue");
    wait_committed(&api).await;
    let on_a_titles = queue_titles(&api).await;

    session.set_config(on_b.clone(), vec!["active_source".into()]);
    session.persist_now().await;
    assert!(queue_titles(&api).await.is_empty(), "B was never visited");
    api.set_queue(replace(&["b-0"])).await.expect("set queue");
    wait_committed(&api).await;

    session.set_config(on_a.clone(), vec!["active_source".into()]);
    session.persist_now().await;
    assert_eq!(
        queue_titles(&api).await,
        on_a_titles,
        "A resumes its own queue"
    );

    session.set_config(on_b, vec!["active_source".into()]);
    session.persist_now().await;
    assert_eq!(queue_titles(&api).await.len(), 1, "and B its own");
}

/// A deleted source is not parked: its queue would outlive it.
#[tokio::test]
async fn leaving_a_deleted_source_parks_nothing() {
    let store = Arc::new(MemoryStore {
        saved: Mutex::new(Vec::new()),
    });
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut on_b = config::AppConfig::default();
    on_b.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    on_b.active_source = config::Source::LocalLibrary("local:b".into());
    let services = PlaybackServices {
        config: on_b.clone(),
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
    api.set_queue(replace(&["b-0"])).await.expect("set queue");
    wait_committed(&api).await;
    let saves_of_b = || {
        store
            .saved
            .lock()
            .expect("store lock")
            .iter()
            .filter(|(at, _)| *at == config::Source::LocalLibrary("local:b".into()))
            .count()
    };
    session.persist_now().await;
    assert!(saves_of_b() > 0, "the queue was stored while B was active");

    let mut deleted = on_b.clone();
    deleted.remove_local_source("local:b");
    session.set_config(
        deleted,
        vec!["local_sources".into(), "active_source".into()],
    );
    session.persist_now().await;

    assert_eq!(
        saves_of_b(),
        0,
        "a deleted source's queue is dropped, not parked"
    );
    assert!(queue_titles(&api).await.is_empty());
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

/// Records what the daemon asked the integration to do, standing in for a
/// Spotify Connect device.
#[derive(Default)]
struct StubExternal {
    calls: Mutex<Vec<String>>,
}

impl StubExternal {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }

    fn record(&self, call: &str) {
        self.calls.lock().expect("calls").push(call.to_string());
    }
}

#[async_trait::async_trait]
impl crate::external::ExternalPlayer for StubExternal {
    fn kind(&self) -> &'static str {
        "stub"
    }

    fn service(&self) -> config::MusicService {
        config::MusicService::Spotify
    }

    async fn load(&self, track: &Track, _artwork: Option<String>) -> Result<(), ApiError> {
        self.record(&format!("load:{}", track.title));
        Ok(())
    }

    async fn resume(&self) -> Result<(), ApiError> {
        self.record("resume");
        Ok(())
    }

    async fn pause(&self) -> Result<(), ApiError> {
        self.record("pause");
        Ok(())
    }

    async fn seek(&self, position_ms: u64) -> Result<(), ApiError> {
        self.record(&format!("seek:{position_ms}"));
        Ok(())
    }

    async fn set_volume(&self, volume: f32) -> Result<(), ApiError> {
        self.record(&format!("volume:{volume}"));
        Ok(())
    }

    async fn stop(&self) -> Result<(), ApiError> {
        self.record("stop");
        Ok(())
    }
}

fn external_track(title: &str) -> Track {
    Track {
        id: reader::models::TrackId::Server {
            service: config::MusicService::Spotify,
            item_id: title.to_string(),
        },
        cover: None,
        album_id: String::new(),
        title: title.to_string(),
        artist: "Artist".into(),
        album: "Album".into(),
        duration: 180,
        khz: 44,
        bitrate: 320,
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
    }
}

#[tokio::test]
async fn transport_commands_reach_the_integration_that_owns_playback() {
    let harness = harness(|_| {});
    let stub = Arc::new(StubExternal::default());
    harness.api.session.attach_external(stub.clone());
    harness.api.session.report_external(crate::ExternalReport {
        track: Some(external_track("remote")),
        position_ms: 1000,
        playing: true,
        ..Default::default()
    });

    let state = wait_state(&harness.api, "external track shown", |state| {
        state.track.as_ref().is_some_and(|t| t.title == "remote")
    })
    .await;
    assert_eq!(
        state.external.as_ref().map(|e| e.kind.as_str()),
        Some("stub"),
        "clients are told playback is external"
    );
    assert_eq!(state.phase, ApiPhase::Playing);

    for command in [
        PlayerCommand::Next,
        PlayerCommand::Previous,
        PlayerCommand::Seek { position_ms: 4200 },
        PlayerCommand::Toggle,
    ] {
        harness
            .api
            .player_command(command)
            .await
            .expect("command accepted");
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        stub.calls(),
        vec!["seek:4200", "pause"],
        "next and previous drive the queue, not the integration"
    );
}

/// Switching source replaces what is playing, an integration's device included, so commands stop reaching it.
#[tokio::test]
async fn switching_source_releases_the_integration_that_owned_playback() {
    let harness = harness(|_| {});
    let stub = Arc::new(StubExternal::default());
    harness.api.session.attach_external(stub.clone());
    harness.api.session.report_external(crate::ExternalReport {
        track: Some(external_track("remote")),
        position_ms: 1000,
        playing: true,
        ..Default::default()
    });
    wait_state(&harness.api, "external track shown", |state| {
        state.external.is_some()
    })
    .await;

    let mut elsewhere = config::AppConfig::default();
    elsewhere.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    elsewhere.active_source = config::Source::LocalLibrary("local:b".into());
    harness
        .api
        .session
        .set_config(elsewhere, vec!["active_source".into()]);
    let state = wait_state(&harness.api, "playback handed back", |state| {
        state.external.is_none()
    })
    .await;
    assert_ne!(state.phase, ApiPhase::Playing);
    harness
        .api
        .player_command(PlayerCommand::Toggle)
        .await
        .expect("toggle accepted");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        stub.calls(),
        vec!["stop"],
        "the device is stopped once and hears nothing after"
    );
}

/// Cannot read one source's queue, as a damaged or locked database would.
struct UnreadableStore {
    inner: MemoryStore,
    unreadable: config::Source,
}

#[async_trait::async_trait]
impl crate::persistence::QueueStore for UnreadableStore {
    async fn load(&self, source: &config::Source) -> Result<db::QueueSnapshot, db::DbError> {
        if *source == self.unreadable {
            return Err(db::DbError::Backend("unreadable".into()));
        }
        self.inner.load(source).await
    }

    async fn save(&self, source: &config::Source, snapshot: db::QueueSnapshot) {
        self.inner.save(source, snapshot).await;
    }

    async fn forget(&self, source: &config::Source) {
        self.inner.forget(source).await;
    }
}

/// A queue that could not be read is not saved over, until a new one is built in its place.
#[tokio::test]
async fn an_unreadable_queue_is_not_saved_over() {
    let b = config::Source::LocalLibrary("local:b".into());
    let store = Arc::new(UnreadableStore {
        inner: MemoryStore {
            saved: Mutex::new(Vec::new()),
        },
        unreadable: b.clone(),
    });
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut on_a = config::AppConfig::default();
    on_a.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    let mut on_b = on_a.clone();
    on_b.active_source = b.clone();
    let services = PlaybackServices {
        config: on_a.clone(),
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
    api.set_queue(replace(&["a-0"])).await.expect("set queue");
    wait_committed(&api).await;
    let saves_of_b = || {
        store
            .inner
            .saved
            .lock()
            .expect("store lock")
            .iter()
            .filter(|(at, _)| *at == b)
            .count()
    };

    session.set_config(on_b.clone(), vec!["active_source".into()]);
    session.persist_now().await;
    assert!(queue_titles(&api).await.is_empty());
    api.player_command(PlayerCommand::SetVolume { volume: 0.3 })
        .await
        .expect("a change that marks the session dirty");
    api.player_command(PlayerCommand::SetMode {
        shuffle: Some(true),
        loop_mode: None,
    })
    .await
    .expect("a shuffle toggle on nothing builds no queue");
    session.persist_now().await;
    session.set_config(on_a, vec!["active_source".into()]);
    session.persist_now().await;
    assert_eq!(saves_of_b(), 0, "nothing was saved over B's queue");

    session.set_config(on_b, vec!["active_source".into()]);
    session.persist_now().await;
    api.set_queue(replace(&["b-new"])).await.expect("set queue");
    wait_committed(&api).await;
    session.persist_now().await;
    assert!(saves_of_b() > 0, "a queue built on B is saved again");
}

/// An equalizer preview reaches the engine without becoming the session's settings.
#[tokio::test]
async fn an_equalizer_preview_leaves_the_settings_alone() {
    let harness = harness(|_| {});
    let before = harness.api.session.config_watch().borrow().clone();
    let mut preview = before.equalizer.clone();
    preview.enabled = !preview.enabled;
    harness.api.session.preview_equalizer(preview);
    harness.api.session.persist_now().await;
    assert_eq!(*harness.api.session.config_watch().borrow(), before);
}

#[tokio::test]
async fn detaching_an_integration_returns_the_session_to_the_engine() {
    let harness = harness(|_| {});
    let stub = Arc::new(StubExternal::default());
    harness.api.session.attach_external(stub.clone());
    harness.api.session.report_external(crate::ExternalReport {
        track: Some(external_track("remote")),
        playing: true,
        ..Default::default()
    });
    wait_state(&harness.api, "external active", |state| {
        state.external.is_some()
    })
    .await;

    harness.api.session.detach_external();
    let state = wait_state(&harness.api, "external cleared", |state| {
        state.external.is_none()
    })
    .await;
    assert_eq!(state.phase, ApiPhase::Idle);
    assert!(state.track.is_none());

    harness
        .api
        .set_queue(replace(&["a"]))
        .await
        .expect("queue replaced");
    let state = wait_state(&harness.api, "engine plays again", |state| {
        state.track.as_ref().is_some_and(|t| t.title == "a")
    })
    .await;
    assert!(state.external.is_none());
    assert!(
        stub.calls().iter().all(|call| call != "next"),
        "the detached integration hears nothing further"
    );
}

/// The whole point of a sink: a track the engine cannot decode is handed to
/// the integration, and the next one that it can comes back to the engine.
/// Both directions go through the load path, so a jump switches sides too.
#[tokio::test]
async fn playback_moves_between_the_engine_and_an_integration() {
    let harness = harness(|_| {});
    let stub = Arc::new(StubExternal::default());
    harness.api.session.set_external_sink(stub.clone());

    harness
        .api
        .set_queue(replace(&["spotify:remote", "local.wav"]))
        .await
        .expect("queue replaced");

    let state = wait_state(&harness.api, "the integration took the track", |state| {
        state.external.is_some()
    })
    .await;
    assert_eq!(
        state.external.as_ref().map(|it| it.kind.as_str()),
        Some("stub")
    );
    assert!(
        stub.calls()
            .iter()
            .any(|call| call == "load:spotify:remote"),
        "the integration was asked to play the track, got {:?}",
        stub.calls()
    );

    // Ending the remote track advances kopuz's queue, which lands on a track
    // the engine plays -- so the integration is told to stop.
    harness.api.session.report_external(crate::ExternalReport {
        track: Some(external_track("spotify:remote")),
        playing: true,
        completed: true,
        ..Default::default()
    });
    wait_state(&harness.api, "the engine took it back", |state| {
        state.external.is_none() && state.track.as_ref().is_some_and(|t| t.title == "local.wav")
    })
    .await;
    assert!(
        stub.calls().iter().any(|call| call == "stop"),
        "the integration was told to give up playback, got {:?}",
        stub.calls()
    );
}

async fn play_order(api: &LocalApi) -> Vec<String> {
    api.queue_window(Page::default())
        .await
        .expect("window")
        .items
        .into_iter()
        .map(|item| item.track.title)
        .collect()
}

async fn shuffled_harness() -> Harness {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&[
            "track-0", "track-1", "track-2", "track-3", "track-4", "track-5", "track-6", "track-7",
        ]))
        .await
        .expect("set queue");
    harness
        .api
        .player_command(PlayerCommand::SetMode {
            shuffle: Some(true),
            loop_mode: None,
        })
        .await
        .expect("shuffle on");
    harness
}

#[tokio::test]
async fn a_queue_row_plays_without_reshuffling_the_rest() {
    let harness = shuffled_harness().await;
    let before = play_order(&harness.api).await;

    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 5 })
        .await
        .expect("jump");

    assert_eq!(play_order(&harness.api).await, before);
}

#[tokio::test]
async fn a_track_list_row_repins_the_shuffle_around_itself() {
    let harness = shuffled_harness().await;

    harness
        .api
        .queue_edit(QueueEdit::JumpPhysical { index: 6 })
        .await
        .expect("jump");

    assert_eq!(play_order(&harness.api).await[0], "track-6");
}

/// A local playlist has no remote listing, so refreshing one must not reach
/// the epoch sweep that drops whatever the remote no longer lists -- for
/// Local that is every entry it has. Reachable from any client: the gRPC
/// surface exposes the call with no capability guard of its own.
#[tokio::test]
async fn refreshing_a_local_playlist_keeps_its_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("playlists.db"))
        .await
        .expect("db");
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices::default(),
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    let playlists = crate::PlaylistService::new(database, session);
    let keys = vec!["song-a".to_string(), "song-b".to_string()];
    let id = playlists.create("mix", &keys).await.expect("create");

    playlists.refresh(&id).await.expect("refresh");

    let catalog = playlists.catalog().await.expect("catalog");
    let playlist = catalog
        .playlists
        .iter()
        .find(|playlist| playlist.id == id)
        .expect("the playlist survives its own refresh");
    assert_eq!(playlist.track_keys, keys);
}

/// The core debounces a volume change by 750 ms so that a slider drag is one
/// database write. Closing inside that window leaves the newest value held
/// only by the engine, and the config service is what a whole-config write
/// reads volume back from, so the shutdown flush is what has to carry it.
#[tokio::test]
async fn the_shutdown_flush_persists_the_volume_the_debounce_still_holds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("volume.db")).await.expect("db");
    let service = crate::ConfigService::new(
        database.clone(),
        dir.path().join("settings.toml"),
        config::AppConfig {
            volume: 0.8,
            ..Default::default()
        },
    );
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices::default(),
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    service
        .mutate_state(&[], |_| {})
        .await
        .expect("seed the database");
    LocalApi::new(session.clone())
        .player_command(api::PlayerCommand::SetVolume { volume: 0.2 })
        .await
        .expect("set volume");

    session.persist_now().await;
    assert_eq!(
        stored_volume(&database).await,
        0.8,
        "the queue flush alone leaves the debounced volume behind"
    );

    crate::boot::flush_volume(&session, &service).await;

    assert_eq!(
        stored_volume(&database).await,
        0.2,
        "the engine's volume reaches the database"
    );
}

/// Concurrent writers each save then hand the session a config; the session must end on the one saved last, and every save moves the revision.
#[tokio::test]
async fn config_changes_reach_the_session_in_the_order_they_were_saved() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("order.db")).await.expect("db");
    let service = Arc::new(crate::ConfigService::new(
        database,
        dir.path().join("settings.toml"),
        config::AppConfig::default(),
    ));
    let player = Player::try_with_sink(Box::new(FakeSink(FakeSinkHandle::default())))
        .expect("headless player starts");
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices::default(),
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    service.attach_session(session.clone());
    let first = service.view().await.expect("view").revision;

    let writers: Vec<_> = (1..=20u8)
        .map(|crossfade| {
            let service = service.clone();
            tokio::spawn(async move {
                service
                    .mutate_state(&["crossfade_seconds"], move |config| {
                        config.crossfade_seconds = crossfade;
                    })
                    .await
            })
        })
        .collect();
    for writer in writers {
        writer.await.expect("writer").expect("saved");
    }
    session.persist_now().await;

    let saved = service.snapshot().await.crossfade_seconds;
    assert_eq!(
        session.config_watch().borrow().crossfade_seconds,
        saved,
        "the session holds what was saved last"
    );
    assert_eq!(service.view().await.expect("view").revision, first + 20);
}

async fn stored_volume(database: &db::Db) -> f32 {
    database
        .load_config()
        .await
        .expect("load")
        .expect("stored config")
        .volume
}

/// A source with track radio and nothing else, for the autoplay gate.
struct RadioSource {
    db: db::Db,
    source: config::Source,
}

#[async_trait::async_trait]
impl server::source::MediaSource for RadioSource {
    fn source(&self) -> &config::Source {
        &self.source
    }
    fn db(&self) -> &db::Db {
        &self.db
    }
    fn capabilities(&self) -> server::source::Capabilities {
        use server::source::{AlbumType, ArtistView, FavoritesSync, PlaylistOps, RadioSeeds};
        server::source::Capabilities {
            edit_tags: false,
            delete_from_disk: false,
            scan_folders: false,
            folders: false,
            browse_folders: false,
            external_devices: false,
            browser_playback: false,
            sync: false,
            downloads: false,
            discover: false,
            dont_recommend: false,
            radio: RadioSeeds {
                track: true,
                ..RadioSeeds::NONE
            },
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
            account_avatar: false,
            stream_quality: false,
            explicit_flags: true,
            watch_history: false,
        }
    }
    async fn add_to_playlist(
        &self,
        _: &str,
        _: &[String],
    ) -> Result<Vec<String>, server::source::SourceError> {
        Err(server::source::SourceError::unsupported("playlists"))
    }
    async fn create_playlist(
        &self,
        _: &str,
        _: &[String],
    ) -> Result<String, server::source::SourceError> {
        Err(server::source::SourceError::unsupported("playlists"))
    }
    async fn remove_from_playlist(
        &self,
        _: &str,
        _: &reader::Track,
        _: usize,
    ) -> Result<(), server::source::SourceError> {
        Err(server::source::SourceError::unsupported("playlists"))
    }
    async fn resolve_stream(
        &self,
        _: &str,
        _: config::StreamQuality,
    ) -> Result<server::source::StreamInfo, server::source::SourceError> {
        Err(server::source::SourceError::unsupported("streams"))
    }
    async fn validate(&self) -> server::source::AuthOutcome {
        server::source::AuthOutcome::Valid
    }
    async fn fetch_favorites(&self) -> Result<Vec<String>, server::source::SourceError> {
        Ok(Vec::new())
    }
    async fn push_favorite(&self, _: &str, _: bool) -> Result<(), server::source::SourceError> {
        Ok(())
    }
}

struct PlaybackSettingsHarness {
    harness: Harness,
    session: SessionHandle,
    _dir: tempfile::TempDir,
}

/// A session over a source that has track radio, with `configure` applied.
async fn radio_harness(configure: impl FnOnce(&mut config::AppConfig)) -> PlaybackSettingsHarness {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("radio.db")).await.expect("db");
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut services = PlaybackServices::default();
    services.config.crossfade_seconds = 0;
    configure(&mut services.config);
    services.active_source = Some(Arc::new(RadioSource {
        db: database,
        source: config::Source::default(),
    }));
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        services,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    PlaybackSettingsHarness {
        harness: Harness {
            api: LocalApi::new(session.clone()),
            sink,
        },
        session,
        _dir: dir,
    }
}

#[tokio::test]
async fn skipping_explicit_tracks_leaves_them_out_of_a_new_queue() {
    let harness = harness(|config| config.skip_explicit = true);
    let mut request = replace(&["track-0", "explicit-1", "track-2"]);
    request.start_index = Some(1);
    harness.api.set_queue(request).await.expect("set queue");
    assert_eq!(queue_titles(&harness.api).await, ["track-0", "track-2"]);
    let state = wait_committed(&harness.api).await;
    assert_eq!(
        state.track.map(|track| track.title),
        Some("track-2".to_string()),
        "a start on an explicit track moves to the next clean one"
    );

    harness
        .api
        .set_queue(enqueue(QueueMode::Append, &["explicit-3", "track-4"]))
        .await
        .expect("append");
    assert_eq!(
        queue_titles(&harness.api).await,
        ["track-0", "track-2", "track-4"]
    );

    let refused = harness
        .api
        .set_queue(replace(&["explicit-5"]))
        .await
        .expect_err("a queue of only explicit tracks");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert_eq!(
        queue_titles(&harness.api).await.len(),
        3,
        "the queue is untouched"
    );
}

#[tokio::test]
async fn skipping_explicit_tracks_steps_over_ones_already_queued() {
    let PlaybackSettingsHarness {
        harness, session, ..
    } = radio_harness(|_| {}).await;
    harness
        .api
        .set_queue(replace(&["short-0", "short-explicit-1", "short-2"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    let mut config = session.config_watch().borrow().clone();
    config.skip_explicit = true;
    session.set_config(config, vec!["skip_explicit".into()]);

    let state = drive_until(&harness, "the track after the explicit one", |state| {
        state.queue.index != Some(0) && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    assert_eq!(state.queue.index, Some(2));
    assert_eq!(
        state.track.map(|track| track.title),
        Some("short-2".to_string())
    );
    assert_eq!(
        queue_titles(&harness.api).await.len(),
        3,
        "the queue keeps it"
    );
}

#[tokio::test]
async fn a_run_out_queue_goes_on_as_a_radio_when_autoplay_is_on() {
    let PlaybackSettingsHarness { harness, .. } = radio_harness(|config| {
        config.autoplay_radio = true;
        config.skip_explicit = true;
    })
    .await;
    harness
        .api
        .set_queue(replace(&["short-seed"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    let state = drive_until(&harness, "the radio after the seed", |state| {
        state.queue.index == Some(1) && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    assert_eq!(
        state.track.map(|track| track.title),
        Some("radio-short-1".to_string())
    );
    assert_eq!(
        queue_titles(&harness.api).await,
        ["short-seed", "radio-short-1", "radio-short-3"],
        "the seed is not queued twice and the explicit pick is skipped"
    );
}

#[tokio::test]
async fn a_run_out_queue_stops_without_autoplay_or_without_track_radio() {
    for (autoplay, with_radio) in [(false, true), (true, false)] {
        let (harness, _keep) = if with_radio {
            let PlaybackSettingsHarness {
                harness,
                session,
                _dir,
            } = radio_harness(|config| config.autoplay_radio = autoplay).await;
            (harness, Some((session, _dir)))
        } else {
            (harness(|config| config.autoplay_radio = autoplay), None)
        };
        harness
            .api
            .set_queue(replace(&["short-seed"]))
            .await
            .expect("set queue");
        wait_committed(&harness.api).await;
        drive_until(&harness, "end-of-queue stop", |state| {
            state.intent == Intent::Stopped && state.phase == ApiPhase::Ended
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            queue_titles(&harness.api).await,
            ["short-seed"],
            "autoplay {autoplay}, track radio {with_radio}"
        );
    }
}
