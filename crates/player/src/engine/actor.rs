//! The engine actor: one OS thread that owns the sink, the decode workers and
//! all session state. Commands are processed FIFO; the ~100ms tick derives
//! drain/fade completion from atomics the RT callback publishes and emits
//! token-tagged events.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use config::{ChannelMode, EqualizerSettings};

use super::rt::{Retired, RtCmd, RtSession, RtState};
use super::sink::{AudioSink, DataCallbackFactory, SinkConfig};
use super::worker::{WorkerCmd, WorkerHandle, WorkerMsg};
use super::{
    Command, EngineStatus, Event, FadingStatus, LoadOutcome, LoadReply, LoadRequest, Phase,
    Transition,
};
use crate::player::PlayerInitError;
#[cfg(any(target_os = "android", target_os = "linux", target_os = "macos"))]
use crate::systemint;

const TICK: Duration = Duration::from_millis(100);
const SEEK_END_GUARD: Duration = Duration::from_millis(2000);

/// Ring buffer length between the decode worker and the audio callback.
/// - Desktop: 2s — plenty of headroom for big seeks and metadata stalls.
/// - Android: 1s — smaller heap footprint matters on phones with 2-3GB RAM,
///   and a smaller buffer recovers from underruns faster.
#[cfg(target_os = "android")]
const RING_BUF_SECONDS: usize = 1;
#[cfg(not(target_os = "android"))]
const RING_BUF_SECONDS: usize = 2;

pub(crate) enum ActorMsg {
    Cmd(Command),
    Worker(WorkerMsg),
    DeviceError,
    /// The OS default output changed; migrate unless that would kill a live
    /// (non-seekable) stream.
    DefaultDeviceChanged,
}

