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
            // A source answers with the seed among its mix.
            QueueContext::TrackRadio { key } => Ok([
                key.as_str(),
                "radio-short-1",
                "radio-short-explicit-2",
                "radio-short-3",
            ]
            .iter()
            .map(|key| test_track(&key.to_string()))
            .collect()),
            container => {
                let name = match container {
                    QueueContext::Album { id }
                    | QueueContext::Playlist { id }
                    | QueueContext::PlaylistRadio { id } => id.clone(),
                    QueueContext::Artist { artist } => artist.clone(),
                    QueueContext::Genre { name } => name.clone(),
                    QueueContext::Radio { station_id, .. } => station_id.clone(),
                    QueueContext::TrackRadio { key } => key.clone(),
                    QueueContext::Filter { .. } | QueueContext::Tracks { .. } => "filter".into(),
                };
                // A container named "empty" has nothing in it.
                if name == "empty" {
                    return Ok(Vec::new());
                }
                Ok([format!("{name}-0"), format!("{name}-1")]
                    .iter()
                    .map(test_track)
                    .collect())
            }
        }
    }
}

fn test_track(key: &String) -> Track {
    let duration = if key.starts_with("radio:") {
        u64::MAX
    } else if key.contains("short") {
        1
    } else if key.contains("long") {
        60
    } else {
        6
    };
    Track {
        id: if key.starts_with("unavailable-server:") {
            reader::models::TrackId::Server {
                service: config::MusicService::Jellyfin,
                item_id: key.clone(),
            }
        } else if key.starts_with("ytm-") {
            reader::models::TrackId::Server {
                service: config::MusicService::YtMusic,
                item_id: key.clone(),
            }
        } else if let Some(item_id) = key.strip_prefix("spotify:") {
            reader::models::TrackId::Server {
                service: config::MusicService::Spotify,
                item_id: item_id.to_string(),
            }
        } else if let Some(item_id) = key.strip_prefix("smb:") {
            reader::models::TrackId::Server {
                service: config::MusicService::Smb,
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
        counterpart: key
            .starts_with("ytm-song:")
            .then(|| Box::new(music_video_of(key))),
    }
}

/// A video whose first second is an intro the song does not have.
fn music_video_of(key: &str) -> reader::Counterpart {
    reader::Counterpart {
        item_id: format!("{key}-video"),
        video: true,
        duration_ms: Some(7_000),
        segments: vec![reader::SharedSegment {
            start_ms: 0,
            counterpart_start_ms: 1_000,
            duration_ms: 6_000,
        }],
    }
}

#[test]
fn smb_now_playing_artwork_resolves_cached_files_and_missing_covers() {
    let mut track = test_track(&"smb:Album/song.flac".to_string());
    track.cover = Some("/cache/album cover.jpg".into());
    assert_eq!(
        load::now_playing_artwork(None, &track).as_deref(),
        Some("/cache/album cover.jpg")
    );
    track.cover = Some(reader::CoverRef::NO_COVER.into());
    assert_eq!(load::now_playing_artwork(None, &track), None);
    track.cover = Some("https://example.com/cover.jpg".into());
    assert_eq!(
        load::now_playing_artwork(None, &track).as_deref(),
        Some("https://example.com/cover.jpg")
    );
}

#[tokio::test]
async fn album_artwork_arriving_after_playback_refreshes_the_queue() {
    let source = config::Source::Server("nas".into());
    let harness = harness(|_| {});
    harness.api.session.set_sources(active_on(source.clone()));
    harness
        .api
        .set_queue(replace(&["smb:01.flac", "smb:02.flac"]))
        .await
        .unwrap();
    wait_committed(&harness.api).await;
    harness
        .api
        .player_command(PlayerCommand::Pause)
        .await
        .unwrap();
    let album = reader::Album {
        id: String::new(),
        title: "Album".into(),
        artist: String::new(),
        genre: String::new(),
        year: 0,
        cover_path: Some(PathBuf::from("/cache/album.jpg")),
        manual_cover: false,
        artist_id: None,
        artist_key: None,
    };
    harness.api.session.update_album_covers(
        &config::Source::Server("other".into()),
        std::slice::from_ref(&album),
    );
    let unchanged = harness.api.queue_snapshot().await.unwrap();
    assert!(unchanged.items.iter().all(|track| track.artwork.is_none()));
    harness.api.session.update_album_covers(&source, &[album]);
    let state = wait_state(&harness.api, "background cover", |state| {
        state
            .track
            .as_ref()
            .is_some_and(|track| track.artwork.is_some())
    })
    .await;
    assert_eq!(state.phase, ApiPhase::Paused);
    assert_eq!(state.queue.index, Some(0));
    let queue = harness.api.queue_snapshot().await.unwrap();
    assert!(queue.items.iter().all(|track| track.artwork.is_some()));
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
        Err(std::io::Error::other("synthetic source failure"))
    })
}

struct Harness {
    api: LocalApi,
    sink: FakeSinkHandle,
    engine_events: mpsc::UnboundedReceiver<EngineEvent>,
}

fn harness(configure: impl FnOnce(&mut config::Settings)) -> Harness {
    harness_with_provider(
        configure,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    )
}

fn harness_with_provider(
    configure: impl FnOnce(&mut config::Settings),
    provider: FactoryOverride,
) -> Harness {
    harness_with_services(configure, provider, None)
}

fn harness_with_services(
    configure: impl FnOnce(&mut config::Settings),
    provider: FactoryOverride,
    recorder: Option<Arc<dyn PlaybackRecorder>>,
) -> Harness {
    let mut services = PlaybackServices {
        recorder,
        ..Default::default()
    };
    services.settings.playback.crossfade_seconds = 0;
    configure(&mut services.settings);
    spawn_harness(services, provider)
}

fn harness_at_volume(volume: f32, muted: bool) -> Harness {
    let mut services = PlaybackServices {
        volume,
        muted,
        ..Default::default()
    };
    services.settings.playback.crossfade_seconds = 0;
    spawn_harness(
        services,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    )
}

