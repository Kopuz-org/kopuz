use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::sink::{AudioSink, DataCallback, DataCallbackFactory, SinkConfig};
use super::*;

const TEST_CONFIG: SinkConfig = SinkConfig {
    channels: 2,
    sample_rate: 44_100,
};

#[derive(Default)]
struct FakeShared {
    callback: Option<DataCallback>,
    opened: Option<SinkConfig>,
    playing: bool,
    play_calls: usize,
    pause_calls: usize,
}

#[derive(Clone, Default)]
struct FakeSinkHandle(Arc<Mutex<FakeShared>>);

impl FakeSinkHandle {
    /// Drive the "audio callback" like a device would: ask for `samples`
    /// interleaved f32 samples. Returns silence while paused, like cpal after
    /// `stream.pause()`.
    fn pull(&self, samples: usize) -> Vec<f32> {
        let mut buffer = vec![0.0f32; samples];
        let mut shared = self.0.lock().unwrap();
        if shared.playing
            && let Some(callback) = shared.callback.as_mut()
        {
            callback(&mut buffer);
        }
        buffer
    }

    fn pause_calls(&self) -> usize {
        self.0.lock().unwrap().pause_calls
    }
}

struct FakeSink(FakeSinkHandle);

impl AudioSink for FakeSink {
    fn probe_config(&mut self, _desired_sample_rate: Option<u32>) -> Result<SinkConfig, String> {
        Ok(TEST_CONFIG)
    }

    fn open(
        &mut self,
        _desired_sample_rate: Option<u32>,
        make_cb: DataCallbackFactory,
    ) -> Result<SinkConfig, String> {
        let callback = make_cb(TEST_CONFIG);
        let mut shared = self.0.0.lock().unwrap();
        shared.callback = Some(callback);
        shared.opened = Some(TEST_CONFIG);
        shared.playing = true;
        Ok(TEST_CONFIG)
    }

    fn config(&self) -> Option<SinkConfig> {
        self.0.0.lock().unwrap().opened
    }

    fn play(&mut self) -> Result<(), String> {
        let mut shared = self.0.0.lock().unwrap();
        shared.playing = true;
        shared.play_calls += 1;
        Ok(())
    }

    fn pause(&mut self) {
        let mut shared = self.0.0.lock().unwrap();
        shared.playing = false;
        shared.pause_calls += 1;
    }

    fn close(&mut self) {
        let mut shared = self.0.0.lock().unwrap();
        shared.callback = None;
        shared.opened = None;
        shared.playing = false;
    }
}

fn spawn_engine() -> (FakeSinkHandle, EngineHandle) {
    let sink_handle = FakeSinkHandle::default();
    let for_actor = sink_handle.clone();
    let engine =
        EngineHandle::spawn(move |_tx| Ok(Box::new(FakeSink(for_actor)) as Box<dyn AudioSink>))
            .expect("engine spawn");
    (sink_handle, engine)
}