pub struct EngineHandle {
    tx: Sender<ActorMsg>,
    status: Arc<ArcSwap<EngineStatus>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl EngineHandle {
    /// Spawn the actor thread. `make_sink` runs on that thread (the sink and
    /// its streams live there); spawn blocks until the sink exists so init
    /// errors surface synchronously like the old constructor.
    pub(crate) fn spawn<F>(make_sink: F) -> Result<Self, PlayerInitError>
    where
        F: FnOnce(&Sender<ActorMsg>) -> Result<Box<dyn AudioSink>, PlayerInitError>
            + Send
            + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let status = Arc::new(ArcSwap::from_pointee(EngineStatus::idle()));
        let (init_tx, init_rx) = std::sync::mpsc::channel();

        let actor_tx = tx.clone();
        let actor_status = status.clone();
        let join = std::thread::Builder::new()
            .name("kopuz-player-engine".to_string())
            .spawn(move || {
                let sink = match make_sink(&actor_tx) {
                    Ok(sink) => {
                        let _ = init_tx.send(Ok(()));
                        sink
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };
                Actor::new(rx, actor_tx, sink, actor_status).run();
            })
            .map_err(PlayerInitError::EngineThread)?;

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                tx,
                status,
                join: Some(join),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(PlayerInitError::NoOutputDevice),
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(ActorMsg::Cmd(command));
    }

    pub fn status(&self) -> Arc<EngineStatus> {
        self.status.load_full()
    }

    /// Shut down and wait for the actor (and its workers) to exit.
    pub fn shutdown(mut self) {
        self.send(Command::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        // Fire-and-forget: the actor tears itself down; joining here would
        // block the UI thread on app exit.
        let _ = self.tx.send(ActorMsg::Cmd(Command::Shutdown));
    }
}

struct Session {
    token: u64,
    worker: WorkerHandle,
    written: Arc<AtomicU64>,
    played: Arc<AtomicU64>,
    base_micros: u64,
    duration: Duration,
    seekable: bool,
    source_sample_rate: Option<u32>,
    eof: bool,
    ended: bool,
    /// Ring generation, bumped on every seek. The worker echoes it on `Eof`
    /// so an `Eof` in flight across a seek is ignored (it targets the ring the
    /// seek already replaced) instead of ending the freshly-seeked session.
    ring_epoch: u64,
}

/// Everything needed to start a probed source, independent of the reply.
struct StartPlan {
    token: u64,
    worker: WorkerHandle,
    duration: Duration,
    transition: Transition,
    start_at: Option<Duration>,
}

struct Pending {
    plan: StartPlan,
    reply: Option<LoadReply>,
}

/// Producer/consumer halves of a fresh session ring plus its counters.
struct RingParts {
    producer: rtrb::Producer<f32>,
    written: Arc<AtomicU64>,
    played: Arc<AtomicU64>,
    rt_session: RtSession,
}

fn make_ring(config: SinkConfig) -> RingParts {
    let size = (config.sample_rate as usize * config.channels * RING_BUF_SECONDS).max(1);
    let (producer, consumer) = rtrb::RingBuffer::new(size);
    let written = Arc::new(AtomicU64::new(0));
    let played = Arc::new(AtomicU64::new(0));
    RingParts {
        producer,
        written,
        played: played.clone(),
        rt_session: RtSession { consumer, played },
    }
}

struct Actor {
    rx: Receiver<ActorMsg>,
    self_tx: Sender<ActorMsg>,
    sink: Box<dyn AudioSink>,
    status: Arc<ArcSwap<EngineStatus>>,
    /// Event subscribers. Multiple consumers may subscribe (the UI pump plus,
    /// e.g., a future MPRIS or presence hook); a sender whose receiver has been
    /// dropped is pruned on the next emit.
    events: Vec<tokio::sync::mpsc::UnboundedSender<Event>>,

    volume: Arc<AtomicU32>,
    paused: Arc<AtomicBool>,
    eq_settings: EqualizerSettings,
    channel_mode: ChannelMode,
    device_change_behavior: config::DeviceChangeBehavior,

    rt_tx: Option<Sender<RtCmd>>,
    retire_rx: Option<Receiver<Retired>>,

    current: Option<Session>,
    pending: Option<Pending>,
    /// The outgoing crossfade session, kept whole (not just its worker) so a
    /// seek mid-fade can cancel the fade and resume it in place. Its consumer
    /// lives in the RT callback until the fade completes or is killed.
    fading: Option<Session>,
    /// Detached workers (superseded probes, stopped sessions) awaiting exit.
    /// Never joined on the command path — a worker stuck in network I/O must
    /// not stall the actor.
    graveyard: Vec<std::thread::JoinHandle<()>>,
    last_phase: Phase,
    last_token: u64,
    last_position_emitted: Option<(u64, u64)>,
    last_output_rebuild: Option<Instant>,
    /// Ring consumers handed to the current RT (via Swap) not yet shipped back
    /// (via Retired). Zero ⇒ nothing left to reap, so the loop may park.
    rt_rings_outstanding: usize,
    shutting_down: bool,
}

impl Actor {
    fn new(
        rx: Receiver<ActorMsg>,
        self_tx: Sender<ActorMsg>,
        sink: Box<dyn AudioSink>,
        status: Arc<ArcSwap<EngineStatus>>,
    ) -> Self {
        Self {
            rx,
            self_tx,
            sink,
            status,
            events: Vec::new(),
            volume: Arc::new(AtomicU32::new(super::rt::volume_bits(1.0))),
            paused: Arc::new(AtomicBool::new(false)),
            eq_settings: EqualizerSettings::default(),
            channel_mode: ChannelMode::Stereo,
            device_change_behavior: config::DeviceChangeBehavior::Resume,
            rt_tx: None,
            retire_rx: None,
            current: None,
            pending: None,
            fading: None,
            graveyard: Vec::new(),
            last_phase: Phase::Idle,
            last_token: 0,
            last_position_emitted: None,
            last_output_rebuild: None,
            rt_rings_outstanding: 0,
            shutting_down: false,
        }
    }

    fn run(mut self) {
        // Open the output up front so the pipeline is warm (silence until the
        // first Load), matching the old constructor's behavior.
        if let Err(e) = self.open_output(None) {
            tracing::error!(error = %e, "failed to open initial output stream");
        }

        while !self.shutting_down {
            if self.is_idle() {
                // Nothing for a tick to derive — park instead of spinning.
                match self.rx.recv() {
                    Ok(msg) => self.handle(msg),
                    Err(_) => break,
                }
            } else {
                match self.rx.recv_timeout(TICK) {
                    Ok(msg) => self.handle(msg),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            self.tick();
        }

        self.teardown();
    }

    /// No session, pending load, detached worker, or outstanding RT ring — the
    /// tick has nothing to compute, so the loop can block on the next message.
    fn is_idle(&self) -> bool {
        self.current.is_none()
            && self.fading.is_none()
            && self.pending.is_none()
            && self.graveyard.is_empty()
            && self.rt_rings_outstanding == 0
    }

    // ── message handling ────────────────────────────────────────────────

    fn handle(&mut self, msg: ActorMsg) {
        match msg {
            ActorMsg::Cmd(cmd) => self.handle_command(cmd),
            ActorMsg::Worker(msg) => self.handle_worker(msg),
            ActorMsg::DeviceError => self.handle_device_error(),
            ActorMsg::DefaultDeviceChanged => {
                // Radio can't re-seek onto a rebuilt stream; playing on the old
                // (still-working) device beats stopping.
                if self.current.as_ref().is_some_and(|c| !c.seekable) {
                    tracing::info!(
                        "default output changed during a live stream; staying on the old device"
                    );
                    return;
                }
                self.handle_device_error();
            }
        }
    }

    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Load(request) => self.handle_load(request),
            Command::CancelPending => {
                self.discard_pending();
                self.publish();
            }
            Command::Seek { position, token } => self.handle_seek(position, token),
            Command::Pause => {
                self.paused.store(true, Ordering::Relaxed);
                self.sink.pause();
                self.publish();
            }
            Command::Resume => {
                self.paused.store(false, Ordering::Relaxed);
                if let Err(e) = self.sink.play() {
                    tracing::warn!(error = %e, "failed to resume output stream");
                }
                self.publish();
            }
            Command::Stop { pause_device } => self.handle_stop(pause_device),
            Command::SetVolume(volume) => {
                let gain = volume.clamp(0.0, 1.0).powi(3);
                self.volume
                    .store(super::rt::volume_bits(gain), Ordering::Relaxed);
            }
            Command::SetChannelMode(mode) => {
                self.channel_mode = mode;
                self.send_rt(RtCmd::SetChannelMode(mode));
            }
            Command::SetEqualizer(settings) => {
                self.eq_settings = settings.clone();
                self.send_rt(RtCmd::SetEqualizer(settings));
            }
            Command::SetDeviceChangeBehavior(behavior) => {
                self.device_change_behavior = behavior;
            }
            Command::SetDuration(duration) => {
                if let Some(current) = &mut self.current {
                    current.duration = duration;
                }
                self.publish();
            }
            Command::Subscribe(tx) => self.events.push(tx),
            Command::Shutdown => self.shutting_down = true,
        }
    }

    /// Drop the probing load, if any. Its reply resolves as cancelled (channel
    /// closed, no error) and the detached worker exits on its own — never join
    /// a live worker here, it may be stuck in network I/O.
    fn discard_pending(&mut self) {
        if let Some(old) = self.pending.take() {
            drop(old.reply);
            self.graveyard.push(old.plan.worker.join);
        }
    }

    fn handle_load(&mut self, request: LoadRequest) {
        self.discard_pending();

        let LoadRequest {
            token,
            factory,
            duration,
            transition,
            start_at,
            reply,
        } = request;

        let worker = match super::worker::spawn(token, factory, self.self_tx.clone()) {
            Ok(worker) => worker,
            Err(e) => {
                let message = format!("failed to spawn decode worker: {e}");
                if let Some(reply) = reply {
                    let _ = reply.send(Err(message.clone()));
                }
                self.emit(Event::Error { token, message });
                return;
            }
        };
        self.pending = Some(Pending {
            plan: StartPlan {
                token,
                worker,
                duration,
                transition,
                start_at,
            },
            reply,
        });
        // Make the pending load visible in status right away, so the UI can see
        // a transition is in flight without waiting for the next tick.
        self.publish();
    }

    fn handle_worker(&mut self, msg: WorkerMsg) {
        match msg {
            WorkerMsg::Ready {
                token,
                source_sample_rate,
                seekable,
            } => {
                if self.pending.as_ref().is_none_or(|p| p.plan.token != token) {
                    // Stale probe from a superseded load; its command sender is
                    // gone, so the worker exits by itself.
                    return;
                }
                let pending = self.pending.take().expect("checked above");
                self.start_session(pending, source_sample_rate, seekable);
            }
            WorkerMsg::Eof { token, epoch } => {
                if let Some(current) = &mut self.current
                    && current.token == token
                    && current.ring_epoch == epoch
                {
                    current.eof = true;
                }
            }
            WorkerMsg::Failed { token, error } => {
                if self.pending.as_ref().is_some_and(|p| p.plan.token == token) {
                    let pending = self.pending.take().expect("checked above");
                    if let Some(reply) = pending.reply {
                        let _ = reply.send(Err(error.clone()));
                    }
                    self.graveyard.push(pending.plan.worker.join);
                    self.emit(Event::Error {
                        token,
                        message: error,
                    });
                    // The pending load is gone; clear it from status.
                    self.publish();
                } else if let Some(current) = self.current.take_if(|c| c.token == token) {
                    // A LIVE session's worker can fail too: a post-EOF seek
                    // whose re-probe errors sends Failed and exits. Ignoring it
                    // left the session in a silent Playing forever — retire it
                    // and report, so the controller can react.
                    self.retire_session(current);
                    self.emit(Event::Error {
                        token,
                        message: error,
                    });
                    self.publish();
                } else if let Some(fading) = self.fading.take_if(|f| f.token == token) {
                    // The outgoing side of a crossfade failing just ends its
                    // fade-out early; the incoming session is unaffected.
                    self.retire_session(fading);
                    self.publish();
                }
            }
        }
    }

    /// A probed source is ready: decide crossfade vs immediate and start it.
    fn start_session(&mut self, pending: Pending, source_sample_rate: Option<u32>, seekable: bool) {
        let Pending { plan, reply } = pending;
        let token = plan.token;

        match self.try_start_session(plan, source_sample_rate, seekable) {
            Ok(outcome) => {
                // Publish before resolving the reply so a caller that reads
                // status right after awaiting sees the new session.
                self.publish();
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(outcome));
                }
                self.emit(Event::Loaded { token });
            }
            Err(error) => {
                if let Some(reply) = reply {
                    let _ = reply.send(Err(error.clone()));
                }
                self.emit(Event::Error {
                    token,
                    message: error,
                });
            }
        }
    }

