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
    open_calls: usize,
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

    fn play_calls(&self) -> usize {
        self.0.lock().unwrap().play_calls
    }

    fn open_calls(&self) -> usize {
        self.0.lock().unwrap().open_calls
    }

    fn is_playing(&self) -> bool {
        self.0.lock().unwrap().playing
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
        shared.open_calls += 1;
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
    let (sink, engine, _tx) = spawn_engine_with_tx();
    (sink, engine)
}

fn spawn_engine_with_tx() -> (
    FakeSinkHandle,
    EngineHandle,
    std::sync::mpsc::Sender<super::actor::ActorMsg>,
) {
    let sink_handle = FakeSinkHandle::default();
    let for_actor = sink_handle.clone();
    let tx_slot: Arc<Mutex<Option<std::sync::mpsc::Sender<super::actor::ActorMsg>>>> =
        Arc::new(Mutex::new(None));
    let slot = tx_slot.clone();
    let engine = EngineHandle::spawn(move |tx| {
        *slot.lock().unwrap() = Some(tx.clone());
        Ok(Box::new(FakeSink(for_actor)) as Box<dyn AudioSink>)
    })
    .expect("engine spawn");
    let tx = tx_slot.lock().unwrap().take().expect("actor tx captured");
    (sink_handle, engine, tx)
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

/// A real cue-less WebM/Opus stream (matches YouTube's itag-774 structure):
/// symphonia's Matroska demuxer can't `seek` once it has read past EOF, which
/// is the seek-after-a-YT-track-ends repro.
fn webm_live_factory() -> (SourceFactory, Duration) {
    let bytes = include_bytes!("testdata/tone_live.webm").to_vec();
    let factory: SourceFactory =
        Box::new(move || Ok(crate::decoder::from_stream(std::io::Cursor::new(bytes))));
    (factory, Duration::from_secs(2))
}

fn load(engine: &EngineHandle, token: u64, factory: SourceFactory, duration: Duration) {
    let outcome = load_with(engine, token, factory, duration, Transition::Immediate);
    assert!(!outcome.crossfaded);
}

fn load_with(
    engine: &EngineHandle,
    token: u64,
    factory: SourceFactory,
    duration: Duration,
    transition: Transition,
) -> LoadOutcome {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    engine.send(Command::Load(LoadRequest {
        token,
        factory,
        duration,
        transition,
        start_at: None,
        reply: Some(reply_tx),
    }));
    reply_rx
        .blocking_recv()
        .expect("load reply")
        .expect("load ok")
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

    // Expect real samples (worker feeding the ring), then pull ~1s of audio;
    // the ring fills asynchronously, so keep pulling until position moved.
    wait_until("non-silent audio", || {
        sink.pull(4410).iter().any(|s| *s != 0.0)
    });
    let before = engine.status().position();
    wait_until("position advances ~1s with pulled audio", || {
        sink.pull(TEST_CONFIG.sample_rate as usize * TEST_CONFIG.channels / 4);
        engine.status().position() >= before + Duration::from_millis(900)
    });

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

/// A `Read + Seek` WAV source whose reads block while the gate is closed, so a
/// test can hold the decode worker at zero bytes written into a fresh ring.
struct GatedReader {
    inner: std::io::Cursor<Vec<u8>>,
    gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl std::io::Read for GatedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let (lock, cvar) = &*self.gate;
        let mut closed = lock.lock().unwrap();
        while *closed {
            closed = cvar.wait(closed).unwrap();
        }
        drop(closed);
        self.inner.read(buf)
    }
}

impl std::io::Seek for GatedReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn stale_eof_after_seek_does_not_end_session() {
    // A worker Eof that crosses a seek carries the pre-seek ring epoch. If the
    // actor honored it, it would re-latch `eof` on the freshly-seeked session
    // whose new ring reads played == written == 0, and the next tick would fire
    // a spurious Ended that auto-advances mid-track. The fix drops the stale Eof
    // by ring epoch.
    //
    // Determinism: the gate holds the worker inside a blocked read once closed,
    // so after the seek installs a fresh (empty) ring the worker cannot write to
    // it — played == written == 0 is pinned while the stale Eof is delivered.
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let frames = 5 * TEST_CONFIG.sample_rate as usize;
    let bytes = wav_bytes(frames, TEST_CONFIG.sample_rate, TEST_CONFIG.channels as u16);
    let reader_gate = gate.clone();
    let factory: SourceFactory = Box::new(move || {
        Ok(crate::decoder::from_stream(GatedReader {
            inner: std::io::Cursor::new(bytes),
            gate: reader_gate,
        }))
    });

    let (sink, engine, actor_tx) = spawn_engine_with_tx();
    let mut events = engine_subscribe(&engine);
    let mut seen = Vec::new();

    load(&engine, 1, factory, Duration::from_secs(5));
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("non-silent audio", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    // Close the gate, then seek: the worker will block before it can write any
    // post-seek audio, pinning the fresh ring at played == written == 0.
    *gate.0.lock().unwrap() = true;
    engine.send(Command::Seek(Duration::from_secs(1)));
    let _ = actor_tx.send(super::actor::ActorMsg::Worker(
        super::worker::WorkerMsg::Eof { token: 1, epoch: 0 },
    ));

    // Several ticks pass with the ring pinned empty; without the epoch guard the
    // stale Eof would end the session here (0 >= 0). It must not.
    std::thread::sleep(Duration::from_millis(300));
    drain_events(&mut events, &mut seen);
    assert!(
        !seen.iter().any(|e| matches!(e, Event::Ended { .. })),
        "a stale-epoch Eof must not end the seeked session: {seen:?}"
    );
    assert_eq!(
        engine.status().phase,
        Phase::Playing,
        "session must still be playing after a dropped stale Eof"
    );

    // Reopen the gate: the seeked session resumes and ends normally at its real
    // EOF, whose Eof carries the current epoch (1) and is honored.
    {
        *gate.0.lock().unwrap() = false;
        gate.1.notify_all();
    }
    wait_until("phase Ended at natural EOF", || {
        sink.pull(4096);
        engine.status().phase == Phase::Ended
    });
    drain_events(&mut events, &mut seen);
    assert_eq!(
        seen.iter()
            .filter(|e| matches!(e, Event::Ended { token: 1 }))
            .count(),
        1,
        "exactly one Ended for the natural end: {seen:?}"
    );

    engine.shutdown();
}

#[test]
fn seek_after_eof_reprobes_webm_and_resumes() {
    // WebM/Opus (all YouTube audio): symphonia's Matroska demuxer errors on a
    // seek once the reader has passed EOF ("element is not an ancestor"), so
    // the parked-worker revive must re-probe from the buffered bytes. Without
    // that, the seek yields silence.
    let (sink, engine) = spawn_engine();

    let (factory, duration) = webm_live_factory();
    load(&engine, 11, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    wait_until("phase Ended", || {
        sink.pull(4096);
        engine.status().phase == Phase::Ended
    });

    engine.send(Command::Seek(Duration::from_millis(500)));
    wait_until("audio after webm seek-revive", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}

#[test]
fn seek_after_end_of_queue_pause_resumes_playback() {
    // End-of-queue: the track drains to Ended, then the controller pauses the
    // device to stop the idle stream while keeping the parked worker alive.
    // Scrubbing back into the track must resume playback, not sit silently
    // paused at the seek position.
    let (sink, engine) = spawn_engine();

    let (factory, duration) = wav_factory(0.25);
    load(&engine, 9, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    wait_until("phase Ended", || {
        sink.pull(4096);
        engine.status().phase == Phase::Ended
    });

    // The end-of-queue pause (player_controller_queue.rs) quiesces the device;
    // the `ended` latch keeps phase == Ended, so the pause only shows up as a
    // device pause_calls bump.
    let pauses_before = sink.pause_calls();
    engine.send(Command::Pause);
    wait_until("device paused at end of queue", || {
        sink.pause_calls() > pauses_before
    });
    let plays_before = sink.play_calls();

    engine.send(Command::Seek(Duration::from_millis(50)));
    wait_until("phase Playing after seek-revive", || {
        engine.status().phase == Phase::Playing
    });
    assert!(
        sink.play_calls() > plays_before,
        "seek out of a paused-ended track must resume the device"
    );
    wait_until("audio after revive", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

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
    let (reply_tx, mut reply_rx) = tokio::sync::oneshot::channel();
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
    // Cancellation resolves as a dropped reply channel, not an error.
    wait_until("superseded reply dropped", || {
        matches!(
            reply_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        )
    });

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

fn try_load_with(
    engine: &EngineHandle,
    token: u64,
    factory: SourceFactory,
    duration: Duration,
    transition: Transition,
) -> Result<LoadOutcome, String> {
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    engine.send(Command::Load(LoadRequest {
        token,
        factory,
        duration,
        transition,
        start_at: None,
        reply: Some(reply_tx),
    }));
    reply_rx.blocking_recv().expect("load reply")
}

#[test]
fn factory_error_reports_and_keeps_prior_audio() {
    let (sink, engine) = spawn_engine();
    let mut events = engine_subscribe(&engine);

    let (factory, duration) = wav_factory(10.0);
    load(&engine, 1, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    let broken: SourceFactory = Box::new(|| Err("boom".to_string()));
    let result = try_load_with(
        &engine,
        2,
        broken,
        Duration::from_secs(1),
        Transition::Immediate,
    );
    assert!(result.is_err(), "broken factory must fail the load");

    // Prior session is untouched: still token 1, still playing real audio.
    assert_eq!(engine.status().token, 1);
    assert_eq!(engine.status().phase, Phase::Playing);
    wait_until("audio from token 1", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });
    let mut seen = Vec::new();
    drain_events(&mut events, &mut seen);
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::Error { token: 2, .. })),
        "Error event carries the failed token: {seen:?}"
    );

    engine.shutdown();
}

#[test]
fn seek_moves_position_immediately_on_fresh_counters() {
    let (sink, engine) = spawn_engine();
    let (factory, duration) = wav_factory(10.0);
    load(&engine, 1, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("audio flowing", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    let target = Duration::from_secs(3);
    engine.send(Command::Seek(target));
    wait_until("position jumps to the seek target", || {
        engine.status().position() == target
    });

    // Playback continues from the fresh ring.
    wait_until("audio after seek", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });
    assert!(engine.status().position() > target);

    engine.shutdown();
}

#[test]
fn crossfade_while_paused_falls_back_and_stays_paused() {
    let (sink, engine) = spawn_engine();
    let (factory_a, duration_a) = wav_factory(10.0);
    load(&engine, 1, factory_a, duration_a);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    engine.send(Command::Pause);
    wait_until("phase Paused", || engine.status().phase == Phase::Paused);

    let (factory_b, duration_b) = wav_factory(10.0);
    let outcome = load_with(
        &engine,
        2,
        factory_b,
        duration_b,
        Transition::Crossfade(Duration::from_secs(1)),
    );
    assert!(!outcome.crossfaded, "paused crossfade must fall back");
    assert_eq!(engine.status().phase, Phase::Paused, "pause is honored");
    assert!(!sink.is_playing(), "device stays paused");
    for _ in 0..5 {
        assert!(sink.pull(4096).iter().all(|s| *s == 0.0));
    }

    engine.send(Command::Resume);
    wait_until("phase Playing after resume", || {
        engine.status().phase == Phase::Playing
    });
    wait_until("audio from the new track", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}

#[test]
fn crossfade_on_idle_engine_falls_back_to_immediate() {
    let (sink, engine) = spawn_engine();
    let (factory, duration) = wav_factory(5.0);
    let outcome = load_with(
        &engine,
        1,
        factory,
        duration,
        Transition::Crossfade(Duration::from_secs(2)),
    );
    assert!(!outcome.crossfaded, "nothing to fade from");
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("audio flowing", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });
    engine.shutdown();
}

#[test]
fn seek_during_crossfade_resumes_the_outgoing_track() {
    let (sink, engine) = spawn_engine();
    let mut events = engine_subscribe(&engine);

    let (factory_a, duration_a) = wav_factory(30.0);
    load(&engine, 1, factory_a, duration_a);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("audio from A", || sink.pull(4096).iter().any(|s| *s != 0.0));

    let (factory_b, duration_b) = wav_factory(30.0);
    let outcome = load_with(
        &engine,
        2,
        factory_b,
        duration_b,
        Transition::Crossfade(Duration::from_secs(5)),
    );
    assert!(outcome.crossfaded);
    wait_until("status on incoming token 2", || engine.status().token == 2);

    // A seek mid-crossfade targets the outgoing (visible) track: the engine
    // promotes the outgoing session (token 1) back to active, cancels the fade,
    // and seeks it in place — no re-load, and no TrackSwitched.
    engine.send(Command::Seek(Duration::from_secs(10)));
    wait_until("outgoing token 1 restored at the seek target", || {
        let status = engine.status();
        status.token == 1 && status.position() >= Duration::from_secs(10)
    });
    let mut seen = Vec::new();
    for _ in 0..80 {
        sink.pull(8192);
        std::thread::sleep(Duration::from_millis(5));
    }
    drain_events(&mut events, &mut seen);
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, Event::TrackSwitched { .. })),
        "fade cancelled, no TrackSwitched expected: {seen:?}"
    );
    assert_eq!(engine.status().token, 1);
    assert_eq!(engine.status().phase, Phase::Playing);
    wait_until("audio after seek", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}

#[test]
fn seek_during_crossfade_from_radio_is_ignored() {
    // Seeking out of a crossfade whose outgoing (visible) source is non-seekable
    // (radio) must be a no-op. The old ordering retired the incoming session and
    // only then early-returned on !seekable, stranding the RT mid-fade into a
    // stopped ring — silence with the status wedged. The fix checks the visible
    // session's seekability before any teardown.
    let (sink, engine) = spawn_engine();
    let mut events = engine_subscribe(&engine);

    // Outgoing: a long non-seekable stream, like internet radio.
    let frames = 30 * TEST_CONFIG.sample_rate as usize;
    let bytes = wav_bytes(frames, TEST_CONFIG.sample_rate, TEST_CONFIG.channels as u16);
    let radio: SourceFactory = Box::new(move || {
        Ok(crate::decoder::from_stream_with_hint(
            std::io::Cursor::new(bytes),
            "wav",
        ))
    });
    load(&engine, 1, radio, Duration::from_secs(u64::MAX / 2));
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("audio from radio", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    // Incoming: a seekable file, crossfaded in.
    let (factory_b, duration_b) = wav_factory(30.0);
    let outcome = load_with(
        &engine,
        2,
        factory_b,
        duration_b,
        Transition::Crossfade(Duration::from_secs(5)),
    );
    assert!(outcome.crossfaded);
    wait_until("status on incoming token 2", || engine.status().token == 2);

    // Seek mid-fade — ignored, because the visible (outgoing) source is radio.
    engine.send(Command::Seek(Duration::from_secs(10)));

    // The fade runs to completion normally: TrackSwitched for the incoming, which
    // becomes the sole session; audio never goes silent.
    let mut seen = Vec::new();
    wait_until("fade completes to token 2", || {
        sink.pull(8192);
        drain_events(&mut events, &mut seen);
        seen.iter()
            .any(|e| matches!(e, Event::TrackSwitched { token: 2, .. }))
    });
    assert_eq!(
        engine.status().token,
        2,
        "the incoming session must survive the ignored seek"
    );
    assert_eq!(engine.status().phase, Phase::Playing);
    wait_until("audio still flowing after the ignored seek", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}

#[test]
fn radio_source_ignores_seek_and_ends_at_eof() {
    let (sink, engine) = spawn_engine();

    // Non-seekable source, like internet radio.
    let frames = (0.25 * TEST_CONFIG.sample_rate as f64) as usize;
    let bytes = wav_bytes(frames, TEST_CONFIG.sample_rate, TEST_CONFIG.channels as u16);
    let factory: SourceFactory = Box::new(move || {
        Ok(crate::decoder::from_stream_with_hint(
            std::io::Cursor::new(bytes),
            "wav",
        ))
    });
    // Radio uses the u64::MAX duration sentinel.
    load(&engine, 1, factory, Duration::from_secs(u64::MAX / 2));
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("audio flowing", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    let before = engine.status().position();
    engine.send(Command::Seek(Duration::from_secs(60)));
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        engine.status().position() < before + Duration::from_secs(30),
        "seek on a non-seekable source must be ignored"
    );

    wait_until("stream end drains to Ended", || {
        sink.pull(8192);
        engine.status().phase == Phase::Ended
    });

    engine.shutdown();
}

#[test]
fn device_error_rebuilds_stream_and_resumes_position() {
    let (sink, engine, actor_tx) = spawn_engine_with_tx();
    let (factory, duration) = wav_factory(30.0);
    load(&engine, 1, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    // Play a couple of seconds in (the ring fills asynchronously).
    wait_until("played two seconds", || {
        sink.pull(TEST_CONFIG.sample_rate as usize * TEST_CONFIG.channels);
        engine.status().position() >= Duration::from_secs(2)
    });
    let position_before = engine.status().position();
    let opens_before = sink.open_calls();

    let _ = actor_tx.send(super::actor::ActorMsg::DeviceError);

    wait_until("stream rebuilt", || sink.open_calls() > opens_before);
    wait_until("still playing after rebuild", || {
        engine.status().phase == Phase::Playing
    });
    // Position resumed near where the device died (seek-to-current protocol).
    let resumed = engine.status().position();
    assert!(
        resumed >= position_before.saturating_sub(Duration::from_millis(500)),
        "position must survive the rebuild: {position_before:?} -> {resumed:?}"
    );
    wait_until("audio after rebuild", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}

#[test]
fn full_stop_pauses_the_device() {
    let (sink, engine) = spawn_engine();
    let (factory, duration) = wav_factory(5.0);
    load(&engine, 1, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);

    engine.send(Command::Stop { pause_device: true });
    wait_until("phase Idle", || engine.status().phase == Phase::Idle);
    assert!(!sink.is_playing(), "full stop pauses the device");

    engine.shutdown();
}

#[test]
fn default_device_change_migrates_seekable_session() {
    let (sink, engine, actor_tx) = spawn_engine_with_tx();
    let (factory, duration) = wav_factory(30.0);
    load(&engine, 1, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("played one second", || {
        sink.pull(TEST_CONFIG.sample_rate as usize * TEST_CONFIG.channels);
        engine.status().position() >= Duration::from_secs(1)
    });
    let position_before = engine.status().position();
    let opens_before = sink.open_calls();

    let _ = actor_tx.send(super::actor::ActorMsg::DefaultDeviceChanged);

    wait_until("stream rebuilt on the new default", || {
        sink.open_calls() > opens_before
    });
    wait_until("still playing after migration", || {
        engine.status().phase == Phase::Playing
    });
    assert!(
        engine.status().position() >= position_before.saturating_sub(Duration::from_millis(500)),
        "position survives the migration"
    );
    wait_until("audio after migration", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}

#[test]
fn default_device_change_leaves_radio_alone() {
    let (sink, engine, actor_tx) = spawn_engine_with_tx();

    let frames = TEST_CONFIG.sample_rate as usize * 10;
    let bytes = wav_bytes(frames, TEST_CONFIG.sample_rate, TEST_CONFIG.channels as u16);
    let factory: SourceFactory = Box::new(move || {
        Ok(crate::decoder::from_stream_with_hint(
            std::io::Cursor::new(bytes),
            "wav",
        ))
    });
    load(&engine, 1, factory, Duration::from_secs(u64::MAX / 2));
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("audio flowing", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });
    let opens_before = sink.open_calls();

    let _ = actor_tx.send(super::actor::ActorMsg::DefaultDeviceChanged);
    std::thread::sleep(Duration::from_millis(400));

    assert_eq!(
        sink.open_calls(),
        opens_before,
        "a live stream must not be migrated (it cannot re-seek)"
    );
    assert_eq!(engine.status().phase, Phase::Playing);
    wait_until("radio audio continues", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}

#[test]
fn device_change_pause_behavior_holds_after_migration() {
    let (sink, engine, actor_tx) = spawn_engine_with_tx();
    engine.send(Command::SetDeviceChangeBehavior(
        config::DeviceChangeBehavior::Pause,
    ));

    let (factory, duration) = wav_factory(30.0);
    load(&engine, 1, factory, duration);
    wait_until("phase Playing", || engine.status().phase == Phase::Playing);
    wait_until("played one second", || {
        sink.pull(TEST_CONFIG.sample_rate as usize * TEST_CONFIG.channels);
        engine.status().position() >= Duration::from_secs(1)
    });
    let position_before = engine.status().position();
    let opens_before = sink.open_calls();

    let _ = actor_tx.send(super::actor::ActorMsg::DeviceError);

    wait_until("stream rebuilt", || sink.open_calls() > opens_before);
    wait_until("held paused on the new device", || {
        engine.status().phase == Phase::Paused
    });
    assert!(!sink.is_playing(), "device stays paused");
    assert!(
        engine.status().position() >= position_before.saturating_sub(Duration::from_millis(500)),
        "position preserved while paused"
    );

    // Resume continues where the migration left off.
    engine.send(Command::Resume);
    wait_until("phase Playing after resume", || {
        engine.status().phase == Phase::Playing
    });
    wait_until("audio after resume", || {
        sink.pull(4096).iter().any(|s| *s != 0.0)
    });

    engine.shutdown();
}