/// Minimal 16-bit PCM WAV with a deterministic non-zero pattern.
fn wav_bytes(frames: usize, sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = frames * channels as usize * 2;
    let mut v = Vec::with_capacity(44 + data_len);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    v.extend_from_slice(&(channels * 2).to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..frames {
        let value = (((i % 100) as i16) + 1) * 100;
        for _ in 0..channels {
            v.extend_from_slice(&value.to_le_bytes());
        }
    }
    v
}

fn wav_factory(seconds: f64) -> (SourceFactory, Duration) {
    let frames = (seconds * TEST_CONFIG.sample_rate as f64) as usize;
    let bytes = wav_bytes(frames, TEST_CONFIG.sample_rate, TEST_CONFIG.channels as u16);
    let factory: SourceFactory =
        Box::new(move || Ok(crate::decoder::from_stream(std::io::Cursor::new(bytes))));
    (factory, Duration::from_secs_f64(seconds))
}

fn load(engine: &EngineHandle, token: u64, factory: SourceFactory, duration: Duration) {
    load_with(engine, token, factory, duration, Transition::Immediate);
}

fn load_with(
    engine: &EngineHandle,
    token: u64,
    factory: SourceFactory,
    duration: Duration,
    transition: Transition,
) {
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    engine.send(Command::Load(LoadRequest {
        token,
        factory,
        duration,
        transition,
        start_at: None,
        reply: Some(reply_tx),
    }));
    reply_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("load reply")
        .expect("load ok");
}

fn wait_until(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for: {what}");
}

fn drain_events(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Event>, into: &mut Vec<Event>) {
    while let Ok(event) = rx.try_recv() {
        into.push(event);
    }
}

#[test]
fn load_plays_and_position_advances() {
    let (sink, engine) = spawn_engine();
    let (factory, duration) = wav_factory(5.0);
    load(&engine, 1, factory, duration);

    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    // Pull one second of audio; expect real samples (worker feeding the ring).
    wait_until("non-silent audio", || {
        sink.pull(4410).iter().any(|s| *s != 0.0)
    });
    let before = engine.status().position();
    sink.pull(TEST_CONFIG.sample_rate as usize * TEST_CONFIG.channels);
    let after = engine.status().position();
    assert!(
        after >= before + Duration::from_millis(900),
        "position should advance ~1s with pulled audio: {before:?} -> {after:?}"
    );

    engine.shutdown();
}

#[test]
fn eof_emits_ended_once_and_seek_revives() {
    let (sink, engine) = spawn_engine();
    let mut events = engine_subscribe(&engine);
    let mut seen = Vec::new();

    let (factory, duration) = wav_factory(0.25);
    load(&engine, 7, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    // Drain the whole track.
    wait_until("phase Ended", || {
        sink.pull(4096);
        engine.status().phase == Phase::Ended
    });

    // Keep pulling; Ended must not fire again.
    for _ in 0..10 {
        sink.pull(4096);
        std::thread::sleep(Duration::from_millis(20));
    }
    drain_events(&mut events, &mut seen);
    let ended_count = seen
        .iter()
        .filter(|e| matches!(e, Event::Ended { token: 7 }))
        .count();
    assert_eq!(ended_count, 1, "Ended must fire exactly once: {seen:?}");

    // The 4aedd347 scenario: seek after natural end must revive playback.
    engine.send(Command::Seek(Duration::from_millis(50)));
    wait_until("phase Playing after seek-revive", || {
        engine.status().phase == Phase::Playing
    });
    wait_until("audio after revive", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    // And it can end again.
    wait_until("second Ended", || {
        sink.pull(4096);
        engine.status().phase == Phase::Ended
    });
    drain_events(&mut events, &mut seen);
    let ended_count = seen
        .iter()
        .filter(|e| matches!(e, Event::Ended { token: 7 }))
        .count();
    assert_eq!(
        ended_count, 2,
        "revived session may end once more: {seen:?}"
    );

    engine.shutdown();
}

#[test]
fn pause_freezes_drain_and_blocks_ended() {
    let (sink, engine) = spawn_engine();
    let (factory, duration) = wav_factory(0.25);
    load(&engine, 3, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    engine.send(Command::Pause);
    wait_until("phase Paused", || engine.status().phase == Phase::Paused);
    assert!(sink.pause_calls() >= 1, "device must be paused");

    // Paused: pulls yield silence and the track must not drain to Ended.
    let position = engine.status().position();
    for _ in 0..10 {
        assert!(sink.pull(4096).iter().all(|s| *s == 0.0));
    }
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(engine.status().position(), position);
    assert_ne!(engine.status().phase, Phase::Ended);

    engine.send(Command::Resume);
    wait_until("phase Playing after resume", || {
        engine.status().phase == Phase::Playing
    });
    wait_until("phase Ended after resume", || {
        sink.pull(4096);
        engine.status().phase == Phase::Ended
    });

    engine.shutdown();
}

#[test]
fn crossfade_mixes_and_emits_track_switched() {
    let (sink, engine) = spawn_engine();
    let mut events = engine_subscribe(&engine);
    let mut seen = Vec::new();

    let (factory_a, duration_a) = wav_factory(30.0);
    load(&engine, 1, factory_a, duration_a);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("audio from A", || sink.pull(4096).iter().any(|s| *s != 0.0));

    let (factory_b, duration_b) = wav_factory(30.0);
    load_with(
        &engine,
        2,
        factory_b,
        duration_b,
        Transition::Crossfade(Duration::from_millis(500)),
    );
    wait_until("status switches to token 2", || engine.status().token == 2);

    // Pull well past the fade length; the actor should observe fade completion
    // and emit TrackSwitched.
    wait_until("TrackSwitched", || {
        sink.pull(8192);
        drain_events(&mut events, &mut seen);
        seen.iter()
            .any(|e| matches!(e, Event::TrackSwitched { token: 2 }))
    });

    assert_eq!(engine.status().phase, Phase::Playing);
    engine.shutdown();
}

#[test]
fn stop_for_transition_goes_idle_without_pausing_device() {
    let (sink, engine) = spawn_engine();
    let (factory, duration) = wav_factory(5.0);
    load(&engine, 1, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    let pauses_before = sink.pause_calls();
    engine.send(Command::Stop {
        pause_device: false,
    });
    wait_until("phase Idle", || engine.status().phase == Phase::Idle);
    assert_eq!(engine.status().position(), Duration::ZERO);
    assert_eq!(sink.pause_calls(), pauses_before, "device keeps running");

    engine.shutdown();
}

fn engine_subscribe(engine: &EngineHandle) -> tokio::sync::mpsc::UnboundedReceiver<Event> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    engine.send(Command::Subscribe(tx));
    rx
}

#[test]
fn superseding_load_drops_stale_session() {
    let (sink, engine) = spawn_engine();

    // First load's factory blocks until released — a probe stuck on network.
    let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
    let frames = TEST_CONFIG.sample_rate as usize;
    let bytes = wav_bytes(frames, TEST_CONFIG.sample_rate, TEST_CONFIG.channels as u16);
    let slow_factory: SourceFactory = Box::new(move || {
        let _ = gate_rx.recv();
        Ok(crate::decoder::from_stream(std::io::Cursor::new(bytes)))
    });
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    engine.send(Command::Load(LoadRequest {
        token: 1,
        factory: slow_factory,
        duration: Duration::from_secs(1),
        transition: Transition::Immediate,
        start_at: None,
        reply: Some(reply_tx),
    }));

    // Supersede while the first is still "probing".
    let (factory, duration) = wav_factory(5.0);
    load(&engine, 2, factory, duration);
    assert!(
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reply")
            .is_err(),
        "superseded load must resolve as an error"
    );

    // Release the stale worker; the engine must stay on token 2.
    let _ = gate_tx.send(());
    wait_until("playing token 2", || {
        let status = engine.status();
        status.token == 2 && status.phase == Phase::Playing
    });
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(engine.status().token, 2);
    wait_until("audio from token 2", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}