    /// Stop and detach a worker whose session never started.
    fn abort_start(&mut self, worker: WorkerHandle, error: String) -> String {
        self.retire_worker(worker);
        error
    }

    fn try_start_session(
        &mut self,
        plan: StartPlan,
        source_sample_rate: Option<u32>,
        seekable: bool,
    ) -> Result<LoadOutcome, String> {
        let StartPlan {
            token,
            worker,
            duration,
            transition,
            start_at,
        } = plan;

        let fade = match transition {
            Transition::Crossfade(fade) if !fade.is_zero() => Some(fade),
            _ => None,
        };
        // Crossfade needs a live, audible outgoing session and an open stream.
        // The fade runs at the LIVE config — the incoming worker resamples to it
        // — so a source-rate mismatch (YT mixes 48kHz Opus and 44.1kHz AAC
        // freely) doesn't silently downgrade the fade to a hard cut. While
        // paused we fall back to an immediate switch and stay paused instead of
        // blasting audio through the user's pause; a drained (ended) outgoing
        // session has nothing left to fade out. Take the outgoing session here
        // so the invariant ("a fade has an outgoing") is local.
        let live_config = self.sink.config();
        let outgoing = fade
            .filter(|_| !self.paused.load(Ordering::Relaxed) && self.rt_tx.is_some())
            .and_then(|fade| Some((fade, live_config?)))
            .and_then(|(fade, config)| {
                self.current
                    .take_if(|c| !c.ended)
                    .map(|session| (fade, config, session))
            });

        // Branch only on the fade decision; the ring/start/swap/install tail is
        // shared. `config`, `fade_frames`, and `crossfaded` capture the delta.
        let (config, fade_frames, crossfaded) = if let Some((fade, config, outgoing)) = outgoing {
            self.stop_fading();
            self.fading = Some(outgoing);
            let fade_frames = (fade.as_secs_f64() * config.sample_rate as f64).round() as u64;
            (config, Some(fade_frames.max(1)), true)
        } else {
            // Immediate switch: reopen at the source's preferred rate when it
            // differs, after retiring the outgoing sessions.
            let desired_config = match self.sink.probe_config(source_sample_rate) {
                Ok(config) => config,
                Err(e) => return Err(self.abort_start(worker, e)),
            };
            if let Some(current) = self.current.take() {
                self.retire_session(current);
            }
            self.stop_fading();

            if (self.sink.config() != Some(desired_config) || self.rt_tx.is_none())
                && let Err(e) = self.open_output(source_sample_rate)
            {
                return Err(self.abort_start(worker, e));
            }
            let Some(config) = self.sink.config() else {
                return Err(self.abort_start(worker, "no output stream".to_string()));
            };

            // A load un-pauses, including the device — a paused stream would
            // play the new track silently. Exception: a crossfade that fell
            // back *because* the user is paused honors the pause instead of
            // blasting the next track through it; it starts on Resume.
            let honor_pause = fade.is_some() && self.paused.load(Ordering::Relaxed);
            if !honor_pause {
                self.paused.store(false, Ordering::Relaxed);
                if let Err(e) = self.sink.play() {
                    tracing::warn!(error = %e, "failed to start output stream");
                }
            }
            (config, None, false)
        };

        let RingParts {
            producer,
            written,
            played,
            rt_session,
        } = make_ring(config);
        let _ = worker.cmd_tx.send(WorkerCmd::Start {
            producer,
            written: written.clone(),
            channels: config.channels,
            sample_rate: config.sample_rate,
            start_at,
            epoch: 0,
        });
        self.send_rt(RtCmd::Swap {
            session: rt_session,
            fade_frames,
        });

        self.current = Some(Session {
            token,
            worker,
            written,
            played,
            base_micros: start_at.unwrap_or(Duration::ZERO).as_micros() as u64,
            duration,
            seekable,
            source_sample_rate,
            eof: false,
            ended: false,
            ring_epoch: 0,
        });
        self.last_token = token;
        Ok(LoadOutcome { crossfaded })
    }