fn spawn_harness(services: PlaybackServices, provider: FactoryOverride) -> Harness {
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let engine_events = player.subscribe();
    let session =
        SessionHandle::spawn_with_factory(Arc::new(StubLibrary), player, services, provider);
    Harness {
        api: LocalApi::new(session),
        sink,
        engine_events,
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
    let engine_events = player.subscribe();
    let mut services = PlaybackServices::default();
    services.settings.playback.crossfade_seconds = 0;
    services.settings.playback.replay_gain.mode = config::ReplayGainMode::Auto;
    let session =
        SessionHandle::spawn_with_factory(Arc::new(GainLibrary), player, services, provider);
    Harness {
        api: LocalApi::new(session),
        sink,
        engine_events,
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
async fn muting_silences_output_and_unmuting_restores_the_level() {
    let harness = harness(|_| {});
    harness.api.set_queue(replace(&["a"])).await.unwrap();
    wait_committed(&harness.api).await;
    wait_replay_gain(&harness, 0.0).await;

    harness
        .api
        .player_command(PlayerCommand::SetMuted { muted: true })
        .await
        .unwrap();
    let state = harness.api.player_state().await.unwrap();
    assert!(state.muted);
    assert_eq!(state.volume, 1.0, "muting leaves the level alone");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let peak = harness
            .sink
            .pull(1024)
            .into_iter()
            .fold(0.0_f32, |p, s| p.max(s.abs()));
        if peak < 1e-4 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "still audible: {peak}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    harness
        .api
        .player_command(PlayerCommand::SetMuted { muted: false })
        .await
        .unwrap();
    let state = harness.api.player_state().await.unwrap();
    assert!(!state.muted);
    wait_replay_gain(&harness, 0.0).await;

    harness
        .api
        .player_command(PlayerCommand::SetMuted { muted: true })
        .await
        .unwrap();
    harness
        .api
        .player_command(PlayerCommand::SetVolume { volume: 0.4 })
        .await
        .unwrap();
    let state = harness.api.player_state().await.unwrap();
    assert!(!state.muted, "a new level unmutes");
    assert_eq!(state.volume, 0.4);
}

#[tokio::test]
async fn replay_gain_settings_reach_playing_audio_through_config_api() {
    let mut harness = harness(|_| {});
    let dir = tempfile::tempdir().unwrap();
    let database = db::init(&dir.path().join("test.db")).await.unwrap();
    let config = harness.api.session.settings_rx.borrow().clone();
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
        let mut next = harness
            .api
            .config()
            .await
            .unwrap()
            .settings
            .playback
            .replay_gain;
        next.mode = mode;
        next.preamp_db = preamp;
        let report = harness
            .api
            .edit_config(vec![config::SettingsEdit::Playback(
                config::PlaybackEdit::ReplayGain(next),
            )])
            .await
            .unwrap();
        assert_eq!(report.outcomes, vec![api::EditOutcome::Applied]);
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
        Some(if track.title == "long-slow" {
            gated_factory(6, provider_gate.clone())
        } else {
            wav_factory(6)
        })
    });
    let harness = harness_with_provider(|_| {}, provider);
    harness
        .api
        .set_queue(replace(&["long-fast", "long-slow"]))
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
    let harness = harness_with_provider(|config| config.playback.crossfade_seconds = 1, provider);
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
    let harness = harness(|config| config.playback.crossfade_seconds = 1);
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
    let harness = harness(|config| config.playback.crossfade_seconds = 1);
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
    let harness = harness(|config| config.playback.crossfade_seconds = 1);
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
    let harness = harness(|config| config.playback.crossfade_seconds = 1);
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
    let harness = harness(|config| config.playback.crossfade_seconds = 1);
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
async fn a_running_crossfade_publishes_a_live_anchor_for_the_outgoing_track() {
    let harness = harness(|settings| settings.playback.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    let state = drive_until(&harness, "running crossfade", |state| {
        state.fading.is_some() && matches!(state.intent, Intent::Committed { token: 2 })
    })
    .await;

    let anchor = state.position.expect("anchor during the fade");
    assert!(anchor.playing, "a client keeps counting through the fade");
    assert!(
        anchor.ms >= 4_500,
        "the anchor is the outgoing track's position near its end, not the incoming one's start: {anchor:?}"
    );
}

#[tokio::test]
async fn a_queued_gapless_track_leaves_the_outgoing_anchor_running() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    let queued = drive_until(&harness, "gapless queued", |state| {
        state.queue.index == Some(0) && matches!(state.intent, Intent::Committed { token: 2 })
    })
    .await;
    assert!(queued.fading.is_none());
    assert_eq!(
        queued.track.as_ref().map(|track| track.title.as_str()),
        Some("track-0")
    );
    assert!(
        queued.position.expect("anchor while queued").playing,
        "a client keeps counting the outgoing track"
    );

    let switched = drive_until(&harness, "gapless switch", |state| {
        state.queue.index == Some(1)
    })
    .await;
    assert!(switched.position.expect("anchor after the switch").playing);
}

#[tokio::test]
async fn seek_during_crossfade_is_guarded_to_the_visible_token() {
    let harness = harness(|config| config.playback.crossfade_seconds = 1);
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

/// A track's bytes as a stream that has delivered only so many of them yet; reading further blocks until more arrive.
struct Trickle {
    bytes: Cursor<Vec<u8>>,
    arrived: Arc<(Mutex<u64>, Condvar)>,
}

impl std::io::Read for Trickle {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let at = self.bytes.position();
        let len = self.bytes.get_ref().len() as u64;
        // The probe looks for tags at the end, which a server answers with a range request.
        if at >= len.saturating_sub(TRICKLE_TAIL) {
            return self.bytes.read(buf);
        }
        let (lock, more) = &*self.arrived;
        let mut arrived = lock.lock().expect("trickle lock");
        while *arrived <= at {
            arrived = more.wait(arrived).expect("trickle wait");
        }
        let room = usize::try_from(*arrived - at).unwrap_or(usize::MAX);
        drop(arrived);
        let len = buf.len().min(room);
        self.bytes.read(&mut buf[..len])
    }
}

impl std::io::Seek for Trickle {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.bytes.seek(to)
    }
}

/// One second of audio and the header have arrived.
const FIRST_SECOND: u64 = 44 + 44_100 * 4;
const TRICKLE_TAIL: u64 = 64 * 1024;

fn trickle_harness() -> (Harness, Arc<(Mutex<u64>, Condvar)>) {
    let arrived = Arc::new((Mutex::new(FIRST_SECOND), Condvar::new()));
    let provider = arrived.clone();
    let harness = harness_with_provider(
        |_| {},
        Arc::new(move |track| {
            let stream = Trickle {
                bytes: Cursor::new(wav_bytes(track.duration.min(6))),
                arrived: provider.clone(),
            };
            Some(Box::new(move || Ok(player::decoder::from_stream(stream))) as SourceFactory)
        }),
    );
    (harness, arrived)
}

fn arrive_all(arrived: &(Mutex<u64>, Condvar)) {
    *arrived.0.lock().expect("trickle lock") = u64::MAX;
    arrived.1.notify_all();
}

/// A rebuffering stream stops the audio, so a client counting on would run ahead of it.
#[tokio::test]
async fn a_stream_that_runs_dry_holds_the_anchor_until_audio_flows_again() {
    let (harness, arrived) = trickle_harness();
    harness
        .api
        .set_queue(replace(&["track-0"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    let held = drive_until(&harness, "the stream runs dry", |state| {
        state
            .position
            .is_some_and(|anchor| !anchor.playing && anchor.ms >= 500)
    })
    .await;
    assert_eq!(held.phase, ApiPhase::Playing, "a stall is not a pause");
    let stopped_at = held.position.expect("anchor").ms;
    assert!(stopped_at <= 1_000, "{stopped_at}");

    arrive_all(&arrived);
    let running = drive_until(&harness, "the audio flows again", |state| {
        state.position.is_some_and(|anchor| anchor.playing)
    })
    .await;
    assert!(running.position.expect("anchor").ms >= stopped_at);
}

/// A seek past what a stream has delivered waits on it, so its anchor holds at the target.
#[tokio::test]
async fn a_seek_into_what_has_not_arrived_holds_the_anchor_at_the_target() {
    let (harness, arrived) = trickle_harness();
    harness
        .api
        .set_queue(replace(&["track-0"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    drive_until(&harness, "the audio flows", |state| {
        state.position.is_some_and(|anchor| anchor.playing)
    })
    .await;

    harness
        .api
        .player_command(PlayerCommand::Seek { position_ms: 4_000 })
        .await
        .expect("seek");
    let held = drive_until(&harness, "the seek waits on the stream", |state| {
        state
            .position
            .is_some_and(|anchor| !anchor.playing && anchor.ms == 4_000)
    })
    .await;
    assert_eq!(held.phase, ApiPhase::Playing);

    arrive_all(&arrived);
    let running = drive_until(&harness, "the audio passes the target", |state| {
        state.position.is_some_and(|anchor| anchor.playing)
    })
    .await;
    assert!(running.position.expect("anchor").ms >= 4_000);
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
    let now = track_info(&track, &crate::offline::OfflineIndex::default());
    assert_eq!(now.kind, TrackKind::Radio);
    assert_eq!(now.duration_ms, None);
    assert!(!now.seekable);
}

#[test]
fn an_unknown_duration_goes_out_as_none() {
    let track = Track {
        duration: 0,
        ..test_track(&"a-0".to_string())
    };
    let now = track_info(&track, &crate::offline::OfflineIndex::default());
    assert_eq!(now.duration_ms, None);
    assert_eq!(now.duration_secs(), None);
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
    let now = track_info(&server, &crate::offline::OfflineIndex::default());
    assert_eq!(now.key, "abc123", "key is the bare library ref");
    assert_eq!(now.uid, "ytmusic:abc123", "uid is source-qualified");
    assert_ne!(now.key, now.uid, "the two must not be conflated");

    let local = test_track(&"/music/a.flac".to_string());
    let now = track_info(&local, &crate::offline::OfflineIndex::default());
    assert_eq!(now.key, now.uid);
}

#[derive(Default)]
struct MemoryStore {
    saved: Mutex<Vec<(config::Source, db::QueueSnapshot)>>,
    synced: Mutex<HashMap<String, String>>,
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

    async fn synced_remote(&self, source: &config::Source) -> Option<String> {
        self.synced
            .lock()
            .expect("store lock")
            .get(source.as_str())
            .cloned()
    }

    async fn set_synced_remote(&self, source: &config::Source, fingerprint: &str) {
        self.synced
            .lock()
            .expect("store lock")
            .insert(source.as_str().to_string(), fingerprint.to_string());
    }
}

struct MemoryRecorder {
    recents: Mutex<Vec<String>>,
    listens: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl PlaybackRecorder for MemoryRecorder {
    async fn record_recent(&self, _: Option<&server::source::ActiveSource>, track: &Track) {
        self.recents
            .lock()
            .expect("recorder lock")
            .push(track.title.clone());
    }

    async fn bump_listen_count(&self, _: Option<&server::source::ActiveSource>, track: &Track) {
        self.listens
            .lock()
            .expect("recorder lock")
            .push(track.title.clone());
    }
}

#[tokio::test]
async fn recents_record_once_per_committed_track() {
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

/// A track counts once half of it has played, before it ends: the fake sink
/// only advances when pulled, so the one-second track stays playing.
#[tokio::test]
async fn a_track_counts_as_listened_once_half_of_it_has_played() {
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
    tokio::time::sleep(Duration::from_millis(1_600)).await;

    let listens = recorder.listens.lock().expect("lock").clone();
    assert_eq!(listens, vec!["short-a".to_string()]);
}

/// Skipping before the threshold counts nothing, and the skipped track's
/// watcher stops rather than counting later; a paused track gains no time.
#[tokio::test]
async fn a_skip_before_half_the_track_counts_no_listen() {
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
        .set_queue(replace(&["a-1", "a-2"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    harness
        .api
        .player_command(PlayerCommand::Next)
        .await
        .expect("next");
    wait_state(&harness.api, "second track committed", |state| {
        state.queue.index == Some(1) && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    harness
        .api
        .player_command(PlayerCommand::Pause)
        .await
        .expect("pause");
    // Past the six-second tracks' three-second threshold.
    tokio::time::sleep(Duration::from_millis(3_600)).await;

    assert!(recorder.listens.lock().expect("lock").is_empty());
}

/// Notes which source each listen was credited to.
#[derive(Default)]
struct CreditRecorder {
    listens: Mutex<Vec<Option<String>>>,
}

#[async_trait::async_trait]
impl PlaybackRecorder for CreditRecorder {
    async fn record_recent(&self, _: Option<&server::source::ActiveSource>, _: &Track) {}

    async fn bump_listen_count(&self, source: Option<&server::source::ActiveSource>, _: &Track) {
        self.listens
            .lock()
            .expect("recorder lock")
            .push(source.map(|source| source.source().as_str().to_string()));
    }
}

/// A listen after a source switch belongs to the source switched to, not the one the session started with.
#[tokio::test]
async fn a_listen_is_credited_to_the_source_active_when_it_played() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("credit.db")).await.expect("db");
    let source_of = |active: config::Source| -> server::source::ActiveSource {
        let sources = server::sources::Sources {
            active,
            ..Default::default()
        };
        Arc::from(server::source::active(database.clone(), &sources))
    };
    let recorder = Arc::new(CreditRecorder::default());
    let harness = harness_with_services(
        |_| {},
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
        Some(recorder.clone()),
    );
    let session = harness.api.session.clone();
    session.set_active_source(Some(source_of(config::Source::default())));
    session.set_active_source(Some(source_of(config::Source::LocalLibrary(
        "local:b".into(),
    ))));

    harness
        .api
        .set_queue(replace(&["short-a"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    // The fake sink only advances when pulled, so the track stays playing in
    // real time and crosses its one-second threshold.
    tokio::time::sleep(Duration::from_millis(1_600)).await;

    assert_eq!(
        recorder.listens.lock().expect("lock").as_slice(),
        [Some("local:b".to_string())]
    );
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
    async fn record_recent(&self, _: Option<&server::source::ActiveSource>, _: &Track) {
        self.write().await;
    }

    async fn bump_listen_count(&self, _: Option<&server::source::ActiveSource>, _: &Track) {
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
    // The first track plays past its one-second threshold, so it counts.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
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
async fn a_track_whose_media_cannot_be_reached_says_so() {
    for (kind, code) in [
        (std::io::ErrorKind::NotFound, api::ErrorCode::NotFound),
        (
            std::io::ErrorKind::NotConnected,
            api::ErrorCode::SourceUnreachable,
        ),
    ] {
        let provider: FactoryOverride = Arc::new(move |_| {
            Some(Box::new(move || Err(std::io::Error::new(kind, "shut"))) as SourceFactory)
        });
        let harness = harness_with_provider(|_| {}, provider);
        harness
            .api
            .set_queue(replace(&["track-0"]))
            .await
            .expect("set queue");
        let state = wait_state(&harness.api, "load failed", |state| state.error.is_some()).await;
        assert_eq!(state.error.map(|error| error.code), Some(code), "{kind:?}");
    }
}

/// A failed crossfade and its retry count the outgoing track once, by playtime.
#[tokio::test]
async fn failed_crossfade_preparation_can_retry_and_counts_the_outgoing_track_once() {
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
        |config| config.playback.crossfade_seconds = 1,
        provider,
        Some(recorder.clone()),
    );

    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    // Past the six-second track's three-second threshold before driving on;
    // the fake sink only advances when pulled, so this is playing time.
    tokio::time::sleep(Duration::from_millis(3_400)).await;
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
    assert_eq!(
        recorder.listens.lock().expect("lock").as_slice(),
        ["track-0"]
    );

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
        radio_cursor: None,
        origin: None,
        stations: Default::default(),
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
    let store = Arc::new(MemoryStore::default());
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

fn active_on(source: config::Source) -> Arc<server::sources::Sources> {
    Arc::new(server::sources::Sources {
        active: source,
        ..Default::default()
    })
}

/// Each source keeps its queue: leaving one parks it, coming back resumes it, and a source never visited starts empty.
#[tokio::test]
async fn switching_sources_parks_and_resumes_each_queue() {
    let store = Arc::new(MemoryStore::default());
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut config = config::Settings::default();
    config.library.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    let on_a = active_on(config::Source::default());
    let on_b = active_on(config::Source::LocalLibrary("local:b".into()));
    let services = PlaybackServices {
        settings: config,
        sources: on_a.clone(),
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

    session.set_sources(on_b.clone());
    session.persist_now().await;
    assert!(queue_titles(&api).await.is_empty(), "B was never visited");
    api.set_queue(replace(&["b-0"])).await.expect("set queue");
    wait_committed(&api).await;

    session.set_sources(on_a.clone());
    session.persist_now().await;
    assert_eq!(
        queue_titles(&api).await,
        on_a_titles,
        "A resumes its own queue"
    );

    session.set_sources(on_b);
    session.persist_now().await;
    assert_eq!(queue_titles(&api).await.len(), 1, "and B its own");
}

/// Entering a source with nothing queued must not keep the position of the one left.
#[tokio::test]
async fn switching_to_an_empty_source_drops_the_position_left_behind() {
    let store = Arc::new(MemoryStore::default());
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut config = config::Settings::default();
    config.library.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    let on_b = config::Source::LocalLibrary("local:b".into());
    let services = PlaybackServices {
        settings: config,
        sources: active_on(config::Source::default()),
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
    session
        .restore_queue(db::QueueSnapshot {
            version: 1,
            queue: vec![test_track(&"a-0".to_string())],
            current_queue_index: 0,
            progress_secs: 4,
            ..Default::default()
        })
        .await
        .expect("restore");
    let paused = api.player_state().await.expect("state");
    assert_eq!(paused.position.map(|anchor| anchor.ms), Some(4000));

    session.set_sources(active_on(on_b.clone()));
    session.persist_now().await;

    let state = api.player_state().await.expect("state");
    assert!(state.track.is_none(), "{:?}", state.track);
    assert_eq!(state.queue.length, 0);
    assert_eq!(state.position, None);
    let saved = store.saved.lock().expect("store lock");
    let (_, last) = saved
        .iter()
        .rev()
        .find(|(at, _)| *at == on_b)
        .expect("B was saved");
    assert_eq!(last.progress_secs, 0);
}

/// A deleted source is not parked: its queue would outlive it.
#[tokio::test]
async fn leaving_a_deleted_source_parks_nothing() {
    let store = Arc::new(MemoryStore::default());
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut on_b = config::Settings::default();
    on_b.library.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    let services = PlaybackServices {
        settings: on_b.clone(),
        sources: active_on(config::Source::LocalLibrary("local:b".into())),
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

    // The order SourceService sends them in: the folder list first, then the active source.
    let mut deleted = on_b.clone();
    deleted.library.remove_local_source("local:b");
    session.set_settings(
        deleted,
        vec![config::SettingKey::Library(
            config::LibraryKey::LocalSources,
        )],
        0,
    );
    session.set_sources(active_on(config::Source::default()));
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
        counterpart: None,
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
        transport_calls(&stub),
        vec!["seek:4200", "pause"],
        "next and previous drive the queue, not the integration"
    );
}

/// An integration has no mute of its own: muting sends volume zero, unmuting the level back.
#[tokio::test]
async fn muting_an_integration_zeroes_its_volume_and_unmuting_restores_it() {
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

    for command in [
        PlayerCommand::SetVolume { volume: 0.4 },
        PlayerCommand::SetMuted { muted: true },
        PlayerCommand::SetMuted { muted: false },
    ] {
        harness
            .api
            .player_command(command)
            .await
            .expect("command accepted");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let calls = stub.calls();
    // The first call is the level the integration was handed as it took over.
    assert_eq!(
        calls[calls.len() - 3..],
        ["volume:0.4", "volume:0", "volume:0.4"]
    );
    let state = harness.api.player_state().await.expect("state");
    assert!(!state.muted);
    assert_eq!(state.volume, 0.4);
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

    let mut elsewhere = config::Settings::default();
    elsewhere
        .library
        .local_sources
        .push(config::SavedLocalSource {
            id: "local:b".into(),
            name: "B".into(),
            directories: Vec::new(),
        });
    harness.api.session.set_settings(
        elsewhere,
        vec![config::SettingKey::Library(
            config::LibraryKey::LocalSources,
        )],
        0,
    );
    harness
        .api
        .session
        .set_sources(active_on(config::Source::LocalLibrary("local:b".into())));
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
        transport_calls(&stub),
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

    async fn synced_remote(&self, source: &config::Source) -> Option<String> {
        self.inner.synced_remote(source).await
    }

    async fn set_synced_remote(&self, source: &config::Source, fingerprint: &str) {
        self.inner.set_synced_remote(source, fingerprint).await;
    }
}

/// A queue that could not be read is not saved over, until a new one is built in its place.
#[tokio::test]
async fn an_unreadable_queue_is_not_saved_over() {
    let b = config::Source::LocalLibrary("local:b".into());
    let store = Arc::new(UnreadableStore {
        inner: MemoryStore::default(),
        unreadable: b.clone(),
    });
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut config = config::Settings::default();
    config.library.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    let on_a = active_on(config::Source::default());
    let on_b = active_on(b.clone());
    let services = PlaybackServices {
        settings: config,
        sources: on_a.clone(),
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

    session.set_sources(on_b.clone());
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
    session.set_sources(on_a);
    session.persist_now().await;
    assert_eq!(saves_of_b(), 0, "nothing was saved over B's queue");

    session.set_sources(on_b);
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
    let before = harness.api.session.settings_watch().borrow().clone();
    let mut preview = before.playback.equalizer.clone();
    preview.enabled = !preview.enabled;
    harness.api.session.preview_equalizer(preview);
    harness.api.session.persist_now().await;
    assert_eq!(*harness.api.session.settings_watch().borrow(), before);
}

#[tokio::test]
async fn a_replay_gain_preview_leaves_the_settings_alone() {
    let harness = harness(|_| {});
    let before = harness.api.session.settings_watch().borrow().clone();
    let mut preview = before.playback.replay_gain;
    preview.preamp_db += 3.0;
    harness.api.session.preview_replay_gain(preview);
    harness.api.session.persist_now().await;
    assert_eq!(*harness.api.session.settings_watch().borrow(), before);
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

/// The Spotify tab's report carries no album id and sometimes no art, so the
/// track kopuz queued keeps its library row while the tab is playing it.
#[tokio::test]
async fn an_integration_playing_the_queued_track_shows_the_library_row() {
    let harness = harness(|_| {});
    let stub = Arc::new(StubExternal::default());
    harness.api.session.set_external_sink(stub.clone());
    harness
        .api
        .set_queue(replace(&["spotify:remote"]))
        .await
        .expect("queue replaced");
    wait_state(&harness.api, "the integration took the track", |state| {
        state.external.is_some()
    })
    .await;

    harness.api.session.report_external(crate::ExternalReport {
        track: Some(external_track("remote")),
        position_ms: 1000,
        playing: true,
        ..Default::default()
    });
    let state = wait_state(&harness.api, "the report applied", |state| {
        state.phase == ApiPhase::Playing && state.position.is_some_and(|p| p.ms == 1000)
    })
    .await;
    assert_eq!(
        state.track.map(|track| track.title),
        Some("spotify:remote".to_string()),
        "the queued row, not the tab's bare copy"
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

#[tokio::test]
async fn the_snapshot_lists_a_shuffled_queue_in_play_order() {
    let harness = shuffled_harness().await;
    let titles = |snapshot: &api::QueueSnapshot| -> Vec<String> {
        snapshot.items.iter().map(|t| t.title.clone()).collect()
    };

    let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
    assert!(snapshot.shuffle);
    assert_eq!(titles(&snapshot), play_order(&harness.api).await);
    assert_eq!(snapshot.shuffle_order.len(), 8);
    let mut sorted = snapshot.shuffle_order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..8).collect::<Vec<u32>>(), "a full permutation");
    for (position, physical) in snapshot.shuffle_order.iter().enumerate() {
        assert_eq!(
            snapshot.items[position].title,
            format!("track-{physical}"),
            "shuffle_order[{position}] names the unshuffled index"
        );
    }
    assert_eq!(snapshot.position, Some(0));

    harness
        .api
        .queue_edit(QueueEdit::JumpPhysical { index: 6 })
        .await
        .expect("jump");
    let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
    let position = snapshot.position.expect("position") as usize;
    assert_eq!(snapshot.items[position].title, "track-6");
    assert_eq!(snapshot.shuffle_order[position], 6);
    assert_eq!(titles(&snapshot), play_order(&harness.api).await);

    harness
        .api
        .player_command(PlayerCommand::SetMode {
            shuffle: Some(false),
            loop_mode: None,
        })
        .await
        .expect("shuffle off");
    let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
    assert!(!snapshot.shuffle && snapshot.shuffle_order.is_empty());
    assert_eq!(
        titles(&snapshot),
        (0..8).map(|i| format!("track-{i}")).collect::<Vec<_>>()
    );
    let position = snapshot.position.expect("position") as usize;
    assert_eq!(
        snapshot.items[position].title, "track-6",
        "still playing it"
    );
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

/// A session spawned the way boot spawns it, from the playback state the database holds.
async fn session_from_database(database: &db::Db) -> SessionHandle {
    let (volume, muted) = database.playback_state().await.expect("playback state");
    let player = Player::try_with_sink(Box::new(FakeSink(FakeSinkHandle::default())))
        .expect("headless player starts");
    SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices {
            volume,
            muted,
            ..Default::default()
        },
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    )
}

/// A close inside the 750 ms debounce leaves the newest volume and mute held only by the engine, so the shutdown flush carries them, the level to return to included, to the next start.
#[tokio::test]
async fn volume_and_mute_survive_a_restart_through_the_shutdown_flush() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("volume.db")).await.expect("db");
    database
        .set_playback_state(0.8, false)
        .await
        .expect("seed the database");

    let session = session_from_database(&database).await;
    let state = session.state();
    assert_eq!((state.volume, state.muted), (0.8, false));
    let local = LocalApi::new(session.clone());
    local
        .player_command(PlayerCommand::SetVolume { volume: 0.2 })
        .await
        .expect("set volume");
    local
        .player_command(PlayerCommand::SetMuted { muted: true })
        .await
        .expect("mute");
    wait_state(&local, "muted at the new level", |state| {
        state.muted && state.volume == 0.2
    })
    .await;

    session.persist_now().await;
    assert_eq!(
        database.playback_state().await.expect("load"),
        (0.8, false),
        "the queue flush alone leaves the debounced playback state behind"
    );

    crate::boot::flush_playback_state(&session, &database).await;
    assert_eq!(database.playback_state().await.expect("load"), (0.2, true));

    let restarted = session_from_database(&database).await;
    let state = restarted.state();
    assert_eq!(
        (state.volume, state.muted),
        (0.2, true),
        "the next session seeds from what was flushed"
    );
}

#[tokio::test]
async fn the_shutdown_flush_persists_mute_without_touching_the_volume() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("mute.db")).await.expect("db");
    database
        .set_playback_state(0.8, false)
        .await
        .expect("seed the database");
    let session = session_from_database(&database).await;
    LocalApi::new(session.clone())
        .player_command(api::PlayerCommand::SetMuted { muted: true })
        .await
        .expect("mute");

    crate::boot::flush_playback_state(&session, &database).await;

    assert_eq!(database.playback_state().await.expect("load"), (0.8, true));
}

/// Concurrent writers each save then hand the session a config; the session must end on the one saved last, and every save moves the revision.
#[tokio::test]
async fn config_changes_reach_the_session_in_the_order_they_were_saved() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("order.db")).await.expect("db");
    let service = Arc::new(crate::ConfigService::new(
        database,
        dir.path().join("settings.toml"),
        config::Settings::default(),
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
                    .mutate_state(
                        &[config::SettingKey::Playback(
                            config::PlaybackKey::CrossfadeSeconds,
                        )],
                        move |config| {
                            config.playback.crossfade_seconds = crossfade;
                        },
                    )
                    .await
            })
        })
        .collect();
    for writer in writers {
        writer.await.expect("writer").expect("saved");
    }
    session.persist_now().await;

    let saved = service.snapshot().await.playback.crossfade_seconds;
    assert_eq!(
        session.settings_watch().borrow().playback.crossfade_seconds,
        saved,
        "the session holds what was saved last"
    );
    assert_eq!(service.view().await.expect("view").revision, first + 20);
}

/// Like `drive_until`, keeping every sample, plus `tail` more pulls.
async fn record_until(
    harness: &Harness,
    description: &str,
    tail: usize,
    predicate: impl Fn(&PlayerState) -> bool,
) -> (PlayerState, Vec<f32>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut samples = Vec::new();
    loop {
        samples.extend(harness.sink.pull(2048));
        let state = harness.api.player_state().await.expect("player state");
        if predicate(&state) {
            for _ in 0..tail {
                samples.extend(harness.sink.pull(2048));
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            return (state, samples);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out driving audio until {description}: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn longest_inner_silence(samples: &[f32]) -> usize {
    let audible = |sample: &f32| *sample != 0.0;
    let (Some(first), Some(last)) = (
        samples.iter().position(audible),
        samples.iter().rposition(audible),
    ) else {
        return samples.len();
    };
    samples[first..=last]
        .split(audible)
        .map(<[f32]>::len)
        .max()
        .unwrap_or(0)
}

#[tokio::test]
async fn gapless_switches_on_the_last_sample_of_the_outgoing_track() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["short-a", "short-b"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    let mut audible_before_switch = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut samples = Vec::new();
    loop {
        let block = harness.sink.pull(2048);
        samples.extend_from_slice(&block);
        let state = harness.api.player_state().await.expect("player state");
        if state.queue.index == Some(1) {
            assert!(matches!(state.intent, Intent::Committed { token: 2 }));
            assert!(state.fading.is_none());
            break;
        }
        assert_eq!(
            state.track.as_ref().map(|track| track.title.as_str()),
            Some("short-a")
        );
        audible_before_switch = samples.iter().filter(|sample| **sample != 0.0).count();
        assert!(
            tokio::time::Instant::now() < deadline,
            "no switch: {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The queue only moves once all of the first track has been heard.
    let one_second = TEST_CONFIG.sample_rate as usize * TEST_CONFIG.channels;
    assert!(
        audible_before_switch >= one_second - 2 * 2048,
        "switched after {audible_before_switch} samples"
    );
    let (_, tail) = record_until(&harness, "second track audible", 10, |_| true).await;
    samples.extend(tail);
    assert_eq!(longest_inner_silence(&samples), 0);
}

async fn finish_track_during_preload(harness: &mut Harness) {
    wait_state(&harness.api, "next track preloading", |state| {
        matches!(state.intent, Intent::Loading { token: 2, .. })
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            harness.sink.pull(2048);
            while let Ok(event) = harness.engine_events.try_recv() {
                if event == (EngineEvent::Ended { token: 1 }) {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("outgoing track ends while the preload is blocked");
}

#[tokio::test]
async fn failed_gapless_preload_retries_only_after_the_outgoing_track_ends() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider_attempts = attempts.clone();
    let mut harness = harness_with_provider(
        |_| {},
        Arc::new(move |track| {
            Some(
                if track.title == "track-b"
                    && provider_attempts.fetch_add(1, Ordering::Relaxed) == 0
                {
                    Box::new(|| Err(std::io::Error::other("preload failed")))
                } else {
                    wav_factory(track.duration)
                },
            )
        }),
    );
    harness
        .api
        .set_queue(replace(&["track-a", "track-b"]))
        .await
        .unwrap();
    wait_state(&harness.api, "preload failed", |state| {
        state.error.is_some() && state.intent == Intent::Committed { token: 1 }
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            harness.sink.pull(2048);
            while let Ok(event) = harness.engine_events.try_recv() {
                if matches!(event, EngineEvent::Position { token: 1, position } if position >= Duration::from_secs(2)) {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(attempts.load(Ordering::Relaxed), 1);

    drive_until(&harness, "retry after outgoing track ends", |state| {
        state.queue.index == Some(1) && state.intent == Intent::Committed { token: 3 }
    })
    .await;
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn late_failed_gapless_preload_stops_and_can_retry_the_next_track() {
    let gate = Arc::new((Mutex::new(true), Condvar::new()));
    let provider_gate = gate.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider_attempts = attempts.clone();
    let recorder = Arc::new(MemoryRecorder {
        recents: Mutex::new(Vec::new()),
        listens: Mutex::new(Vec::new()),
    });
    let mut harness = harness_with_services(
        |_| {},
        Arc::new(move |track| {
            Some(
                if track.title == "track-b"
                    && provider_attempts.fetch_add(1, Ordering::Relaxed) == 0
                {
                    gated_failing_factory(provider_gate.clone())
                } else {
                    wav_factory(track.duration)
                },
            )
        }),
        Some(recorder.clone()),
    );
    harness
        .api
        .set_queue(replace(&["short-a", "track-b"]))
        .await
        .unwrap();
    // Let the outgoing track play past its one-second listen threshold.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    finish_track_during_preload(&mut harness).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    *gate.0.lock().unwrap() = false;
    gate.1.notify_all();

    let failed = wait_state(&harness.api, "late preload failed", |state| {
        state.error.is_some()
    })
    .await;
    assert_eq!(failed.intent, Intent::Stopped);
    assert_eq!(failed.phase, ApiPhase::Ended);
    assert_eq!(failed.queue.index, Some(1));
    assert_eq!(
        failed.position.map(|anchor| (anchor.ms, anchor.playing)),
        Some((0, false))
    );
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
    assert!(harness.sink.pull(2048).iter().all(|sample| *sample == 0.0));

    harness
        .api
        .player_command(PlayerCommand::Play)
        .await
        .unwrap();
    wait_committed(&harness.api).await;
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(harness.sink.pull(2048).iter().any(|sample| *sample != 0.0));
    assert_eq!(recorder.listens.lock().unwrap().as_slice(), ["short-a"]);
    assert_eq!(
        recorder.recents.lock().unwrap().as_slice(),
        ["short-a", "track-b"]
    );
}

#[tokio::test]
async fn late_successful_gapless_preload_commits_history_and_seek_duration() {
    let gate = Arc::new((Mutex::new(true), Condvar::new()));
    let provider_gate = gate.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider_attempts = attempts.clone();
    let recorder = Arc::new(MemoryRecorder {
        recents: Mutex::new(Vec::new()),
        listens: Mutex::new(Vec::new()),
    });
    let mut harness = harness_with_services(
        |_| {},
        Arc::new(move |track| {
            Some(if track.title == "track-b" {
                provider_attempts.fetch_add(1, Ordering::Relaxed);
                gated_factory(track.duration, provider_gate.clone())
            } else {
                wav_factory(track.duration)
            })
        }),
        Some(recorder.clone()),
    );
    harness
        .api
        .set_queue(replace(&["short-a", "track-b"]))
        .await
        .unwrap();
    // Let the outgoing track play past its one-second listen threshold.
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    finish_track_during_preload(&mut harness).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let waiting = harness.api.player_state().await.unwrap();
    *gate.0.lock().unwrap() = false;
    gate.1.notify_all();
    assert_eq!(waiting.phase, ApiPhase::Ended);
    assert_eq!(waiting.position.map(|anchor| anchor.playing), Some(false));

    wait_state(&harness.api, "late preload committed", |state| {
        state.queue.index == Some(1)
            && state.phase == ApiPhase::Playing
            && matches!(state.intent, Intent::Committed { token: 2 })
    })
    .await;
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
    harness
        .api
        .player_command(PlayerCommand::Seek { position_ms: 3_000 })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = harness.engine_events.recv().await {
            if matches!(event, EngineEvent::Position { token: 2, position } if position >= Duration::from_secs(3)) {
                return;
            }
        }
        panic!("engine events closed before the seek");
    })
    .await
    .expect("seek uses the incoming duration rather than the outgoing one-second track");
    assert_eq!(recorder.listens.lock().unwrap().as_slice(), ["short-a"]);
    assert_eq!(
        recorder.recents.lock().unwrap().as_slice(),
        ["short-a", "track-b"]
    );
}

#[tokio::test]
async fn late_gapless_preload_can_be_cancelled_after_the_queue_advances() {
    for command in [PlayerCommand::Pause, PlayerCommand::Next] {
        let gate = Arc::new((Mutex::new(true), Condvar::new()));
        let provider_gate = gate.clone();
        let mut harness = harness_with_provider(
            |_| {},
            Arc::new(move |track| {
                Some(if track.title == "track-b" {
                    gated_factory(track.duration, provider_gate.clone())
                } else {
                    wav_factory(track.duration)
                })
            }),
        );
        harness
            .api
            .set_queue(replace(&["short-a", "track-b", "track-c"]))
            .await
            .unwrap();
        finish_track_during_preload(&mut harness).await;
        wait_state(&harness.api, "queue advanced while preloading", |state| {
            state.queue.index == Some(1) && state.phase == ApiPhase::Ended
        })
        .await;
        harness.api.player_command(command).await.unwrap();
        *gate.0.lock().unwrap() = false;
        gate.1.notify_all();
        if command == PlayerCommand::Next {
            wait_committed(&harness.api).await;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
        let state = harness.api.player_state().await.unwrap();
        if command == PlayerCommand::Pause {
            assert_eq!(state.intent, Intent::Stopped);
            assert_eq!(state.queue.index, Some(1));
            assert!(harness.sink.pull(2048).iter().all(|sample| *sample == 0.0));
        } else {
            assert_eq!(state.intent, Intent::Committed { token: 3 });
            assert_eq!(state.queue.index, Some(2));
            assert!(harness.sink.pull(2048).iter().any(|sample| *sample != 0.0));
        }
    }
}

#[tokio::test]
async fn a_queue_edit_after_gapless_arming_rearms_the_next_track() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["short-a", "short-b"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    wait_state(&harness.api, "gapless armed", |state| {
        state.queue.index == Some(0) && matches!(state.intent, Intent::Committed { token: 2 })
    })
    .await;

    harness
        .api
        .set_queue(enqueue(QueueMode::PlayNext, &["short-c"]))
        .await
        .expect("play next after arming");
    assert_eq!(
        queue_titles(&harness.api).await,
        vec!["short-a", "short-c", "short-b"]
    );
    wait_state(&harness.api, "re-armed", |state| {
        state.queue.index == Some(0) && matches!(state.intent, Intent::Committed { token: 3 })
    })
    .await;

    let (state, samples) = record_until(&harness, "edited next track playing", 10, |state| {
        state.queue.index == Some(1)
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("short-c")
    );
    assert_eq!(longest_inner_silence(&samples), 0);
}

#[tokio::test]
async fn repeat_one_loops_the_track_gapless() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["short-a", "short-b"]))
        .await
        .expect("set queue");
    harness
        .api
        .player_command(PlayerCommand::SetMode {
            shuffle: None,
            loop_mode: Some(LoopMode::Track),
        })
        .await
        .expect("repeat one");
    wait_committed(&harness.api).await;

    let (state, samples) = record_until(&harness, "track replayed", 10, |state| {
        matches!(state.intent, Intent::Committed { token } if token >= 3)
            && state.queue.index == Some(0)
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("short-a")
    );
    assert_eq!(longest_inner_silence(&samples), 0);
}

#[tokio::test]
async fn crossfade_above_zero_still_fades_instead_of_queueing() {
    let harness = harness(|config| config.playback.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;

    // Nothing is armed until the crossfade window, unlike the gapless preload.
    let early = harness.api.player_state().await.expect("state");
    assert!(matches!(early.intent, Intent::Committed { token: 1 }));

    let state = drive_until(&harness, "crossfade running", |state| {
        state.fading.is_some()
    })
    .await;
    assert_eq!(state.queue.index, Some(0));
    assert!(matches!(state.intent, Intent::Committed { token: 2 }));
    drive_until(&harness, "crossfade committed", |state| {
        state.queue.index == Some(1) && state.fading.is_none()
    })
    .await;
}

/// A source whose radio pages by cursor: `p1` and `p2` add tracks, and `p2`
/// hands back `p1` again, the loop a source echoing its token makes.
#[derive(Default)]
struct RadioLibrary {
    asked: Mutex<Vec<String>>,
    gate: Option<tokio::sync::Semaphore>,
}

impl RadioLibrary {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("asked lock").clone()
    }
}

fn radio_tracks(keys: &[&str]) -> Vec<Track> {
    keys.iter()
        .map(|key| test_track(&(*key).to_string()))
        .collect()
}

#[async_trait::async_trait]
impl QueueMaterializer for RadioLibrary {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
        StubLibrary.materialize(context).await
    }

    async fn materialize_queue(&self, context: &QueueContext) -> Result<RadioPage, ApiError> {
        match context {
            QueueContext::TrackRadio { key } => Ok(RadioPage {
                tracks: radio_tracks(&[key, "r1", "r2", "r3", "r4", "r5"]),
                more: Some("p1".into()),
            }),
            other => Ok(RadioPage::last(self.materialize(other).await?)),
        }
    }

    async fn more_radio(&self, cursor: &str) -> Result<RadioPage, ApiError> {
        self.asked
            .lock()
            .expect("asked lock")
            .push(cursor.to_string());
        if let Some(gate) = &self.gate {
            gate.acquire().await.expect("gate open").forget();
        }
        Ok(match cursor {
            "p1" => RadioPage {
                tracks: radio_tracks(&["r6", "r3", "r7", "r8", "r9", "r10"]),
                more: Some("p2".into()),
            },
            "p2" => RadioPage {
                tracks: radio_tracks(&["r11", "r12", "r13"]),
                more: Some("p1".into()),
            },
            _ => RadioPage::default(),
        })
    }
}

fn radio_harness(library: Arc<RadioLibrary>) -> Harness {
    let sink = FakeSinkHandle::default();
    let player = Player::try_with_sink(Box::new(FakeSink(sink.clone()))).unwrap();
    let engine_events = player.subscribe();
    let mut services = PlaybackServices::default();
    services.settings.playback.crossfade_seconds = 0;
    let session = SessionHandle::spawn_with_factory(
        library,
        player,
        services,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    Harness {
        api: LocalApi::new(session),
        sink,
        engine_events,
    }
}

fn track_radio(key: &str) -> SetQueueRequest {
    SetQueueRequest {
        mode: QueueMode::Replace,
        context: QueueContext::TrackRadio { key: key.into() },
        start_index: Some(0),
        shuffle: None,
    }
}

fn from_context(mode: QueueMode, context: QueueContext) -> SetQueueRequest {
    SetQueueRequest {
        mode,
        context,
        start_index: None,
        shuffle: None,
    }
}

async fn queue_keys(api: &LocalApi) -> Vec<String> {
    let snapshot = api.queue_snapshot().await.expect("queue snapshot");
    snapshot.items.into_iter().map(|track| track.key).collect()
}

#[tokio::test]
async fn a_radio_from_the_playing_song_keeps_playing_it() {
    let harness = radio_harness(Arc::new(RadioLibrary::default()));
    harness.api.set_queue(replace(&["seed"])).await.unwrap();
    let before = wait_committed(&harness.api).await;

    harness.api.set_queue(track_radio("seed")).await.unwrap();
    let after = harness.api.player_state().await.expect("player state");
    assert_eq!(after.intent, before.intent, "the song was not loaded again");
    assert_eq!(after.queue.index, Some(0));
    assert_eq!(after.queue.length, 6);
    assert_eq!(
        queue_keys(&harness.api).await,
        ["seed", "r1", "r2", "r3", "r4", "r5"]
    );
}

#[tokio::test]
async fn a_radio_from_another_song_starts_its_seed() {
    let harness = radio_harness(Arc::new(RadioLibrary::default()));
    harness.api.set_queue(replace(&["other"])).await.unwrap();
    let before = wait_committed(&harness.api).await;

    harness.api.set_queue(track_radio("seed")).await.unwrap();
    let after = wait_state(&harness.api, "the seed committed", |state| {
        matches!(state.intent, Intent::Committed { .. }) && state.intent != before.intent
    })
    .await;
    assert_eq!(after.track.map(|track| track.key).as_deref(), Some("seed"));
}

#[tokio::test]
async fn a_radio_tops_up_after_a_gapless_switch() {
    let library = Arc::new(RadioLibrary::default());
    let harness = radio_harness(library.clone());
    harness.api.set_queue(track_radio("seed")).await.unwrap();
    wait_committed(&harness.api).await;

    let (state, samples) = record_until(&harness, "radio topped up after switching", 10, |state| {
        state.queue.index == Some(1) && state.queue.length == 11
    })
    .await;

    assert_eq!(state.track.as_ref().unwrap().title, "r1");
    assert!(state.fading.is_none());
    assert_eq!(library.asked(), ["p1"]);
    assert_eq!(longest_inner_silence(&samples), 0);
}

#[tokio::test]
async fn a_radio_tops_up_below_five_and_stops_on_a_repeated_cursor() {
    let library = Arc::new(RadioLibrary::default());
    let harness = radio_harness(library.clone());
    harness.api.set_queue(track_radio("seed")).await.unwrap();
    wait_committed(&harness.api).await;
    assert!(
        library.asked().is_empty(),
        "five tracks follow the seed, so nothing is fetched yet"
    );

    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 1 })
        .await
        .unwrap();
    wait_state(&harness.api, "the first top-up", |state| {
        state.queue.length == 11
    })
    .await;
    assert_eq!(library.asked(), ["p1"]);

    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 8 })
        .await
        .unwrap();
    wait_state(&harness.api, "the second top-up", |state| {
        state.queue.length == 14
    })
    .await;

    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 13 })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        library.asked(),
        ["p1", "p2"],
        "p2 handed back p1, which was already spent"
    );
    let keys = queue_keys(&harness.api).await;
    let unique: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!(
        unique.len(),
        keys.len(),
        "no track is queued twice: {keys:?}"
    );
}

#[tokio::test]
async fn a_radio_that_runs_out_plays_on_once_the_top_up_lands() {
    let library = Arc::new(RadioLibrary {
        gate: Some(tokio::sync::Semaphore::new(0)),
        ..Default::default()
    });
    let harness = radio_harness(library.clone());
    harness.api.set_queue(track_radio("seed")).await.unwrap();
    wait_committed(&harness.api).await;
    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 5 })
        .await
        .unwrap();
    wait_committed(&harness.api).await;

    harness
        .api
        .player_command(PlayerCommand::Next)
        .await
        .unwrap();
    let ended = harness.api.player_state().await.unwrap();
    assert_eq!(
        (
            ended.queue.index,
            ended.queue.length,
            ended.phase == ApiPhase::Playing
        ),
        (Some(5), 6, false),
        "the page is still on its way"
    );

    library.gate.as_ref().unwrap().add_permits(1);
    let state = wait_state(&harness.api, "playback onto the new page", |state| {
        state.queue.index == Some(6) && state.phase == ApiPhase::Playing
    })
    .await;
    assert_eq!(state.queue.length, 11);
}

#[tokio::test]
async fn replacing_a_radio_queue_ends_its_top_ups() {
    let library = Arc::new(RadioLibrary::default());
    let harness = radio_harness(library.clone());
    harness.api.set_queue(track_radio("seed")).await.unwrap();
    wait_committed(&harness.api).await;

    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .unwrap();
    wait_committed(&harness.api).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(library.asked().is_empty(), "{:?}", library.asked());
    assert_eq!(harness.api.player_state().await.unwrap().queue.length, 2);
}

#[tokio::test]
async fn a_restored_radio_queue_picks_its_cursor_back_up() {
    let library = Arc::new(RadioLibrary::default());
    let harness = radio_harness(library.clone());
    let snapshot = db::QueueSnapshot {
        version: 1,
        queue: radio_tracks(&["seed", "r1", "r2"]),
        radio_cursor: Some("p1".into()),
        ..Default::default()
    };
    harness.api.session.restore_queue(snapshot).await.unwrap();

    wait_state(&harness.api, "the restored radio's top-up", |state| {
        state.queue.length == 9
    })
    .await;
    assert_eq!(library.asked(), ["p1"]);
}

/// A server keeping a play queue of its own, recording each queue it takes and refusing them while `failing` is set.
struct QueueServer {
    inner: server::source::ActiveSource,
    saves: Mutex<Vec<Vec<String>>>,
    failing: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl server::source::MediaSource for QueueServer {
    fn source(&self) -> &config::Source {
        self.inner.source()
    }

    fn db(&self) -> &db::Db {
        self.inner.db()
    }

    fn capabilities(&self) -> server::source::Capabilities {
        server::source::Capabilities {
            play_queue: true,
            ..self.inner.capabilities()
        }
    }

    async fn add_to_playlist(
        &self,
        playlist_id: &str,
        item_refs: &[String],
    ) -> Result<Vec<String>, server::source::SourceError> {
        self.inner.add_to_playlist(playlist_id, item_refs).await
    }

    async fn create_playlist(
        &self,
        name: &str,
        item_refs: &[String],
    ) -> Result<String, server::source::SourceError> {
        self.inner.create_playlist(name, item_refs).await
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        track: &Track,
        position: usize,
    ) -> Result<(), server::source::SourceError> {
        self.inner
            .remove_from_playlist(playlist_id, track, position)
            .await
    }

    async fn resolve_stream(
        &self,
        item_id: &str,
        quality: config::StreamQuality,
    ) -> Result<server::source::StreamInfo, server::source::SourceError> {
        self.inner.resolve_stream(item_id, quality).await
    }

    async fn validate(&self) -> server::source::AuthOutcome {
        self.inner.validate().await
    }

    async fn fetch_favorites(&self) -> Result<Vec<String>, server::source::SourceError> {
        self.inner.fetch_favorites().await
    }

    async fn push_favorite(
        &self,
        item_id: &str,
        on: bool,
    ) -> Result<(), server::source::SourceError> {
        self.inner.push_favorite(item_id, on).await
    }

    async fn save_play_queue(
        &self,
        queue: &[Track],
        _current_index: usize,
        _position_ms: u64,
    ) -> Result<bool, server::source::SourceError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(server::source::SourceError::Connectivity);
        }
        self.saves
            .lock()
            .expect("saves lock")
            .push(queue.iter().map(|track| track.title.clone()).collect());
        Ok(true)
    }
}

async fn queue_server() -> (Arc<QueueServer>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = db::init(&dir.path().join("server.db")).await.expect("db");
    let inner = Arc::from(server::source::active(
        db,
        &server::sources::Sources::default(),
    ));
    let server = Arc::new(QueueServer {
        inner,
        saves: Mutex::new(Vec::new()),
        failing: std::sync::atomic::AtomicBool::new(false),
    });
    (server, dir)
}

async fn flush_remote_queue(jobs: &mpsc::UnboundedSender<RemoteQueueJob>) {
    let (done, flushed) = oneshot::channel();
    jobs.send(RemoteQueueJob::Flush(done))
        .expect("writer alive");
    flushed.await.expect("writer answers");
}

/// A push the server refused is not marked as synced, so the next tick sends it again.
#[tokio::test]
async fn a_remote_queue_push_counts_only_once_the_server_takes_it() {
    use crate::persistence::QueueStore as _;
    let (server, _dir) = queue_server().await;
    let store = Arc::new(MemoryStore::default());
    let synced = SyncedMark::default();
    let (jobs, rx) = mpsc::unbounded_channel();
    let writer_store: Arc<dyn crate::persistence::QueueStore> = store.clone();
    tokio::spawn(write_remote_queue(rx, synced.clone(), Some(writer_store)));
    let mark = RemoteQueueMark {
        tracks: 1,
        current_key: Some("a".into()),
        playing: false,
    };
    let push = || {
        RemoteQueueJob::Push(RemoteQueuePush {
            source: server.clone(),
            store_key: config::Source::default(),
            queue: vec![test_track(&"a".to_string())],
            current_index: 0,
            position_ms: 1000,
            mark: mark.clone(),
        })
    };

    server.failing.store(true, Ordering::SeqCst);
    jobs.send(push()).expect("writer alive");
    flush_remote_queue(&jobs).await;
    assert_eq!(*synced.lock().expect("lock"), None);
    assert_eq!(store.synced_remote(&config::Source::default()).await, None);

    server.failing.store(false, Ordering::SeqCst);
    jobs.send(push()).expect("writer alive");
    flush_remote_queue(&jobs).await;
    assert_eq!(*synced.lock().expect("lock"), Some(mark));
    assert_eq!(
        store.synced_remote(&config::Source::default()).await,
        Some(crate::persistence::queue_fingerprint(
            ["a"],
            Some("a"),
            1000
        ))
    );
}

/// Clearing the queue clears the server's, or the next start would adopt it back; a session that never had one sends nothing.
#[tokio::test]
async fn clearing_the_queue_clears_the_servers_too() {
    let (server, _dir) = queue_server().await;
    let player = Player::try_with_sink(Box::new(FakeSink(FakeSinkHandle::default())))
        .expect("headless player starts");
    let services = PlaybackServices {
        active_source: Some(server.clone()),
        queue_store: Some(Arc::new(MemoryStore::default())),
        ..Default::default()
    };
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        services,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    let api = LocalApi::new(session.clone());

    session.persist_now().await;
    assert!(server.saves.lock().expect("lock").is_empty());

    api.set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");
    wait_committed(&api).await;
    session.persist_now().await;
    session.reset_playback().await.expect("clear");
    session.persist_now().await;

    let saves = server.saves.lock().expect("lock").clone();
    assert_eq!(saves.first().map(Vec::len), Some(2));
    assert_eq!(saves.last(), Some(&Vec::new()));
}

/// A queue fetched from the server lands only if nothing changed the local one while it was on its way.
#[tokio::test]
async fn an_adopted_queue_never_replaces_a_newer_local_one() {
    let harness = harness(|_| {});
    let session = harness.api.session.clone();
    let remote = db::QueueSnapshot {
        version: 1,
        queue: ["remote-0", "remote-1", "remote-2"]
            .iter()
            .map(|key| test_track(&(*key).to_string()))
            .collect(),
        ..Default::default()
    };
    let before_fetch = session.state().queue.rev;
    harness
        .api
        .set_queue(replace(&["track-0", "track-1"]))
        .await
        .expect("set queue");

    let adopted = session
        .adopt_queue(remote.clone(), &config::Source::default(), before_fetch)
        .await
        .expect("adopt");
    assert!(adopted.is_none());
    assert_eq!(queue_titles(&harness.api).await, ["track-0", "track-1"]);

    let adopted = session
        .adopt_queue(
            remote,
            &config::Source::default(),
            session.state().queue.rev,
        )
        .await
        .expect("adopt");
    assert!(adopted.is_some());
    assert_eq!(
        queue_titles(&harness.api).await,
        ["remote-0", "remote-1", "remote-2"]
    );

    let elsewhere = config::Source::LocalLibrary("local:b".into());
    let adopted = session
        .adopt_queue(
            db::QueueSnapshot::default(),
            &elsewhere,
            session.state().queue.rev,
        )
        .await
        .expect("adopt");
    assert!(adopted.is_none(), "fetched from a source no longer active");

    let cleared = session
        .adopt_queue(
            db::QueueSnapshot::default(),
            &config::Source::default(),
            session.state().queue.rev,
        )
        .await
        .expect("adopt");
    assert!(cleared.is_some());
    assert!(queue_titles(&harness.api).await.is_empty());
}

/// Entering a source loads its queue, which is no edit: only what is done to it there reaches its server.
#[tokio::test]
async fn switching_to_a_queue_server_pushes_only_the_edits_made_there() {
    let (server, _dir) = queue_server().await;
    let player = Player::try_with_sink(Box::new(FakeSink(FakeSinkHandle::default())))
        .expect("headless player starts");
    let mut config = config::Settings::default();
    config.library.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    let on_a = active_on(config::Source::default());
    let on_b = active_on(config::Source::LocalLibrary("local:b".into()));
    let services = PlaybackServices {
        settings: config,
        sources: on_a.clone(),
        queue_store: Some(Arc::new(MemoryStore::default())),
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

    session.set_active_source(Some(server.clone()));
    session.set_sources(on_b.clone());
    session.persist_now().await;
    assert!(
        server.saves.lock().expect("lock").is_empty(),
        "the switch pushed the queue it loaded"
    );

    api.set_queue(replace(&["b-0"])).await.expect("set queue");
    wait_committed(&api).await;
    session.persist_now().await;
    assert_eq!(*server.saves.lock().expect("lock"), [["b-0"]]);

    session.set_active_source(None);
    session.set_sources(on_a);
    session.persist_now().await;
    session.set_active_source(Some(server.clone()));
    session.set_sources(on_b);
    session.persist_now().await;
    assert_eq!(
        server.saves.lock().expect("lock").len(),
        1,
        "resuming the parked queue pushed it"
    );
}

/// A queue taken from the server is not sent back to it.
#[tokio::test]
async fn an_adopted_queue_is_not_pushed_back() {
    let (server, _dir) = queue_server().await;
    let player = Player::try_with_sink(Box::new(FakeSink(FakeSinkHandle::default())))
        .expect("headless player starts");
    let store = Arc::new(MemoryStore::default());
    let services = PlaybackServices {
        active_source: Some(server.clone()),
        queue_store: Some(store.clone()),
        ..Default::default()
    };
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        services,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    let remote = db::QueueSnapshot {
        version: 1,
        queue: ["remote-0", "remote-1"]
            .iter()
            .map(|key| test_track(&(*key).to_string()))
            .collect(),
        progress_secs: 3,
        ..Default::default()
    };
    let adopted = session
        .adopt_queue(
            remote,
            &config::Source::default(),
            session.state().queue.rev,
        )
        .await
        .expect("adopt");
    assert!(adopted.is_some());
    session.persist_now().await;

    assert!(server.saves.lock().expect("lock").is_empty());
    let saved = store.saved.lock().expect("store lock");
    let (_, last) = saved.last().expect("the adopted queue is stored here");
    assert_eq!(last.queue.len(), 2);
}

#[tokio::test]
async fn switching_to_the_video_keeps_the_place_in_the_song_and_the_queue() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["ytm-song:a", "track-1"]))
        .await
        .expect("queue");
    wait_committed(&harness.api).await;
    for _ in 0..40 {
        harness.sink.pull(2048);
    }
    harness
        .api
        .player_command(PlayerCommand::Pause)
        .await
        .expect("pause");
    let paused_at = harness
        .api
        .player_state()
        .await
        .expect("state")
        .position
        .expect("anchor")
        .ms;
    assert!(paused_at > 0, "the song played before the switch");

    harness
        .api
        .player_command(PlayerCommand::SetVersion {
            version: api::TrackVersion::Video,
        })
        .await
        .expect("switch to the video");
    let state = harness.api.player_state().await.expect("state");
    let track = state.track.expect("the video plays");
    assert_eq!(track.key, "ytm-song:a-video");
    assert_eq!(track.version(), Some(api::TrackVersion::Video));
    assert_eq!(
        track.counterpart.as_ref().map(|other| other.key.as_str()),
        Some("ytm-song:a")
    );
    // Paused, the video waits at the same note: a second later, past its intro.
    assert_eq!(state.position.expect("anchor").ms, paused_at + 1_000);
    assert!(!state.position.unwrap().playing);
    assert_eq!(state.queue.length, 2);
    assert_eq!(queue_titles(&harness.api).await, ["ytm-song:a", "track-1"]);
    let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
    assert_eq!(snapshot.items[0].key, "ytm-song:a-video");
    assert_eq!(snapshot.items[1].key, "track-1");

    harness
        .api
        .player_command(PlayerCommand::Play)
        .await
        .expect("play");
    // The load commits before its seek lands, so wait for the resumed place
    // rather than reading the first committed state.
    let state = wait_state(&harness.api, "video resumed at the song's place", |state| {
        state.phase == ApiPhase::Playing
            && matches!(state.intent, Intent::Committed { .. })
            && state
                .position
                .is_some_and(|anchor| anchor.ms >= paused_at + 1_000)
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|t| t.key.as_str()),
        Some("ytm-song:a-video")
    );

    // Playing, the song comes back where the video stood, less the intro.
    for _ in 0..20 {
        harness.sink.pull(2048);
    }
    let before = harness
        .api
        .player_state()
        .await
        .expect("state")
        .position
        .expect("anchor");
    harness
        .api
        .player_command(PlayerCommand::SetVersion {
            version: api::TrackVersion::Song,
        })
        .await
        .expect("switch back");
    let state = wait_state(&harness.api, "the song playing again", |state| {
        state.track.as_ref().is_some_and(|t| t.key == "ytm-song:a")
            && state.phase == ApiPhase::Playing
            && matches!(state.intent, Intent::Committed { .. })
    })
    .await;
    let resumed = state.position.expect("anchor").ms;
    assert!(
        resumed + 1_000 >= before.ms && resumed <= 6_000,
        "song resumed at {resumed}ms after the video stood at {}ms",
        before.ms
    );
    assert_eq!(state.queue.length, 2);
    assert_eq!(state.queue.index, Some(0));

    let rev = state.rev;
    harness
        .api
        .player_command(PlayerCommand::SetVersion {
            version: api::TrackVersion::Song,
        })
        .await
        .expect("a no-op");
    assert_eq!(
        harness.api.player_state().await.unwrap().track.unwrap().key,
        "ytm-song:a"
    );
    assert!(harness.api.player_state().await.unwrap().rev >= rev);
}

#[tokio::test]
async fn a_track_with_no_other_version_refuses_the_switch() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["track-0"]))
        .await
        .expect("queue");
    wait_committed(&harness.api).await;
    let error = harness
        .api
        .player_command(PlayerCommand::SetVersion {
            version: api::TrackVersion::Video,
        })
        .await
        .expect_err("no video to switch to");
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(
        harness.api.player_state().await.unwrap().track.unwrap().key,
        "track-0"
    );
}

#[tokio::test]
async fn a_counterpart_the_source_finds_later_reaches_the_playing_row() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["ytm-bare:b", "track-1"]))
        .await
        .expect("queue");
    let state = wait_committed(&harness.api).await;
    let track = state.track.expect("playing");
    assert_eq!(track.counterpart, None);

    let _ = harness
        .api
        .session
        .cmd_tx
        .send(SessionCmd::CounterpartFound {
            uid: track.uid.clone(),
            counterpart: Box::new(music_video_of("ytm-bare:b")),
        });
    let state = wait_state(&harness.api, "the counterpart on the row", |state| {
        state
            .track
            .as_ref()
            .is_some_and(|t| t.counterpart.is_some())
    })
    .await;
    assert_eq!(
        state.track.unwrap().version(),
        Some(api::TrackVersion::Song)
    );
    let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
    assert_eq!(
        snapshot.items[0]
            .counterpart
            .as_ref()
            .map(|c| c.key.as_str()),
        Some("ytm-bare:b-video")
    );
    assert_eq!(snapshot.items[1].counterpart, None);
}

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
            uploads: false,
            storage_quota: false,
            discover: false,
            dont_recommend: false,
            radio: RadioSeeds {
                track: true,
                ..RadioSeeds::NONE
            },
            play_queue: false,
            playlists: PlaylistOps::None,
            artist_view: ArtistView::Library,
            albums: AlbumType::Standard,
            favorites_sync: FavoritesSync::Instant,
            account_avatar: false,
            stream_quality: false,
            explicit_flags: true,
            watch_history: false,
            complete_play_history: false,
            accounts: false,
            music_videos: false,
            library_actions: server::source::LibraryActions::NONE,
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

async fn autoplay_harness(
    configure: impl FnOnce(&mut config::Settings),
) -> PlaybackSettingsHarness {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("radio.db")).await.expect("db");
    let sink = FakeSinkHandle::default();
    let player =
        Player::try_with_sink(Box::new(FakeSink(sink.clone()))).expect("headless player starts");
    let mut services = PlaybackServices::default();
    services.settings.playback.crossfade_seconds = 0;
    configure(&mut services.settings);
    services.active_source = Some(Arc::new(RadioSource {
        db: database,
        source: config::Source::default(),
    }));
    let engine_events = player.subscribe();
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
            engine_events,
        },
        session,
        _dir: dir,
    }
}

#[tokio::test]
async fn skipping_explicit_tracks_leaves_them_out_of_a_new_queue() {
    let harness = harness(|config| config.playback.skip_explicit = true);
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
    } = autoplay_harness(|_| {}).await;
    harness
        .api
        .set_queue(replace(&["short-0", "short-explicit-1", "short-2"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    let mut config = session.settings_watch().borrow().clone();
    config.playback.skip_explicit = true;
    session.set_settings(
        config,
        vec![config::SettingKey::Playback(
            config::PlaybackKey::SkipExplicit,
        )],
        1,
    );

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
    let PlaybackSettingsHarness { harness, .. } = autoplay_harness(|config| {
        config.playback.autoplay_radio = true;
        config.playback.skip_explicit = true;
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
            } = autoplay_harness(|config| config.playback.autoplay_radio = autoplay).await;
            (harness, Some((session, _dir)))
        } else {
            (
                harness(|config| config.playback.autoplay_radio = autoplay),
                None,
            )
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

fn album(id: &str) -> QueueContext {
    QueueContext::Album { id: id.into() }
}

fn playlist(id: &str) -> QueueContext {
    QueueContext::Playlist { id: id.into() }
}

async fn origin_of(api: &LocalApi) -> Option<QueueContext> {
    api.player_state().await.expect("state").queue.context
}

#[tokio::test]
async fn a_queue_built_from_a_container_remembers_it() {
    let harness = harness(|_| {});
    let containers = [
        album("a"),
        QueueContext::Artist {
            artist: "UC-x".into(),
        },
        QueueContext::Genre { name: "g".into() },
        playlist("p"),
        QueueContext::Filter {
            filter: api::TrackFilter {
                favorite: Some(true),
                ..Default::default()
            },
        },
        QueueContext::Radio {
            station_id: "s".into(),
            stream_id: "st".into(),
        },
        QueueContext::TrackRadio { key: "k".into() },
        QueueContext::PlaylistRadio { id: "p".into() },
    ];
    for context in containers {
        harness
            .api
            .set_queue(from_context(QueueMode::Replace, context.clone()))
            .await
            .expect("set queue");
        assert_eq!(origin_of(&harness.api).await, Some(context.clone()));
        let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
        assert_eq!(snapshot.context, Some(context));
    }
}

#[tokio::test]
async fn a_raw_track_queue_has_no_origin_and_replaces_one() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(&["t-0", "t-1"]))
        .await
        .expect("set queue");
    assert_eq!(origin_of(&harness.api).await, None);

    harness
        .api
        .set_queue(from_context(QueueMode::Replace, album("a")))
        .await
        .expect("set queue");
    assert_eq!(origin_of(&harness.api).await, Some(album("a")));

    harness
        .api
        .set_queue(replace(&["t-0"]))
        .await
        .expect("set queue");
    assert_eq!(origin_of(&harness.api).await, None);
    let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
    assert_eq!(snapshot.context, None);
}

#[tokio::test]
async fn queue_edits_keep_the_origin() {
    let harness = harness(|_| {});
    let api = &harness.api;
    api.set_queue(from_context(QueueMode::Replace, album("a")))
        .await
        .expect("set queue");

    api.set_queue(enqueue(QueueMode::Append, &["x"]))
        .await
        .expect("append");
    api.set_queue(from_context(QueueMode::Append, album("b")))
        .await
        .expect("append another container");
    api.set_queue(from_context(QueueMode::PlayNext, playlist("p")))
        .await
        .expect("play next");
    assert_eq!(origin_of(api).await, Some(album("a")));

    for edit in [
        QueueEdit::Insert {
            index: 1,
            keys: vec!["i".into()],
        },
        QueueEdit::Move { from: 4, to: 5 },
        QueueEdit::Remove { index: 6 },
        QueueEdit::Jump { index: 2 },
        QueueEdit::JumpPhysical { index: 1 },
    ] {
        api.queue_edit(edit.clone()).await.expect("edit");
        assert_eq!(origin_of(api).await, Some(album("a")), "{edit:?}");
    }
    api.player_command(PlayerCommand::SetMode {
        shuffle: Some(true),
        loop_mode: Some(LoopMode::Queue),
    })
    .await
    .expect("mode");
    assert_eq!(origin_of(api).await, Some(album("a")));
    let snapshot = api.queue_snapshot().await.expect("snapshot");
    assert_eq!(snapshot.context, Some(album("a")));
}

/// Adding to a queue is an edit, so only a queue with nothing in it becomes what was added.
#[tokio::test]
async fn adding_to_an_empty_queue_adopts_its_origin() {
    let harness = harness(|_| {});
    let api = &harness.api;
    api.set_queue(from_context(QueueMode::Append, album("empty")))
        .await
        .expect("append nothing");
    api.set_queue(enqueue(QueueMode::Append, &["x"]))
        .await
        .expect("append raw");
    assert_eq!(origin_of(api).await, None, "a raw list names no container");

    api.set_queue(replace(&[])).await.expect("clear");
    api.set_queue(from_context(QueueMode::Append, album("a")))
        .await
        .expect("append");
    assert_eq!(origin_of(api).await, Some(album("a")));

    api.set_queue(replace(&[])).await.expect("clear");
    api.set_queue(from_context(QueueMode::PlayNext, playlist("p")))
        .await
        .expect("play next");
    assert_eq!(origin_of(api).await, Some(playlist("p")));
}

#[tokio::test]
async fn a_replace_that_fails_keeps_the_origin() {
    let provider: FactoryOverride = Arc::new(|track| {
        (!track.title.starts_with("unavailable-server:"))
            .then(|| wav_factory(track.duration.min(6)))
    });
    let harness = harness_with_provider(|_| {}, provider);
    harness
        .api
        .set_queue(from_context(QueueMode::Replace, album("a")))
        .await
        .expect("set queue");

    let error = harness
        .api
        .set_queue(from_context(
            QueueMode::Replace,
            album("unavailable-server:x"),
        ))
        .await
        .expect_err("unavailable replacement is rejected");
    assert_eq!(error.code, ErrorCode::SourceUnreachable);
    assert_eq!(origin_of(&harness.api).await, Some(album("a")));
}

#[tokio::test]
async fn emptying_the_queue_drops_the_origin() {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(from_context(QueueMode::Replace, album("a")))
        .await
        .expect("set queue");

    harness
        .api
        .set_queue(from_context(QueueMode::Replace, album("empty")))
        .await
        .expect("replace with a container with nothing in it");
    assert_eq!(harness.api.player_state().await.unwrap().queue.length, 0);
    assert_eq!(origin_of(&harness.api).await, None);
}

#[tokio::test]
async fn the_origin_is_saved_with_the_queue() {
    let store = Arc::new(MemoryStore::default());
    let player = Player::try_with_sink(Box::new(FakeSink(FakeSinkHandle::default())))
        .expect("headless player starts");
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices {
            queue_store: Some(store.clone()),
            ..Default::default()
        },
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    let api = LocalApi::new(session.clone());
    let last_saved = || {
        let saved = store.saved.lock().expect("store lock");
        saved.last().expect("a snapshot").1.origin.clone()
    };

    api.set_queue(from_context(QueueMode::Replace, playlist("p")))
        .await
        .expect("set queue");
    session.persist_now().await;
    assert_eq!(
        last_saved().as_deref().and_then(origin::decode),
        Some(playlist("p"))
    );

    api.set_queue(replace(&["t-0"])).await.expect("set queue");
    session.persist_now().await;
    assert_eq!(last_saved(), None);
}

#[tokio::test]
async fn a_restored_queue_remembers_where_it_came_from() {
    let harness = harness(|_| {});
    let snapshot = |origin: Option<String>, keys: &[&str]| db::QueueSnapshot {
        version: 1,
        queue: keys
            .iter()
            .map(|key| test_track(&(*key).to_string()))
            .collect(),
        origin,
        ..Default::default()
    };
    let stored = origin::encode(&album("a"));
    assert!(stored.is_some());

    let restore = |snapshot| harness.api.session.restore_queue(snapshot);
    restore(snapshot(stored.clone(), &["a-0", "a-1"]))
        .await
        .expect("restore");
    assert_eq!(origin_of(&harness.api).await, Some(album("a")));
    let snapshot_now = harness.api.queue_snapshot().await.expect("snapshot");
    assert_eq!(snapshot_now.context, Some(album("a")));

    restore(snapshot(Some("not an origin".into()), &["a-0", "a-1"]))
        .await
        .expect("restore");
    assert_eq!(
        origin_of(&harness.api).await,
        None,
        "an unreadable origin is dropped"
    );
    assert_eq!(harness.api.player_state().await.unwrap().queue.length, 2);

    restore(snapshot(stored, &[])).await.expect("restore");
    assert_eq!(
        origin_of(&harness.api).await,
        None,
        "an empty queue has no origin"
    );
}

#[tokio::test]
async fn each_source_keeps_its_own_origin_across_a_switch() {
    let store = Arc::new(MemoryStore::default());
    let player = Player::try_with_sink(Box::new(FakeSink(FakeSinkHandle::default())))
        .expect("headless player starts");
    let mut config = config::Settings::default();
    config.library.local_sources.push(config::SavedLocalSource {
        id: "local:b".into(),
        name: "B".into(),
        directories: Vec::new(),
    });
    let on_a = active_on(config::Source::default());
    let on_b = active_on(config::Source::LocalLibrary("local:b".into()));
    let session = SessionHandle::spawn_with_factory(
        Arc::new(StubLibrary),
        player,
        PlaybackServices {
            settings: config,
            sources: on_a.clone(),
            queue_store: Some(store.clone()),
            ..Default::default()
        },
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    let api = LocalApi::new(session.clone());
    let switch = |to: &Arc<server::sources::Sources>| {
        session.set_sources(to.clone());
        session.persist_now()
    };

    api.set_queue(from_context(QueueMode::Replace, album("a")))
        .await
        .expect("set queue");
    switch(&on_b).await;
    assert_eq!(origin_of(&api).await, None, "B was never visited");
    api.set_queue(from_context(QueueMode::Replace, playlist("p")))
        .await
        .expect("set queue");

    switch(&on_a).await;
    assert_eq!(origin_of(&api).await, Some(album("a")));
    switch(&on_b).await;
    assert_eq!(origin_of(&api).await, Some(playlist("p")));
}

/// Whether the sink is being fed sound, polled until it is (or is not) so the engine has time to see the command.
async fn wait_output(harness: &Harness, audible: bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let heard = harness.sink.pull(2048).iter().any(|sample| *sample != 0.0);
        if heard == audible {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the output never became audible={audible}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn muting_silences_the_engine_and_keeps_the_level() {
    let harness = harness_at_volume(0.5, false);
    harness
        .api
        .set_queue(replace(&["track-0"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    wait_output(&harness, true).await;

    harness
        .api
        .player_command(PlayerCommand::SetMuted { muted: true })
        .await
        .expect("mute");
    let state = harness.api.player_state().await.expect("state");
    assert!(state.muted);
    assert_eq!(state.volume, 0.5, "the level to return to is kept");
    assert_eq!(state.phase, ApiPhase::Playing, "muting is not pausing");
    wait_output(&harness, false).await;

    harness
        .api
        .player_command(PlayerCommand::SetMuted { muted: false })
        .await
        .expect("unmute");
    let state = harness.api.player_state().await.expect("state");
    assert!(!state.muted);
    assert_eq!(state.volume, 0.5);
    wait_output(&harness, true).await;
}

/// Touching the level means wanting to hear it, so a `SetVolume` while muted unmutes; starting muted at a stored level does not.
#[tokio::test]
async fn setting_a_level_while_muted_unmutes_and_muting_leaves_the_level() {
    let harness = harness_at_volume(0.5, true);
    let api = &harness.api;
    let mute = |muted| api.player_command(PlayerCommand::SetMuted { muted });

    let state = api.player_state().await.expect("state");
    assert!(state.muted, "a stored mute is kept at spawn");
    assert_eq!(state.volume, 0.5);
    mute(true).await.expect("muting again changes nothing");
    let state = api.player_state().await.expect("state");
    assert!(state.muted);
    assert_eq!(state.volume, 0.5);

    api.player_command(PlayerCommand::SetVolume { volume: 0.3 })
        .await
        .expect("set volume");
    let state = api.player_state().await.expect("state");
    assert!(!state.muted);
    assert_eq!(state.volume, 0.3);

    mute(false).await.expect("unmuting what is not muted");
    assert_eq!(api.player_state().await.unwrap().volume, 0.3);
}

#[tokio::test]
async fn an_integration_taking_over_while_muted_starts_silent() {
    let harness = harness(|_| {});
    harness
        .api
        .player_command(PlayerCommand::SetMuted { muted: true })
        .await
        .expect("mute");
    let stub = Arc::new(StubExternal::default());
    harness.api.session.attach_external(stub.clone());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(stub.calls(), vec!["volume:0"]);
}

/// The stub's calls without the level every attach hands over.
fn transport_calls(stub: &StubExternal) -> Vec<String> {
    stub.calls()
        .into_iter()
        .filter(|call| !call.starts_with("volume:"))
        .collect()
}

fn queue_changes(events: &mut tokio::sync::broadcast::Receiver<ApiEvent>) -> usize {
    let mut changes = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, ApiEvent::QueueChanged { .. }) {
            changes += 1;
        }
    }
    changes
}

async fn titles_and_position(api: &LocalApi) -> (Vec<String>, Option<u32>) {
    let snapshot = api.queue_snapshot().await.expect("snapshot");
    let titles = snapshot.items.iter().map(|t| t.title.clone()).collect();
    (titles, snapshot.position)
}

async fn plain_harness(keys: &[&str], playing: u32) -> Harness {
    let harness = harness(|_| {});
    harness
        .api
        .set_queue(replace(keys))
        .await
        .expect("set queue");
    harness
        .api
        .queue_edit(QueueEdit::Jump { index: playing })
        .await
        .expect("jump");
    harness
}

async fn shuffle_off(api: &LocalApi) {
    api.player_command(PlayerCommand::SetMode {
        shuffle: Some(false),
        loop_mode: None,
    })
    .await
    .expect("shuffle off");
}

const TEN: [&str; 10] = [
    "track-0", "track-1", "track-2", "track-3", "track-4", "track-5", "track-6", "track-7",
    "track-8", "track-9",
];

#[tokio::test]
async fn clear_upcoming_drops_only_what_follows_the_current_track() {
    let harness = plain_harness(&TEN, 3).await;
    let mut events = harness.api.session.subscribe();

    harness
        .api
        .queue_edit(QueueEdit::ClearUpcoming)
        .await
        .expect("clear upcoming");

    let (titles, position) = titles_and_position(&harness.api).await;
    assert_eq!(titles, vec!["track-0", "track-1", "track-2", "track-3"]);
    assert_eq!(position, Some(3));
    assert_eq!(queue_changes(&mut events), 1, "one edit, one QueueChanged");

    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 1 })
        .await
        .expect("history survives the edit");
}

#[tokio::test]
async fn clear_upcoming_under_shuffle_keeps_the_played_order_and_the_unshuffled_queue() {
    let harness = shuffled_harness().await;
    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 3 })
        .await
        .expect("jump");
    let before = harness.api.queue_snapshot().await.expect("snapshot");
    let mut events = harness.api.session.subscribe();

    harness
        .api
        .queue_edit(QueueEdit::ClearUpcoming)
        .await
        .expect("clear upcoming");

    let after = harness.api.queue_snapshot().await.expect("snapshot");
    let titles = |snapshot: &api::QueueSnapshot| -> Vec<String> {
        snapshot.items.iter().map(|t| t.title.clone()).collect()
    };
    assert_eq!(titles(&after), titles(&before)[..4].to_vec());
    assert_eq!(after.position, Some(3));
    assert!(after.shuffle);
    let mut permutation = after.shuffle_order.clone();
    permutation.sort_unstable();
    assert_eq!(
        permutation,
        vec![0, 1, 2, 3],
        "a permutation of what is left"
    );
    assert_eq!(queue_changes(&mut events), 1);

    shuffle_off(&harness.api).await;
    let (unshuffled, position) = titles_and_position(&harness.api).await;
    let mut expected = titles(&after);
    expected.sort();
    assert_eq!(
        unshuffled, expected,
        "survivors keep their unshuffled order"
    );
    assert_eq!(
        unshuffled[position.expect("position") as usize],
        titles(&after)[3]
    );
}

#[tokio::test]
async fn clear_upcoming_with_nothing_upcoming_is_a_quiet_no_op() {
    let harness = plain_harness(&["track-0", "track-1"], 1).await;
    let mut events = harness.api.session.subscribe();

    harness
        .api
        .queue_edit(QueueEdit::ClearUpcoming)
        .await
        .expect("nothing to clear is not an error");

    assert_eq!(queue_changes(&mut events), 0);
    assert_eq!(
        titles_and_position(&harness.api).await,
        (vec!["track-0".to_string(), "track-1".to_string()], Some(1))
    );

    let empty = self::harness(|_| {});
    empty
        .api
        .queue_edit(QueueEdit::ClearUpcoming)
        .await
        .expect("an empty queue has nothing upcoming");
    assert_eq!(titles_and_position(&empty.api).await, (Vec::new(), None));
}

#[tokio::test]
async fn replace_upcoming_swaps_the_tail_in_the_order_given() {
    let harness = plain_harness(&TEN, 2).await;
    let mut events = harness.api.session.subscribe();

    harness
        .api
        .queue_edit(QueueEdit::ReplaceUpcoming {
            keys: vec!["new-b".into(), "new-a".into()],
        })
        .await
        .expect("replace upcoming");

    let (titles, position) = titles_and_position(&harness.api).await;
    assert_eq!(
        titles,
        vec!["track-0", "track-1", "track-2", "new-b", "new-a"]
    );
    assert_eq!(position, Some(2));
    assert_eq!(queue_changes(&mut events), 1);
}

#[tokio::test]
async fn replace_upcoming_under_shuffle_plays_the_new_tracks_in_the_order_given() {
    let harness = shuffled_harness().await;
    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 2 })
        .await
        .expect("jump");
    let before = harness.api.queue_snapshot().await.expect("snapshot");
    let mut events = harness.api.session.subscribe();

    harness
        .api
        .queue_edit(QueueEdit::ReplaceUpcoming {
            keys: vec!["new-b".into(), "new-a".into(), "new-c".into()],
        })
        .await
        .expect("replace upcoming");

    let (titles, position) = titles_and_position(&harness.api).await;
    let mut expected: Vec<String> = before.items[..3].iter().map(|t| t.title.clone()).collect();
    expected.extend(["new-b", "new-a", "new-c"].map(String::from));
    assert_eq!(titles, expected);
    assert_eq!(position, Some(2));
    assert_eq!(queue_changes(&mut events), 1);

    let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
    let mut permutation = snapshot.shuffle_order.clone();
    permutation.sort_unstable();
    assert_eq!(permutation, (0..6).collect::<Vec<u32>>());

    shuffle_off(&harness.api).await;
    let (unshuffled, _) = titles_and_position(&harness.api).await;
    let mut kept: Vec<String> = before.items[..3].iter().map(|t| t.title.clone()).collect();
    kept.sort();
    kept.extend(["new-b", "new-a", "new-c"].map(String::from));
    assert_eq!(
        unshuffled, kept,
        "new tracks join the unshuffled queue at its end"
    );
}

#[tokio::test]
async fn replace_upcoming_with_no_keys_clears_and_with_nothing_upcoming_appends() {
    let harness = plain_harness(&["track-0", "track-1", "track-2"], 0).await;

    harness
        .api
        .queue_edit(QueueEdit::ReplaceUpcoming { keys: Vec::new() })
        .await
        .expect("empty replacement clears");
    assert_eq!(
        titles_and_position(&harness.api).await,
        (vec!["track-0".to_string()], Some(0))
    );
    let mut events = harness.api.session.subscribe();

    harness
        .api
        .queue_edit(QueueEdit::ReplaceUpcoming { keys: Vec::new() })
        .await
        .expect("nothing to clear");
    assert_eq!(queue_changes(&mut events), 0);

    harness
        .api
        .queue_edit(QueueEdit::ReplaceUpcoming {
            keys: vec!["next".into()],
        })
        .await
        .expect("nothing to replace, so the tracks are appended");
    assert_eq!(
        titles_and_position(&harness.api).await,
        (vec!["track-0".to_string(), "next".to_string()], Some(0))
    );
    assert_eq!(queue_changes(&mut events), 1);
}

#[tokio::test]
async fn shuffle_upcoming_reorders_only_what_follows_the_current_track() {
    let keys: Vec<String> = (0..20).map(|n| format!("track-{n}")).collect();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let harness = plain_harness(&keys, 4).await;
    let mut events = harness.api.session.subscribe();

    harness
        .api
        .queue_edit(QueueEdit::ShuffleUpcoming)
        .await
        .expect("shuffle upcoming");

    let (titles, position) = titles_and_position(&harness.api).await;
    assert_eq!(position, Some(4));
    assert_eq!(titles[..5], keys[..5], "history and current are untouched");
    assert_ne!(titles[5..], keys[5..], "the upcoming order changed");
    let mut rest = titles[5..].to_vec();
    rest.sort_by_key(|title| title[6..].parse::<u32>().expect("track number"));
    assert_eq!(rest, keys[5..], "the same tracks, reordered");
    assert_eq!(queue_changes(&mut events), 1);
}

#[tokio::test]
async fn shuffle_upcoming_under_shuffle_leaves_the_unshuffled_queue_alone() {
    let harness = shuffled_harness().await;
    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 1 })
        .await
        .expect("jump");
    let before = harness.api.queue_snapshot().await.expect("snapshot");
    let titles = |snapshot: &api::QueueSnapshot| -> Vec<String> {
        snapshot.items.iter().map(|t| t.title.clone()).collect()
    };
    let mut events = harness.api.session.subscribe();

    let mut changed = false;
    for _ in 0..5 {
        harness
            .api
            .queue_edit(QueueEdit::ShuffleUpcoming)
            .await
            .expect("shuffle upcoming");
        let after = harness.api.queue_snapshot().await.expect("snapshot");
        assert_eq!(titles(&after)[..2], titles(&before)[..2]);
        assert_eq!(after.position, Some(1));
        let mut rest = titles(&after)[2..].to_vec();
        rest.sort();
        let mut expected = titles(&before)[2..].to_vec();
        expected.sort();
        assert_eq!(rest, expected);
        for (position, physical) in after.shuffle_order.iter().enumerate() {
            assert_eq!(after.items[position].title, format!("track-{physical}"));
        }
        changed |= titles(&after) != titles(&before);
    }
    assert!(changed, "five shuffles of six tracks never moved anything");
    assert_eq!(queue_changes(&mut events), 5, "one QueueChanged per edit");

    shuffle_off(&harness.api).await;
    let (unshuffled, _) = titles_and_position(&harness.api).await;
    assert_eq!(
        unshuffled,
        (0..8).map(|n| format!("track-{n}")).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn shuffle_upcoming_with_fewer_than_two_upcoming_is_a_quiet_no_op() {
    let harness = plain_harness(&["track-0", "track-1", "track-2"], 1).await;
    let mut events = harness.api.session.subscribe();

    harness
        .api
        .queue_edit(QueueEdit::ShuffleUpcoming)
        .await
        .expect("one upcoming track has nothing to shuffle against");
    harness
        .api
        .queue_edit(QueueEdit::Jump { index: 2 })
        .await
        .expect("jump");
    let jumped = queue_changes(&mut events);
    harness
        .api
        .queue_edit(QueueEdit::ShuffleUpcoming)
        .await
        .expect("empty upcoming");

    assert_eq!(jumped, 1, "only the jump announced anything");
    assert_eq!(queue_changes(&mut events), 0);
    assert_eq!(
        queue_titles(&harness.api).await,
        vec!["track-0", "track-1", "track-2"]
    );

    let empty = self::harness(|_| {});
    empty
        .api
        .queue_edit(QueueEdit::ShuffleUpcoming)
        .await
        .expect("an empty queue has nothing upcoming");
}

#[tokio::test]
async fn upcoming_edits_during_a_crossfade_spare_the_track_fading_in() {
    let harness = harness(|config| config.playback.crossfade_seconds = 1);
    harness
        .api
        .set_queue(replace(&["track-0", "track-1", "track-2", "track-3"]))
        .await
        .expect("set queue");
    wait_committed(&harness.api).await;
    armed_crossfade(&harness).await;

    harness
        .api
        .queue_edit(QueueEdit::ReplaceUpcoming {
            keys: vec!["later".into()],
        })
        .await
        .expect("replace upcoming mid-crossfade");
    assert_eq!(
        queue_titles(&harness.api).await,
        vec!["track-0", "track-1", "later"]
    );

    let state = drive_until(&harness, "crossfade committed", |state| {
        state.queue.index == Some(1) && state.fading.is_none()
    })
    .await;
    assert_eq!(
        state.track.as_ref().map(|track| track.title.as_str()),
        Some("track-1")
    );
}

/// A station a search found is in no registry after a restart, so its row has to bring its own record back.
#[tokio::test]
async fn a_queued_station_survives_a_restart_without_the_registry() {
    use crate::persistence::QueueStore as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let database = db::init(&dir.path().join("queue.db")).await.expect("db");
    let store = Arc::new(crate::persistence::DbQueueStore::new(database));
    let source = config::Source::default();
    let searched = r#"{"schema_version":"1","id":"searched","name":"Searched FM","description":"","streams":[{"id":"main","name":"MP3","url":"https://radio.example.test/main.mp3"}],"metadata":null}"#;
    let mut queue: Vec<Track> = ["track-0", "radio:searched:main", "radio:gone:main"]
        .iter()
        .map(|key| test_track(&(*key).to_string()))
        .collect();
    for row in &mut queue[1..] {
        row.duration = u64::MAX;
    }
    store
        .save(
            &source,
            db::QueueSnapshot {
                version: 1,
                queue,
                current_queue_index: 1,
                stations: [("radio:searched:main".to_string(), searched.to_string())].into(),
                ..Default::default()
            },
        )
        .await;

    let player = Player::try_with_sink(Box::new(FakeSink(FakeSinkHandle::default())))
        .expect("headless player starts");
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
    let saved = store.load(&source).await.expect("load");
    session.restore_queue(saved).await.expect("restore");

    assert_eq!(queue_titles(&api).await, ["track-0", "radio:searched:main"]);
    let state = api.player_state().await.expect("state");
    assert_eq!(state.queue.index, Some(1));

    session.persist_now().await;
    let resaved = store.load(&source).await.expect("reload");
    assert_eq!(resaved.queue.len(), 2);
    let record: radio::manifest::StationManifest =
        serde_json::from_str(&resaved.stations["radio:searched:main"]).expect("record");
    assert_eq!(record.streams[0].url, "https://radio.example.test/main.mp3");
}

/// Records what a prefetch asked for, and answers with a few bytes.
#[derive(Default)]
struct AskedFor(Mutex<Vec<String>>);

#[async_trait::async_trait]
impl crate::offline_cache::Fetch for AskedFor {
    async fn fetch(
        &self,
        request: &crate::offline_cache::PrefetchRequest,
    ) -> Result<server::stream::stream_buffer::Captured, String> {
        self.0.lock().expect("asked").push(request.item_id.clone());
        Ok(server::stream::stream_buffer::Captured {
            bytes: b"audio".to_vec(),
            content_type: Some("audio/webm".into()),
        })
    }
}

/// The network is gone, so every prefetch fails.
struct Unreachable;

#[async_trait::async_trait]
impl crate::offline_cache::Fetch for Unreachable {
    async fn fetch(
        &self,
        _: &crate::offline_cache::PrefetchRequest,
    ) -> Result<server::stream::stream_buffer::Captured, String> {
        Err("unreachable".into())
    }
}

async fn cached_harness(
    fetcher: Arc<dyn crate::offline_cache::Fetch>,
) -> (Harness, crate::OfflineIndex, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = db::init(&dir.path().join("cache.db")).await.expect("db");
    let cache = crate::offline_cache::OfflineCache::load_with(
        &db,
        dir.path().join("cache"),
        &config::Settings::default(),
        fetcher,
    )
    .await
    .expect("cache");
    let offline = crate::OfflineIndex::default().with_cache(cache);
    let mut services = PlaybackServices {
        offline: offline.clone(),
        ..Default::default()
    };
    services.settings.playback.crossfade_seconds = 0;
    let harness = spawn_harness(
        services,
        Arc::new(|track| Some(wav_factory(track.duration.min(6)))),
    );
    harness
        .api
        .session
        .set_sources(active_on(config::Source::Server("yt".into())));
    (harness, offline, dir)
}

#[tokio::test]
async fn the_song_after_this_one_in_shuffled_play_order_is_fetched_ahead() {
    let fetcher = Arc::new(AskedFor::default());
    let (harness, offline, _dir) = cached_harness(fetcher.clone()).await;
    let keys = ["ytm-a", "ytm-b", "ytm-c", "ytm-d", "ytm-e", "ytm-f"];
    harness
        .api
        .set_queue(SetQueueRequest {
            shuffle: Some(true),
            ..replace(&keys)
        })
        .await
        .expect("queue");
    wait_committed(&harness.api).await;
    let snapshot = harness.api.queue_snapshot().await.expect("snapshot");
    assert!(snapshot.shuffle);
    let position = snapshot.position.expect("a position") as usize;
    let next = snapshot.items[position + 1].key.clone();

    let changed = offline.cache().changed();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !offline.cache().contains(&next) {
            changed.notified().await;
        }
    })
    .await
    .expect("the next song is kept");

    assert_eq!(
        *fetcher.0.lock().expect("asked"),
        std::slice::from_ref(&next)
    );
    let current = &snapshot.items[position].key;
    assert!(
        !offline.cache().contains(current),
        "nothing fetched it twice"
    );
    let rows = harness.api.queue_snapshot().await.expect("snapshot");
    assert!(rows.items[position + 1].cached, "its row says so");
}

#[tokio::test]
async fn offline_mode_queues_only_what_plays_offline_and_refuses_the_rest() {
    let (harness, offline, _dir) = cached_harness(Arc::new(AskedFor::default())).await;
    offline
        .cache()
        .store(
            "ytm-kept",
            "yt",
            server::stream::stream_buffer::Captured {
                bytes: b"audio".to_vec(),
                content_type: Some("audio/webm".into()),
            },
        )
        .await
        .expect("cached");
    offline.set_offline_mode(true);

    harness
        .api
        .set_queue(SetQueueRequest {
            start_index: Some(1),
            ..replace(&["ytm-gone", "ytm-kept", "/music/file.flac", "ytm-lost"])
        })
        .await
        .expect("queue");
    let queued: Vec<String> = harness
        .api
        .queue_snapshot()
        .await
        .expect("snapshot")
        .items
        .into_iter()
        .map(|row| row.key)
        .collect();
    assert_eq!(queued, ["ytm-kept", "/music/file.flac"]);

    let refused = harness
        .api
        .set_queue(replace(&["ytm-gone", "ytm-lost"]))
        .await
        .expect_err("nothing plays offline");
    assert_eq!(refused.reason, Some(api::ErrorReason::OfflineMode));
    assert_eq!(refused.code, ErrorCode::SourceUnreachable);

    offline.set_offline_mode(false);
    harness
        .api
        .set_queue(replace(&["ytm-gone", "ytm-lost"]))
        .await
        .expect("online again, everything queues");
}

#[tokio::test]
async fn offline_mode_refuses_a_jump_to_a_queued_song_without_a_copy() {
    let (harness, offline, _dir) = cached_harness(Arc::new(Unreachable)).await;
    harness
        .api
        .set_queue(replace(&["/music/first.flac", "ytm-gone"]))
        .await
        .expect("queued while online");
    wait_committed(&harness.api).await;
    offline.set_offline_mode(true);
    let before = harness.api.queue_snapshot().await.expect("snapshot");

    for edit in [
        QueueEdit::Jump { index: 1 },
        QueueEdit::JumpPhysical { index: 1 },
    ] {
        let refused = harness
            .api
            .queue_edit(edit.clone())
            .await
            .expect_err("the song has no copy");
        assert_eq!(
            refused.reason,
            Some(api::ErrorReason::OfflineMode),
            "{edit:?}"
        );
    }

    let after = harness.api.queue_snapshot().await.expect("snapshot");
    assert_eq!(after.position, before.position, "the pointer stays");
    assert_eq!(harness.api.player_state().await.expect("state").error, None);
}

#[tokio::test]
async fn offline_mode_refuses_a_chosen_song_without_a_copy_rather_than_play_another() {
    let (harness, offline, _dir) = cached_harness(Arc::new(Unreachable)).await;
    offline
        .cache()
        .store(
            "ytm-kept",
            "yt",
            server::stream::stream_buffer::Captured {
                bytes: b"audio".to_vec(),
                content_type: Some("audio/webm".into()),
            },
        )
        .await
        .expect("cached");
    offline.set_offline_mode(true);
    harness
        .api
        .set_queue(SetQueueRequest {
            start_index: Some(1),
            ..replace(&["/music/first.flac", "ytm-kept"])
        })
        .await
        .expect("a song with a copy plays");
    wait_committed(&harness.api).await;
    let before = harness.api.queue_snapshot().await.expect("snapshot");

    for (keys, start) in [
        (&["ytm-kept", "ytm-gone", "/music/next.flac"][..], 1),
        (&["ytm-gone", "ytm-kept"][..], 0),
        (&["ytm-kept", "ytm-gone"][..], 1),
    ] {
        let refused = harness
            .api
            .set_queue(SetQueueRequest {
                start_index: Some(start),
                ..replace(keys)
            })
            .await
            .expect_err("the chosen song has no copy");
        assert_eq!(
            refused.reason,
            Some(api::ErrorReason::OfflineMode),
            "{keys:?}"
        );
    }

    let after = harness.api.queue_snapshot().await.expect("snapshot");
    let keys = |snapshot: &api::QueueSnapshot| -> Vec<String> {
        snapshot.items.iter().map(|row| row.key.clone()).collect()
    };
    assert_eq!(keys(&after), keys(&before), "the queue is left as it was");
    assert_eq!(after.position, before.position, "nothing else started");
}

#[tokio::test]
async fn offline_mode_steps_over_queued_songs_without_a_copy() {
    // A fetch that never lands, so no prefetch gives the skipped song a copy.
    let (harness, offline, _dir) = cached_harness(Arc::new(Unreachable)).await;
    offline
        .cache()
        .store(
            "ytm-kept",
            "yt",
            server::stream::stream_buffer::Captured {
                bytes: b"audio".to_vec(),
                content_type: Some("audio/webm".into()),
            },
        )
        .await
        .expect("cached");
    harness
        .api
        .set_queue(replace(&["/music/first.flac", "ytm-gone", "ytm-kept"]))
        .await
        .expect("queue");
    wait_committed(&harness.api).await;
    offline.set_offline_mode(true);

    harness
        .api
        .player_command(PlayerCommand::Next)
        .await
        .expect("next");

    let state = wait_state(&harness.api, "the cached song", |state| {
        state
            .track
            .as_ref()
            .is_some_and(|track| track.key == "ytm-kept")
    })
    .await;
    assert_eq!(state.queue.index, Some(2), "ytm-gone was stepped over");
}