    /// Stop a worker and detach its join handle into the graveyard. Never
    /// joined on the command path — a worker stuck in network I/O must not
    /// stall the actor.
    fn retire_worker(&mut self, worker: WorkerHandle) {
        let _ = worker.cmd_tx.send(WorkerCmd::Stop);
        self.graveyard.push(worker.join);
    }

    /// Stop and detach a session's worker into the graveyard.
    fn retire_session(&mut self, session: Session) {
        self.retire_worker(session.worker);
    }

    /// Stop the outgoing crossfade session, if any.
    fn stop_fading(&mut self) {
        if let Some(fading) = self.fading.take() {
            self.retire_session(fading);
        }
    }

    fn handle_seek(&mut self, target: Duration, expect_token: Option<u64>) {
        let Some(config) = self.sink.config() else {
            return;
        };

        // The seek targets the visible track: during a crossfade that's the
        // outgoing (fading) session, otherwise the current one. Decide up front,
        // before any teardown:
        //   - a token that no longer matches the visible session means a
        //     crossfade promoted a different track since the caller issued the
        //     seek — drop it rather than scrub the wrong track;
        //   - seeking out of a non-seekable outgoing source (radio) must leave
        //     the fade running, not retire the incoming session and strand the
        //     RT mid-fade.
        match self.fading.as_ref().or(self.current.as_ref()) {
            None => return,
            Some(visible) if expect_token.is_some_and(|t| t != visible.token) => {
                tracing::debug!("dropping seek for a superseded session");
                return;
            }
            Some(visible) if !visible.seekable => {
                tracing::debug!("ignoring seek on a non-seekable source");
                return;
            }
            Some(_) => {}
        }

        // Cancel the fade and seek the outgoing (visible) worker in place — it
        // is still alive as the fading session, so no re-resolve. The incoming
        // session is dropped.
        if let Some(outgoing) = self.fading.take() {
            if let Some(incoming) = self.current.take() {
                self.retire_session(incoming);
            }
            self.current = Some(outgoing);
        }

        let Some(current) = &mut self.current else {
            return;
        };
        // Captured before the latch is cleared below; drives the resume rule.
        let revive_from_ended = current.ended;

        // Keep a guard gap before the end so a seek can't land past the last
        // packet (matches the old engine's END_GUARD).
        let target = if current.duration > SEEK_END_GUARD {
            target.min(current.duration - SEEK_END_GUARD)
        } else {
            Duration::ZERO
        };

        // Fresh ring: pre-seek samples die with the old one, no drain races. Bump
        // the ring generation first so a pre-seek Eof still in flight from the
        // worker is dropped instead of ending the seeked session.
        current.ring_epoch += 1;
        let ring = make_ring(config);
        let _ = current.worker.cmd_tx.send(WorkerCmd::Seek {
            target,
            producer: ring.producer,
            written: ring.written.clone(),
            // The live config rides along: a device rebuild may have changed
            // the rate/channels since this session's Start, and the decode
            // must retarget with the ring or play at the wrong pitch.
            channels: config.channels,
            sample_rate: config.sample_rate,
            epoch: current.ring_epoch,
        });
        current.written = ring.written;
        current.played = ring.played;
        current.base_micros = target.as_micros() as u64;
        current.eof = false;
        // Seeking an ended session revives its parked worker.
        current.ended = false;

        let rt_session = ring.rt_session;
        self.send_rt(RtCmd::Swap {
            session: rt_session,
            fade_frames: None,
        });
        // Seeking a track out of its ended state resumes playback: `Ended` is
        // terminal and end-of-queue quiesced the device, so scrubbing back in
        // is an intent to listen. A seek on a merely-paused track (ended ==
        // false) never reaches here and stays paused.
        if revive_from_ended {
            self.paused.store(false, Ordering::Relaxed);
            if let Err(e) = self.sink.play() {
                tracing::warn!(error = %e, "failed to resume output stream on seek revive");
            }
        }
        self.publish();
    }

    fn handle_stop(&mut self, pause_device: bool) {
        self.discard_pending();
        if let Some(current) = self.current.take() {
            self.retire_session(current);
        }
        self.stop_fading();
        self.send_rt(RtCmd::DropAll);
        self.paused.store(false, Ordering::Relaxed);
        if pause_device {
            self.sink.pause();
        }
        self.publish();
    }

    /// The output stream died (device unplugged, format lost). Rebuild it and
    /// resume the current session at its last position via the seek protocol.
    fn handle_device_error(&mut self) {
        // The dead stream's callback can emit a burst of errors; rebuild once.
        if self
            .last_output_rebuild
            .is_some_and(|at| at.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        self.last_output_rebuild = Some(Instant::now());

        let position = self.status.load().position();
        let source_rate = self.current.as_ref().and_then(|c| c.source_sample_rate);

        self.stop_fading();

        let was_playing = self.current.is_some() && !self.paused.load(Ordering::Relaxed);

        match self.open_output(source_rate) {
            Ok(_) => {
                let resumable = self.current.as_ref().is_some_and(|c| c.seekable);
                if resumable {
                    tracing::info!("output device lost; rebuilt stream and reseeking");
                    self.handle_seek(position, None);
                } else if let Some(current) = self.current.take() {
                    // Non-seekable (radio): the controller has to re-load.
                    let token = current.token;
                    self.retire_session(current);
                    self.emit(Event::Error {
                        token,
                        message: "output device lost".to_string(),
                    });
                }
                // The user chooses whether a device change keeps playing on the
                // new output or holds paused there (e.g. unplugged headphones
                // shouldn't blast the speakers).
                if was_playing
                    && self.device_change_behavior == config::DeviceChangeBehavior::Pause
                    && self.current.is_some()
                {
                    self.paused.store(true, Ordering::Relaxed);
                }
                if self.paused.load(Ordering::Relaxed) {
                    self.sink.pause();
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to rebuild output stream");
                let token = self.last_token;
                self.handle_stop(false);
                self.emit(Event::Error {
                    token,
                    message: format!("output device lost: {e}"),
                });
            }
        }
        self.publish();
    }

    // ── periodic work ───────────────────────────────────────────────────

    fn tick(&mut self) {
        // Each step publishes if (and only if) it changed observable state, so
        // a steady Playing tick allocates nothing: position reads live off the
        // shared atomic in the last-published status.
        self.reap_rt_retired();
        self.latch_drain_complete();
        self.emit_throttled_position();

        // Finished detached workers just get dropped (JoinHandle drop detaches);
        // live ones are re-checked next tick.
        self.graveyard.retain(|handle| !handle.is_finished());
    }

    /// Resources the RT callback shipped back: drop rings here (never on the
    /// audio thread) and finish crossfades. Each Retired balances a Swap.
    fn reap_rt_retired(&mut self) {
        let mut fade_completed = false;
        let mut retired = 0usize;
        if let Some(retire_rx) = &self.retire_rx {
            while let Ok(msg) = retire_rx.try_recv() {
                retired += 1;
                if matches!(msg, Retired::FadeComplete(_)) {
                    fade_completed = true;
                }
            }
        }
        self.rt_rings_outstanding = self.rt_rings_outstanding.saturating_sub(retired);

        // Emit TrackSwitched only when an outgoing session actually retired: a
        // FadeComplete can race a seek that already cancelled the fade, and
        // that must not fabricate a switch.
        if fade_completed && let Some(outgoing) = self.fading.take() {
            let from_token = outgoing.token;
            self.retire_session(outgoing);
            if let Some(token) = self.current.as_ref().map(|c| c.token) {
                self.emit(Event::TrackSwitched { token, from_token });
            }
            self.publish();
        }
    }

    /// Drain-complete: the worker hit EOF and the audio callback has played
    /// everything it wrote. Exactly-once by the `ended` latch.
    fn latch_drain_complete(&mut self) {
        let ended_token = match &mut self.current {
            Some(current)
                if current.eof
                    && !current.ended
                    && current.played.load(Ordering::Relaxed)
                        >= current.written.load(Ordering::Relaxed) =>
            {
                current.ended = true;
                Some(current.token)
            }
            _ => None,
        };
        if let Some(token) = ended_token {
            // emit() wakes the platform run loop, so the subscriber's
            // auto-advance fires without waiting for a poll tick.
            self.emit(Event::Ended { token });
            self.publish();
        }
    }

    /// Position once per second while playing — subscribers render seconds,
    /// and every event is a wakeup on their side.
    fn emit_throttled_position(&mut self) {
        if self.phase() != Phase::Playing {
            return;
        }
        let position = self.status.load().position();
        if let Some(token) = self.current.as_ref().map(|c| c.token) {
            let mark = (token, position.as_secs());
            if self.last_position_emitted != Some(mark) {
                self.last_position_emitted = Some(mark);
                self.emit(Event::Position { token, position });
            }
        }
        // MPRIS reads position on demand from this stored value; the old
        // engine ran a dedicated 250ms thread for it.
        #[cfg(target_os = "linux")]
        systemint::update_position(position.as_secs_f64());
    }

    // ── plumbing ────────────────────────────────────────────────────────

    fn phase(&self) -> Phase {
        match &self.current {
            None => Phase::Idle,
            Some(c) if c.ended => Phase::Ended,
            Some(_) if self.paused.load(Ordering::Relaxed) => Phase::Paused,
            Some(_) => Phase::Playing,
        }
    }

    fn publish(&mut self) {
        let phase = self.phase();
        let config = self.sink.config();
        let paused = self.paused.load(Ordering::Relaxed);
        let channels = config.map(|c| c.channels as u32).unwrap_or(0);
        let sample_rate = config.map(|c| c.sample_rate).unwrap_or(0);
        let pending_token = self.pending.as_ref().map(|p| p.plan.token);
        let fading = self.fading.as_ref().map(|f| FadingStatus {
            token: f.token,
            duration: f.duration,
            base_micros: f.base_micros,
            played_samples: f.played.clone(),
        });
        let status = match &self.current {
            Some(current) => EngineStatus {
                token: current.token,
                phase,
                paused,
                duration: current.duration,
                pending_token,
                fading,
                base_micros: current.base_micros,
                played_samples: current.played.clone(),
                channels,
                sample_rate,
            },
            None => EngineStatus {
                token: self.last_token,
                phase,
                paused,
                duration: Duration::ZERO,
                pending_token,
                fading,
                base_micros: 0,
                played_samples: Arc::new(AtomicU64::new(0)),
                channels: 0,
                sample_rate: 0,
            },
        };
        self.status.store(Arc::new(status));

        if phase != self.last_phase {
            self.last_phase = phase;
            self.emit(Event::PhaseChanged {
                token: self.last_token,
                phase,
            });
        }
    }

    fn emit(&mut self, event: Event) {
        if self.events.is_empty() {
            return;
        }
        let is_position = matches!(event, Event::Position { .. });
        let mut delivered = false;
        // Fan out to every subscriber, pruning any whose receiver has dropped.
        self.events.retain(|tx| match tx.send(event.clone()) {
            Ok(()) => {
                delivered = true;
                true
            }
            Err(_) => false,
        });
        if delivered && !is_position {
            // Waking tokio isn't enough on platforms where the app main loop
            // itself may be parked (tao/CFRunLoop).
            #[cfg(any(target_os = "android", target_os = "macos"))]
            systemint::wake_run_loop();
        }
    }

    fn send_rt(&mut self, cmd: RtCmd) {
        // Each Swap hands the RT a new ring consumer it will later ship back as
        // a Retired message; track the balance so the loop knows when the RT is
        // empty and it can park.
        if matches!(cmd, RtCmd::Swap { .. }) {
            self.rt_rings_outstanding += 1;
        }
        if let Some(rt_tx) = &self.rt_tx {
            let _ = rt_tx.send(cmd);
        }
    }

    /// (Re)open the output stream with a fresh RT state derived from the
    /// actor-held canonical settings.
    fn open_output(&mut self, desired_sample_rate: Option<u32>) -> Result<SinkConfig, String> {
        let (rt_tx, rt_rx) = std::sync::mpsc::channel();
        let (retire_tx, retire_rx) = std::sync::mpsc::channel();
        let volume = self.volume.clone();
        let paused = self.paused.clone();
        let eq_settings = self.eq_settings.clone();
        let channel_mode = self.channel_mode;

        let make_cb: DataCallbackFactory = Box::new(move |config: SinkConfig| {
            let mut state = RtState::new(
                rt_rx,
                retire_tx,
                volume,
                paused,
                config.channels,
                config.sample_rate,
                eq_settings,
                channel_mode,
            );
            Box::new(move |data: &mut [f32]| state.process(data))
        });

        let config = self.sink.open(desired_sample_rate, make_cb)?;
        self.rt_tx = Some(rt_tx);
        self.retire_rx = Some(retire_rx);
        // The old RT state (and any rings it still held) is dropped with the old
        // stream; the fresh one starts owning nothing.
        self.rt_rings_outstanding = 0;
        Ok(config)
    }

    fn teardown(&mut self) {
        // Detach every remaining worker into the graveyard, then join the lot.
        // A pending probe's cmd channel closes when its handle drops, so it
        // needs no explicit Stop.
        if let Some(pending) = self.pending.take() {
            drop(pending.reply);
            self.graveyard.push(pending.plan.worker.join);
        }
        if let Some(current) = self.current.take() {
            self.retire_session(current);
        }
        if let Some(fading) = self.fading.take() {
            self.retire_session(fading);
        }
        // Closing the sink drops the stream and with it the RT state and any
        // consumers it still owns, unblocking workers stuck on full rings.
        self.sink.close();
        self.rt_tx = None;
        self.retire_rx = None;

        let joins = std::mem::take(&mut self.graveyard);
        for join in joins {
            // A worker wedged in network I/O can't be joined without hanging
            // shutdown; detach it and let process exit clean it up.
            if join.is_finished() {
                let _ = join.join();
            } else {
                std::thread::sleep(Duration::from_millis(50));
                if join.is_finished() {
                    let _ = join.join();
                }
            }
        }
        self.status.store(Arc::new(EngineStatus::idle()));
    }
}
