//! The PlayerSession actor: sole owner of queue, transport, intent, and audio
//! engine state. Commands and engine events are serialized through one tokio
//! task, then projected into watch snapshots and broadcast API events.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, BufferedRange, CommandAck, ExternalPlayback, FadingState, Intent,
    NowPlaying, Page, Phase as ApiPhase, PlayerCommand, PlayerState, PositionAnchor, QueueContext,
    QueueEdit, QueueItem, QueueMode, QueueSummary, QueueWindow, SetQueueRequest, TrackKind,
};
use player::engine::{Event as EngineEvent, Phase as EnginePhase, SourceFactory, Transition};
use player::player::{LoadArgs, NowPlayingMeta, Player, PlayerInitError};
use reader::Track;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use utils::playback_ref::{PlaybackItemRef, ResolvedStreamRef};

use crate::playback::network_factory;
use crate::queue_model::{NextOutcome, QueueModel};

mod load;
mod reconciler;

use load::{LoadFailure, LoadFinished, PreparedLoad};

pub const EVENT_BUFFER: usize = 512;
const POSITION_CORRECTION_INTERVAL: Duration = Duration::from_secs(10);
const MATERIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
const PERSIST_INTERVAL: Duration = Duration::from_secs(5);
const PROGRESS_STEP_SECS: u64 = 5;

struct CachedQueueSnapshot {
    snapshot: db::QueueSnapshot,
    position_secs: u64,
    at: Instant,
    playing: bool,
}

impl Default for CachedQueueSnapshot {
    fn default() -> Self {
        Self {
            snapshot: db::QueueSnapshot::default(),
            position_secs: 0,
            at: Instant::now(),
            playing: false,
        }
    }
}

/// Resolves a wire queue context into concrete tracks daemon-side.
#[async_trait::async_trait]
pub trait QueueMaterializer: Send + Sync {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError>;
}

/// Durable playback bookkeeping: recents on commit, listen counts when a
/// track completes or crossfades out. Implemented over the active source;
/// tests inject a stub.
#[async_trait::async_trait]
pub trait PlaybackRecorder: Send + Sync {
    async fn record_recent(&self, track: &Track);
    async fn bump_listen_count(&self, track: &Track);
}

/// Playback dependencies that will eventually be owned by daemon services.
/// Keeping them together lets the actor land before ConfigService and source
/// lifecycle extraction do.
pub struct PlaybackServices {
    pub config: config::AppConfig,
    pub active_source: Option<server::source::ActiveSource>,
    pub station_registry: Arc<radio::registry::StationRegistry>,
    pub queue_store: Option<Arc<dyn crate::persistence::QueueStore>>,
    pub recorder: Option<Arc<dyn PlaybackRecorder>>,
    pub scrobbler: Option<Arc<crate::scrobbler::Scrobbler>>,
}

impl Default for PlaybackServices {
    fn default() -> Self {
        Self {
            config: config::AppConfig::default(),
            active_source: None,
            station_registry: Arc::new(radio::registry::StationRegistry::default()),
            queue_store: None,
            recorder: None,
            scrobbler: None,
        }
    }
}

pub type FactoryOverride = Arc<dyn Fn(&Track) -> Option<SourceFactory> + Send + Sync>;

enum SessionCmd {
    Player(PlayerCommand, oneshot::Sender<Result<CommandAck, ApiError>>),
    Edit(QueueEdit, oneshot::Sender<Result<CommandAck, ApiError>>),
    RadioMetadata {
        token: u64,
        title: String,
        artist: Option<String>,
    },
    SetQueueMaterialized {
        request_id: u64,
        request: SetQueueRequest,
        tracks: Vec<Track>,
        reply: oneshot::Sender<Result<CommandAck, ApiError>>,
    },
    Window(Page, oneshot::Sender<Result<QueueWindow, ApiError>>),
    RestoreQueue(
        Box<db::QueueSnapshot>,
        oneshot::Sender<Result<CommandAck, ApiError>>,
    ),
    SetConfig {
        config: Box<config::AppConfig>,
        changed: Vec<String>,
    },
    PreviewEqualizer(Box<config::EqualizerSettings>),
    Emit(Box<ApiEvent>),
    SetStationRegistry(Arc<radio::registry::StationRegistry>),
    SetActiveSource(Option<server::source::ActiveSource>),
    SetExternal(Option<ExternalPlayback>),
    ReportExternal {
        track: Option<Track>,
        position_ms: u64,
        playing: bool,
        completed: bool,
        device: Option<String>,
        reply: oneshot::Sender<Result<(), ApiError>>,
    },
    ResetPlayback(oneshot::Sender<Result<CommandAck, ApiError>>),
    Persist(oneshot::Sender<()>),
    LoadPrepared(Box<Result<PreparedLoad, LoadFailure>>),
    LoadFinished(LoadFinished),
    BufferProgress(BufferProgressEvent),
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    ArtworkFetched {
        token: u64,
        meta: Box<NowPlayingMeta>,
    },
}

#[derive(Clone)]
pub struct SessionHandle {
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
    state_rx: watch::Receiver<PlayerState>,
    config_rx: watch::Receiver<config::AppConfig>,
    events: broadcast::Sender<(u64, ApiEvent)>,
    seq: Arc<AtomicU64>,
    history: Arc<Mutex<VecDeque<(u64, ApiEvent)>>>,
    materializer: Arc<dyn QueueMaterializer>,
    queue_request_seq: Arc<AtomicU64>,
    queue_snapshot: Arc<Mutex<CachedQueueSnapshot>>,
}

impl SessionHandle {
    pub fn try_spawn(
        materializer: Arc<dyn QueueMaterializer>,
        services: PlaybackServices,
    ) -> Result<Self, PlayerInitError> {
        let player = Player::try_new()?;
        Ok(Self::spawn_with_player(materializer, player, services))
    }

    pub fn spawn_with_player(
        materializer: Arc<dyn QueueMaterializer>,
        player: Player,
        services: PlaybackServices,
    ) -> Self {
        Self::spawn_inner(materializer, player, services, None)
    }

    fn spawn_inner(
        materializer: Arc<dyn QueueMaterializer>,
        player: Player,
        services: PlaybackServices,
        factory_override: Option<FactoryOverride>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let seq = Arc::new(AtomicU64::new(0));
        let history = Arc::new(Mutex::new(VecDeque::new()));
        let queue_request_seq = Arc::new(AtomicU64::new(0));
        let queue_snapshot = Arc::new(Mutex::new(CachedQueueSnapshot::default()));
        let (config_tx, config_rx) = watch::channel(services.config.clone());
        let engine_events = player.subscribe();
        player.set_volume(services.config.volume);
        player.set_channel_mode(services.config.channel_mode);
        player.set_equalizer(services.config.equalizer.clone());
        player.set_device_change_behavior(services.config.device_change_behavior);
        player.set_sample_rate_mode(services.config.sample_rate_mode);

        let queue_writer = services.queue_store.map(|store| {
            let (tx, mut rx) =
                mpsc::unbounded_channel::<(db::QueueSnapshot, Option<oneshot::Sender<()>>)>();
            tokio::spawn(async move {
                while let Some((snapshot, reply)) = rx.recv().await {
                    store.save(snapshot).await;
                    if let Some(reply) = reply {
                        let _ = reply.send(());
                    }
                }
            });
            tx
        });
        let session = Session {
            model: QueueModel::default(),
            player,
            intent: PlaybackIntent::Stopped,
            next_token: 0,
            current_token: 0,
            pending_resume: None,
            pending_transition: None,
            armed_transition: None,
            load_task: None,
            radio_task: None,
            phase: ApiPhase::Idle,
            position: None,
            position_token: None,
            buffered: Vec::new(),
            error: None,
            rev: 0,
            queue_rev: 0,
            volume: services.config.volume,
            epoch: Instant::now(),
            events: events.clone(),
            seq: seq.clone(),
            history: history.clone(),
            queue_writer,
            queue_request_seq: queue_request_seq.clone(),
            queue_snapshot: queue_snapshot.clone(),
            queue_dirty: false,
            recorder: services.recorder,
            scrobbler: services.scrobbler,
            last_recent_key: None,
            config_tx,
            config: services.config,
            external: None,
            external_track: None,
            external_completed_key: None,
            active_source: services.active_source,
            station_registry: services.station_registry,
            cmd_tx: cmd_tx.clone(),
            factory_override,
        };
        let (state_tx, state_rx) = watch::channel(session.build_state());
        tokio::spawn(session.run(cmd_rx, engine_events, state_tx));
        Self {
            cmd_tx,
            state_rx,
            config_rx,
            events,
            seq,
            history,
            materializer,
            queue_request_seq,
            queue_snapshot,
        }
    }

    /// Test and diagnostic seam: every load resolves through the given
    /// factory instead of classifying real sources. Contract tests use it to
    /// run deterministic decodes against a [`player::engine::NullSink`].
    pub fn spawn_with_factory(
        materializer: Arc<dyn QueueMaterializer>,
        player: Player,
        services: PlaybackServices,
        factory_override: FactoryOverride,
    ) -> Self {
        Self::spawn_inner(materializer, player, services, Some(factory_override))
    }

    pub fn state(&self) -> PlayerState {
        self.state_rx.borrow().clone()
    }

    pub fn state_watch(&self) -> watch::Receiver<PlayerState> {
        self.state_rx.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(u64, ApiEvent)> {
        self.events.subscribe()
    }

    /// The session's live config copy: seeded at spawn, updated by
    /// `set_config`. Integration tasks watch this instead of polling.
    pub fn config_watch(&self) -> watch::Receiver<config::AppConfig> {
        self.config_rx.clone()
    }

    /// Events after `last` from the replay ring. `true` means the ring no
    /// longer reaches back that far: the client must refetch its snapshots
    /// (the `resync` contract) and then continue from the live stream.
    pub fn replay_since(&self, last: u64) -> (bool, Vec<(u64, ApiEvent)>) {
        let newest = self.seq.load(Ordering::Acquire);
        if newest <= last {
            return (false, Vec::new());
        }
        let Ok(history) = self.history.lock() else {
            return (true, Vec::new());
        };
        match history.front() {
            Some((first, _)) if *first <= last + 1 => (
                false,
                history
                    .iter()
                    .filter(|(sequence, _)| *sequence > last)
                    .cloned()
                    .collect(),
            ),
            _ => (true, Vec::new()),
        }
    }

    async fn request<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, ApiError>>) -> SessionCmd,
    ) -> Result<T, ApiError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(build(tx))
            .map_err(|_| ApiError::internal("daemon session terminated"))?;
        rx.await
            .map_err(|_| ApiError::internal("daemon session terminated"))?
    }

    pub async fn player_command(&self, command: PlayerCommand) -> Result<CommandAck, ApiError> {
        self.request(|tx| SessionCmd::Player(command, tx)).await
    }

    pub async fn set_queue(&self, request: SetQueueRequest) -> Result<CommandAck, ApiError> {
        if request.mode != QueueMode::Replace
            && (request.start_index.is_some() || request.shuffle.is_some())
        {
            return Err(ApiError::invalid_input(
                "start_index and shuffle apply to mode \"replace\" only",
            ));
        }
        if request.mode == QueueMode::Insert && request.insert_index.is_none() {
            return Err(ApiError::invalid_input(
                "insert_index is required for mode \"insert\"",
            ));
        }
        if request.mode != QueueMode::Insert && request.insert_index.is_some() {
            return Err(ApiError::invalid_input(
                "insert_index applies to mode \"insert\" only",
            ));
        }
        let request_id = self.queue_request_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let tracks = tokio::time::timeout(
            MATERIALIZE_TIMEOUT,
            self.materializer.materialize(&request.context),
        )
        .await
        .map_err(|_| {
            ApiError::new(
                api::ErrorCode::SourceUnreachable,
                "queue materialization timed out",
            )
        })??;
        if self.queue_request_seq.load(Ordering::Acquire) != request_id {
            return Err(ApiError::new(
                api::ErrorCode::Conflict,
                "queue request superseded",
            ));
        }
        self.request(|reply| SessionCmd::SetQueueMaterialized {
            request_id,
            request,
            tracks,
            reply,
        })
        .await
    }

    pub async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        self.request(|tx| SessionCmd::Window(page, tx)).await
    }

    pub async fn materialize_track(&self, key: String) -> Result<Track, ApiError> {
        self.materializer
            .materialize(&QueueContext::Tracks { keys: vec![key] })
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("unknown track key"))
    }

    pub async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError> {
        self.queue_request_seq.fetch_add(1, Ordering::AcqRel);
        self.request(|tx| SessionCmd::Edit(edit, tx)).await
    }

    /// Restore a persisted queue: paused, with a resume point at the saved
    /// progress, exactly like the app's startup restore.
    pub async fn restore_queue(&self, snapshot: db::QueueSnapshot) -> Result<CommandAck, ApiError> {
        self.queue_request_seq.fetch_add(1, Ordering::AcqRel);
        self.request(|tx| SessionCmd::RestoreQueue(Box::new(snapshot), tx))
            .await
    }

    /// Emit an event through the session's sequenced stream (services use
    /// this for invalidations, job progress, and notices).
    pub fn emit_event(&self, event: ApiEvent) {
        let _ = self.cmd_tx.send(SessionCmd::Emit(Box::new(event)));
    }

    /// Swap the radio station registry after a registry import completes, so
    /// radio contexts resolve against live manifests.
    pub fn set_station_registry(&self, registry: Arc<radio::registry::StationRegistry>) {
        let _ = self.cmd_tx.send(SessionCmd::SetStationRegistry(registry));
    }

    /// Swap the active media source after a server switch or credential
    /// rotation, so subsequent loads resolve against the new backend.
    pub fn set_active_source(&self, source: Option<server::source::ActiveSource>) {
        let _ = self.cmd_tx.send(SessionCmd::SetActiveSource(source));
    }

    /// Declare (or clear) external playback: while set, `PlayerState.external`
    /// is populated and transport commands are forwarded to the owning
    /// frontend as `player.external_command` events instead of driving the
    /// stopped engine.
    pub fn set_external(&self, external: Option<ExternalPlayback>) {
        let _ = self.cmd_tx.send(SessionCmd::SetExternal(external));
    }

    pub async fn report_external(
        &self,
        track: Option<Track>,
        position_ms: u64,
        playing: bool,
        completed: bool,
        device: Option<String>,
    ) -> Result<(), ApiError> {
        self.request(|reply| SessionCmd::ReportExternal {
            track,
            position_ms,
            playing,
            completed,
            device,
            reply,
        })
        .await
    }

    /// Serialize source teardown so external ownership cannot race engine reset.
    pub async fn reset_playback(&self) -> Result<CommandAck, ApiError> {
        self.queue_request_seq.fetch_add(1, Ordering::AcqRel);
        self.request(SessionCmd::ResetPlayback).await
    }

    pub fn queue_snapshot(&self) -> db::QueueSnapshot {
        self.queue_snapshot
            .lock()
            .map(|cached| {
                let mut snapshot = cached.snapshot.clone();
                if cached.playing {
                    let position = cached
                        .position_secs
                        .saturating_add(cached.at.elapsed().as_secs());
                    snapshot.progress_secs = (position / PROGRESS_STEP_SECS) * PROGRESS_STEP_SECS;
                }
                snapshot
            })
            .unwrap_or_default()
    }

    /// Adopt a new config (a ConfigService patch): applies live audio
    /// settings and emits `config.changed`.
    pub fn set_config(&self, config: config::AppConfig, changed: Vec<String>) {
        let _ = self.cmd_tx.send(SessionCmd::SetConfig {
            config: Box::new(config),
            changed,
        });
    }

    pub fn preview_equalizer(&self, equalizer: config::EqualizerSettings) -> Result<(), ApiError> {
        self.cmd_tx
            .send(SessionCmd::PreviewEqualizer(Box::new(equalizer)))
            .map_err(|_| ApiError::internal("player session is unavailable"))
    }

    /// Flush the current queue snapshot to the store and wait for the write;
    /// the shutdown path calls this so a quit never loses the debounce window.
    pub async fn persist_now(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(SessionCmd::Persist(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaybackIntent {
    Stopped,
    Loading {
        token: u64,
        idx: usize,
        crossfade: bool,
        from_token: u64,
    },
    Committed {
        token: u64,
    },
}

impl PlaybackIntent {
    fn token(self) -> u64 {
        match self {
            Self::Stopped => 0,
            Self::Loading { token, .. } | Self::Committed { token } => token,
        }
    }

    fn is_loading(self) -> bool {
        matches!(self, Self::Loading { .. })
    }
}

impl From<PlaybackIntent> for Intent {
    fn from(value: PlaybackIntent) -> Self {
        match value {
            PlaybackIntent::Stopped => Self::Stopped,
            PlaybackIntent::Loading {
                token, from_token, ..
            } => Self::Loading {
                token,
                from_token: (from_token != 0).then_some(from_token),
            },
            PlaybackIntent::Committed { token } => Self::Committed { token },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingResumeState {
    track_key: String,
    position_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransitionStage {
    Loading,
    Fading,
}

/// A crossfade in flight. The queue itself is not forked: only the logical
/// position the pointer will move to once the engine reports the switch, so
/// queue edits arriving mid-fade land on the one live model and survive the
/// commit (issue #656). Structural edits remap `to_position` the same way they
/// remap `current`.
struct PendingTransition {
    to_position: usize,
    to_token: u64,
    from_token: u64,
    stage: TransitionStage,
}

struct Session {
    model: QueueModel,
    player: Player,
    intent: PlaybackIntent,
    next_token: u64,
    current_token: u64,
    pending_resume: Option<PendingResumeState>,
    pending_transition: Option<PendingTransition>,
    armed_transition: Option<u64>,
    load_task: Option<(u64, JoinHandle<()>)>,
    radio_task: Option<JoinHandle<()>>,
    phase: ApiPhase,
    position: Option<PositionAnchor>,
    position_token: Option<u64>,
    buffered: Vec<BufferedRange>,
    error: Option<api::ErrorBody>,
    rev: u64,
    queue_rev: u64,
    volume: f32,
    epoch: Instant,
    events: broadcast::Sender<(u64, ApiEvent)>,
    seq: Arc<AtomicU64>,
    history: Arc<Mutex<VecDeque<(u64, ApiEvent)>>>,
    queue_writer: Option<mpsc::UnboundedSender<(db::QueueSnapshot, Option<oneshot::Sender<()>>)>>,
    queue_request_seq: Arc<AtomicU64>,
    queue_snapshot: Arc<Mutex<CachedQueueSnapshot>>,
    queue_dirty: bool,
    recorder: Option<Arc<dyn PlaybackRecorder>>,
    scrobbler: Option<Arc<crate::scrobbler::Scrobbler>>,
    last_recent_key: Option<String>,
    config_tx: watch::Sender<config::AppConfig>,
    config: config::AppConfig,
    external: Option<ExternalPlayback>,
    external_track: Option<Track>,
    external_completed_key: Option<String>,
    active_source: Option<server::source::ActiveSource>,
    station_registry: Arc<radio::registry::StationRegistry>,
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
    factory_override: Option<FactoryOverride>,
}

impl Session {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<SessionCmd>,
        mut engine_events: mpsc::UnboundedReceiver<EngineEvent>,
        state_tx: watch::Sender<PlayerState>,
    ) {
        let correction_start = tokio::time::Instant::now() + POSITION_CORRECTION_INTERVAL;
        let mut correction =
            tokio::time::interval_at(correction_start, POSITION_CORRECTION_INTERVAL);
        correction.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut persist = tokio::time::interval_at(
            tokio::time::Instant::now() + PERSIST_INTERVAL,
            PERSIST_INTERVAL,
        );
        persist.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            // The correction branch is disabled while nothing plays, so an
            // idle daemon takes zero timer wakeups and this task parks until
            // a command or engine event arrives.
            tokio::select! {
                command = cmd_rx.recv() => {
                    let Some(command) = command else { break };
                    self.handle_command(command, &state_tx).await;
                }
                event = engine_events.recv() => {
                    let Some(event) = event else { break };
                    self.handle_engine_event(event, &state_tx);
                }
                _ = correction.tick(), if self.phase == ApiPhase::Playing && self.external.is_none() => {
                    self.publish_position_anchor(&state_tx, None, None, true);
                }
                _ = persist.tick(), if self.queue_dirty && self.queue_writer.is_some() => {
                    self.persist_async();
                }
            }
        }
    }

    async fn handle_command(&mut self, command: SessionCmd, state_tx: &watch::Sender<PlayerState>) {
        match command {
            SessionCmd::Player(command, reply) => {
                let result = self.handle_player_command(command, state_tx);
                let _ = reply.send(result);
            }
            SessionCmd::SetQueueMaterialized {
                request_id,
                request,
                tracks,
                reply,
            } => {
                let result = if self.queue_request_seq.load(Ordering::Acquire) == request_id {
                    if request.mode == QueueMode::Insert {
                        let count = tracks.len();
                        let at = (request.insert_index.unwrap_or_default() as usize)
                            .min(self.model.len());
                        self.model.insert_at(at, tracks);
                        self.shift_transition_target(at, count);
                        Ok(self.publish(state_tx, true))
                    } else {
                        self.apply_queue_tracks(
                            tracks,
                            request.mode,
                            request.start_index.map(|index| index as usize),
                            request.shuffle,
                            state_tx,
                        )
                    }
                } else {
                    Err(ApiError::new(
                        api::ErrorCode::Conflict,
                        "queue request superseded",
                    ))
                };
                let _ = reply.send(result);
            }
            SessionCmd::Window(page, reply) => {
                let _ = reply.send(Ok(self.window(page)));
            }
            SessionCmd::Edit(edit, reply) => {
                let result = self.handle_queue_edit(edit, state_tx);
                let _ = reply.send(result);
            }
            SessionCmd::RadioMetadata {
                token,
                title,
                artist,
            } => self.apply_radio_metadata(token, title, artist, state_tx),
            SessionCmd::RestoreQueue(snapshot, reply) => {
                let result = self.handle_restore(*snapshot, state_tx);
                let _ = reply.send(result);
            }
            SessionCmd::SetConfig { config, changed } => {
                self.apply_config(*config, changed, state_tx);
            }
            SessionCmd::PreviewEqualizer(equalizer) => {
                self.player.set_equalizer(*equalizer);
            }
            SessionCmd::Emit(event) => self.emit(*event),
            SessionCmd::SetStationRegistry(registry) => self.station_registry = registry,
            SessionCmd::SetActiveSource(source) => self.active_source = source,
            SessionCmd::SetExternal(external) => {
                if self.external != external {
                    if external.is_some() && self.external.is_none() {
                        self.stop(state_tx);
                    }
                    self.external = external;
                    if self.external.is_none() {
                        self.external_track = None;
                        self.external_completed_key = None;
                        self.set_intent(PlaybackIntent::Stopped);
                        self.phase = ApiPhase::Idle;
                        self.position = None;
                    }
                    self.publish(state_tx, false);
                }
            }
            SessionCmd::ReportExternal {
                track,
                position_ms,
                playing,
                completed,
                device,
                reply,
            } => {
                let result = self.apply_external_report(
                    track,
                    position_ms,
                    playing,
                    completed,
                    device,
                    state_tx,
                );
                let _ = reply.send(result);
            }
            SessionCmd::ResetPlayback(reply) => {
                let was_external = self.external.take().is_some();
                if was_external {
                    self.emit(ApiEvent::PlayerExternalCommand(PlayerCommand::Stop));
                }
                tracing::info!(external = was_external, "playback reset");
                self.stop(state_tx);
                let result =
                    self.apply_queue_tracks(Vec::new(), QueueMode::Replace, None, None, state_tx);
                let _ = reply.send(result);
            }
            SessionCmd::Persist(reply) => {
                if let Some(writer) = self.queue_writer.clone() {
                    let snapshot = self.snapshot();
                    self.queue_dirty = false;
                    if let Err(error) = writer.send((snapshot, Some(reply)))
                        && let Some(reply) = error.0.1
                    {
                        let _ = reply.send(());
                    }
                } else {
                    let _ = reply.send(());
                }
            }
            SessionCmd::LoadPrepared(result) => self.handle_prepared_load(*result, state_tx),
            SessionCmd::LoadFinished(result) => self.handle_load_finished(result, state_tx),
            SessionCmd::BufferProgress(event) => self.handle_buffer_progress(event, state_tx),
            SessionCmd::ArtworkFetched { token, meta } => {
                if self.intent.token() == token {
                    self.player.update_metadata(*meta);
                }
            }
        }
    }

    fn handle_player_command(
        &mut self,
        command: PlayerCommand,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        if matches!(command, PlayerCommand::SetVolume { .. }) {
            tracing::debug!(
                ?command,
                external = self.external.is_some(),
                "playback command"
            );
        } else {
            tracing::info!(
                ?command,
                external = self.external.is_some(),
                "playback command"
            );
        }
        // While playback is external the engine is stopped and transport
        // commands would silently no-op against it; forward them to the
        // frontend that owns the external session instead. Volume and mode
        // stay engine/queue-side (the queue mode is daemon state either way).
        if self.external.is_some()
            && matches!(
                command,
                PlayerCommand::Play
                    | PlayerCommand::Pause
                    | PlayerCommand::Toggle
                    | PlayerCommand::Next
                    | PlayerCommand::Previous
                    | PlayerCommand::Stop
                    | PlayerCommand::Seek { .. }
            )
        {
            self.emit(ApiEvent::PlayerExternalCommand(command));
            return Ok(CommandAck { rev: self.rev });
        }
        let mut queue_changed = false;
        match command {
            PlayerCommand::Play => self.resume(state_tx),
            PlayerCommand::Pause => self.pause(state_tx),
            PlayerCommand::Toggle => {
                if self.phase == ApiPhase::Playing {
                    self.pause(state_tx);
                } else {
                    self.resume(state_tx);
                }
            }
            PlayerCommand::Next => self.play_next(false, state_tx)?,
            PlayerCommand::Previous => self.play_previous(state_tx)?,
            PlayerCommand::Stop => self.stop(state_tx),
            PlayerCommand::Seek { position_ms } => self.seek(position_ms, state_tx)?,
            PlayerCommand::SetVolume { volume } => {
                self.volume = volume.clamp(0.0, 1.0);
                self.player.set_volume(self.volume);
            }
            PlayerCommand::SetMode { shuffle, loop_mode } => {
                queue_changed = shuffle.is_some();
                if let Some(on) = shuffle {
                    // Toggling rebuilds the permutation around the outgoing
                    // track, so a pending crossfade target has to be re-found
                    // by the physical index it held before the rebuild.
                    let target = match self.pending_transition.as_ref() {
                        Some(pending) => self.model.physical_index_of(pending.to_position),
                        None => None,
                    };
                    self.model.set_shuffle(on);
                    if let Some(physical) = target {
                        let position = if self.model.shuffle() {
                            self.model.shuffle_position_of(physical)
                        } else {
                            Some(physical)
                        };
                        if let (Some(pending), Some(position)) =
                            (self.pending_transition.as_mut(), position)
                        {
                            pending.to_position = position;
                        }
                    }
                }
                if let Some(mode) = loop_mode {
                    self.model.set_loop_mode(mode);
                }
            }
        }
        Ok(self.publish(state_tx, queue_changed))
    }

    fn handle_queue_edit(
        &mut self,
        edit: QueueEdit,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        let len = self.model.len();
        tracing::info!(?edit, queue_len = len, "queue edit requested");
        match edit {
            QueueEdit::Jump { index } => {
                let index = index as usize;
                let mut candidate = self.model.clone();
                let position = candidate
                    .jump_to_position(index)
                    .ok_or_else(|| ApiError::invalid_input("no track at that queue position"))?;
                self.start_immediate_load(candidate, position)?;
                Ok(self.publish(state_tx, true))
            }
            QueueEdit::Move { from, to } => {
                let (from, to) = (from as usize, to as usize);
                if from >= len || to >= len {
                    return Err(ApiError::invalid_input("queue position out of range"));
                }
                self.model.move_item(from, to);
                if let Some(pending) = self.pending_transition.as_mut() {
                    pending.to_position =
                        QueueModel::remap_queue_index(pending.to_position, from, to);
                }
                Ok(self.publish(state_tx, true))
            }
            QueueEdit::Remove { index } => {
                let index = index as usize;
                if index >= len {
                    return Err(ApiError::invalid_input("queue position out of range"));
                }
                if index == self.model.current_position() {
                    return Err(ApiError::invalid_input(
                        "cannot remove the playing position; skip or stop first",
                    ));
                }
                // The track a crossfade is already fading into is as much "the
                // playing position" as the outgoing one; removing it would land
                // the commit on its neighbour while its audio keeps running.
                if self
                    .pending_transition
                    .as_ref()
                    .is_some_and(|pending| pending.to_position == index)
                {
                    return Err(ApiError::invalid_input(
                        "cannot remove the track being faded into; skip or stop first",
                    ));
                }
                self.model.remove(index);
                if let Some(pending) = self.pending_transition.as_mut()
                    && index < pending.to_position
                {
                    pending.to_position -= 1;
                }
                Ok(self.publish(state_tx, true))
            }
        }
    }

    fn apply_radio_metadata(
        &mut self,
        token: u64,
        title: String,
        artist: Option<String>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        if token != self.current_token || title.trim().is_empty() {
            return;
        }
        let position = self.model.current_position();
        let Some(track) = self.model.track_at_mut(position) else {
            return;
        };
        if track.duration != u64::MAX {
            return;
        }
        track.title = title;
        if let Some(artist) = artist.filter(|artist| !artist.trim().is_empty()) {
            track.artist = artist;
        }
        self.publish(state_tx, false);
    }

    fn cancel_radio_task(&mut self) {
        if let Some(task) = self.radio_task.take() {
            task.abort();
        }
    }

    fn apply_queue_tracks(
        &mut self,
        tracks: Vec<Track>,
        mode: QueueMode,
        start_index: Option<usize>,
        shuffle: Option<bool>,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        let affected = tracks.len();
        match mode {
            QueueMode::Replace => {
                let mut candidate = self.model.clone();
                candidate.replace(tracks);
                if let Some(on) = shuffle {
                    candidate.set_shuffle(on);
                }
                let len = candidate.len();
                if len > 0 {
                    let start = start_index.unwrap_or_else(|| {
                        if candidate.shuffle() {
                            use rand::RngExt;
                            rand::rng().random_range(0..len)
                        } else {
                            0
                        }
                    });
                    let position = candidate.jump_to(start.min(len - 1));
                    self.start_immediate_load(candidate, position)?;
                } else {
                    self.model = candidate;
                    self.stop_playback();
                }
            }
            // Appending lands past every existing position, so a pending
            // crossfade target needs no remap.
            QueueMode::Append => self.model.add(tracks),
            QueueMode::PlayNext => match self.pending_transition.as_ref() {
                // Mid-crossfade the queue still reads as the outgoing track,
                // but "next" means after the one already fading in, or the
                // insertion would be skipped the moment the fade commits.
                Some(pending) => self.model.insert_at(pending.to_position + 1, tracks),
                None => self.model.insert_next(tracks),
            },
            QueueMode::Insert => {
                return Err(ApiError::invalid_input(
                    "insert mode requires an explicit logical position",
                ));
            }
        }
        tracing::info!(
            ?mode,
            affected,
            queue_len = self.model.len(),
            "queue updated"
        );
        Ok(self.publish(state_tx, true))
    }

    fn play_next(
        &mut self,
        allow_crossfade: bool,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<(), ApiError> {
        if allow_crossfade {
            if let NextOutcome::Play(idx) = self.model.peek_next()
                && !self.start_load(idx, true)
            {
                return Err(Self::unavailable_track_error());
            }
            return Ok(());
        }

        if self.pending_transition.is_some() {
            let _ = self.revert_transition();
        }
        let mut candidate = self.model.clone();
        match candidate.advance_next() {
            NextOutcome::Play(idx) => {
                self.start_immediate_load(candidate, idx)?;
            }
            NextOutcome::EndOfQueue => {
                self.model = candidate;
                // End of queue: kill an in-flight load so it cannot restart
                // playback later; the stale-Loaded rule catches a promoted one.
                self.cancel_load_task();
                self.pending_transition = None;
                self.set_intent(PlaybackIntent::Stopped);
                self.player.pause();
                if self.phase != ApiPhase::Ended {
                    self.phase = ApiPhase::Paused;
                }
                self.publish_position_anchor(state_tx, None, None, false);
            }
            NextOutcome::Empty => {}
        }
        Ok(())
    }

    fn play_previous(&mut self, state_tx: &watch::Sender<PlayerState>) -> Result<(), ApiError> {
        let idx = self.model.current_position();
        if self.revert_transition().is_some() {
            let candidate = self.model.clone();
            self.start_immediate_load(candidate, idx)?;
            return Ok(());
        }

        if self.config.back_behavior == config::BackBehavior::RewindThenPrev
            && self.displayed_position().as_secs() > 3
        {
            let _ = self.seek(0, state_tx);
            return Ok(());
        }

        let mut candidate = self.model.clone();
        if let Some(idx) = candidate.previous_position() {
            self.start_immediate_load(candidate, idx)?;
        }
        Ok(())
    }

    fn pause(&mut self, state_tx: &watch::Sender<PlayerState>) {
        let is_radio = self.current_track_is_radio();

        // Pausing mid-load cancels it, else a cancelled reply leaves intent
        // stuck Loading. Resolving crossfades revert whole; immediate loads
        // record a resume point. A running fade is merely frozen.
        if self.intent.is_loading() && self.revert_transition().is_none() {
            self.cancel_load_task();
            if !is_radio {
                self.store_pending_resume();
            }
            self.set_intent(PlaybackIntent::Stopped);
        }

        if is_radio {
            self.player.stop_for_transition();
            self.phase = ApiPhase::Idle;
        } else {
            self.player.pause();
            if self.phase != ApiPhase::Ended {
                self.phase = ApiPhase::Paused;
            }
        }
        self.publish_position_anchor(state_tx, None, None, false);
    }

    fn resume(&mut self, state_tx: &watch::Sender<PlayerState>) {
        let idx = self.model.current_position();
        let is_radio = self.current_track_is_radio();
        if is_radio || !self.player.can_resume() {
            if self.model.track_at(idx).is_some() {
                if !is_radio {
                    self.store_pending_resume();
                }
                self.start_load(idx, false);
            }
            return;
        }

        // Re-adopt a live engine session after a flow that quiesced playback
        // but kept it resumable, or the stale-session rule would stop it.
        let engine_token = self.player.session_token();
        if engine_token != 0 {
            self.set_intent(PlaybackIntent::Committed {
                token: engine_token,
            });
        }
        self.player.play_resume();
        self.phase = ApiPhase::Playing;
        self.maybe_record_recent();
        self.publish_position_anchor(state_tx, Some(engine_token), None, true);
    }

    fn stop(&mut self, state_tx: &watch::Sender<PlayerState>) {
        self.stop_playback();
        self.publish_position_anchor(state_tx, Some(0), Some(Duration::ZERO), false);
    }

    fn stop_playback(&mut self) {
        self.cancel_load_task();
        self.cancel_radio_task();
        self.pending_transition = None;
        self.armed_transition = None;
        self.pending_resume = None;
        self.set_intent(PlaybackIntent::Stopped);
        self.player.stop();
        self.phase = ApiPhase::Idle;
        self.buffered.clear();
        self.position = Some(PositionAnchor {
            ms: 0,
            at_ms: self.now_ms(),
            playing: false,
        });
        self.position_token = Some(0);
    }

    fn seek(
        &mut self,
        position_ms: u64,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<(), ApiError> {
        if self.current_track_is_radio() {
            return Err(ApiError::invalid_input("radio tracks are not seekable"));
        }

        let position = Duration::from_millis(position_ms);
        let token = if let Some(from_token) = self.revert_transition() {
            self.player.seek_for_session(position, from_token);
            from_token
        } else {
            self.player.seek(position);
            self.current_token
        };
        self.armed_transition = None;
        self.publish_position_anchor(
            state_tx,
            Some(token),
            Some(position),
            self.phase == ApiPhase::Playing,
        );
        Ok(())
    }

    fn apply_config(
        &mut self,
        config: config::AppConfig,
        changed: Vec<String>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        for key in &changed {
            match key.as_str() {
                "volume" => {
                    self.volume = config.volume.clamp(0.0, 1.0);
                    self.player.set_volume(self.volume);
                }
                "equalizer" => self.player.set_equalizer(config.equalizer.clone()),
                "channel_mode" => self.player.set_channel_mode(config.channel_mode),
                "sample_rate_mode" => self.player.set_sample_rate_mode(config.sample_rate_mode),
                "device_change_behavior" => {
                    self.player
                        .set_device_change_behavior(config.device_change_behavior);
                }
                _ => {}
            }
        }
        self.config = config;
        let _ = self.config_tx.send(self.config.clone());
        self.emit(ApiEvent::ConfigChanged { keys: changed });
        self.publish(state_tx, false);
    }

    fn apply_external_report(
        &mut self,
        track: Option<Track>,
        position_ms: u64,
        playing: bool,
        completed: bool,
        device: Option<String>,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<(), ApiError> {
        let Some(external) = self.external.as_mut() else {
            return Err(ApiError::new(
                api::ErrorCode::Conflict,
                "external playback is not claimed",
            ));
        };
        external.device = device;
        let mut committed = None;
        let changed = track.as_ref().map(|track| track.id.uid())
            != self.external_track.as_ref().map(|track| track.id.uid());
        if changed {
            self.external_completed_key = None;
            self.last_recent_key = None;
            self.external_track = track;
            if let Some(track) = self.external_track.clone() {
                let token = self.allocate_token();
                self.set_intent(PlaybackIntent::Committed { token });
                self.record_recent_track(track.clone());
                committed = Some((track, token));
            } else {
                self.set_intent(PlaybackIntent::Stopped);
            }
        }
        self.phase = if self.external_track.is_none() {
            ApiPhase::Idle
        } else if playing {
            ApiPhase::Playing
        } else {
            ApiPhase::Paused
        };
        self.position = self.external_track.as_ref().map(|_| PositionAnchor {
            ms: position_ms,
            at_ms: self.now_ms(),
            playing,
        });
        self.position_token = self.external_track.as_ref().map(|_| self.current_token);
        if completed
            && let Some(track) = self.external_track.clone()
            && self.external_completed_key.as_deref() != Some(track.id.uid().as_str())
        {
            self.external_completed_key = Some(track.id.uid());
            self.record_listen_track(track);
        }
        self.publish(state_tx, false);
        if let (Some(scrobbler), Some((track, token))) = (self.scrobbler.clone(), committed) {
            scrobbler.track_committed(track, token);
        }
        Ok(())
    }

    fn record_recent_track(&mut self, track: Track) {
        let uid = track.id.uid();
        if self.last_recent_key.as_deref() == Some(uid.as_str()) {
            return;
        }
        self.last_recent_key = Some(uid);
        if let Some(recorder) = self.recorder.clone() {
            tokio::spawn(async move {
                recorder.record_recent(&track).await;
            });
            self.emit(ApiEvent::LibraryInvalidated {
                table: api::Table::Recents,
                generation: self.rev,
            });
        }
    }

    fn record_listen_track(&self, track: Track) {
        if track.duration == u64::MAX {
            return;
        }
        if let Some(recorder) = self.recorder.clone() {
            tokio::spawn(async move {
                recorder.bump_listen_count(&track).await;
            });
        }
    }

    /// Record the committed track as recently played, once per session track.
    /// The invalidation event lets clients refresh recents immediately even
    /// though the durable write is fire-and-forget.
    fn maybe_record_recent(&mut self) {
        let Some(track) = self.model.current_track() else {
            return;
        };
        self.record_recent_track(track.clone());
    }

    /// Count a completed listen (auto-advance or crossfade arm), mirroring
    /// the pump: bumped for the outgoing track, never for radio.
    pub(super) fn record_listen_of_current(&mut self) {
        let Some(track) = self.model.current_track() else {
            return;
        };
        self.record_listen_track(track.clone());
    }

    /// Port of the app's `restore_queue_state`: stop, restore the model,
    /// seed a paused resume point at the saved progress.
    fn handle_restore(
        &mut self,
        snapshot: db::QueueSnapshot,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        self.cancel_load_task();
        self.cancel_radio_task();
        self.pending_transition = None;
        self.armed_transition = None;
        self.player.stop();
        self.phase = ApiPhase::Idle;
        self.set_intent(PlaybackIntent::Stopped);
        self.pending_resume = None;
        self.buffered.clear();
        self.last_recent_key = None;

        let restored = self.model.restore(
            snapshot.queue,
            snapshot.current_queue_index,
            snapshot.shuffle_order,
            snapshot.shuffle_enabled,
        );
        if let Some(position) = restored
            && let Some(track) = self.model.track_at(position).cloned()
        {
            let progress_secs = snapshot.progress_secs.min(track.duration);
            if track.duration != u64::MAX {
                self.pending_resume = Some(PendingResumeState {
                    track_key: track.id.uid(),
                    position_ms: progress_secs.saturating_mul(1000),
                });
            }
            self.publish_position_anchor(
                state_tx,
                None,
                Some(Duration::from_secs(progress_secs)),
                false,
            );
        }
        let ack = self.publish(state_tx, true);
        self.queue_dirty = false;
        Ok(ack)
    }

    fn snapshot(&self) -> db::QueueSnapshot {
        let progress_secs = if self.external.is_some() {
            self.position
                .map(|anchor| anchor.ms / 1000)
                .unwrap_or_default()
        } else if self.phase == ApiPhase::Playing {
            let secs = self.displayed_position().as_secs();
            (secs / PROGRESS_STEP_SECS) * PROGRESS_STEP_SECS
        } else {
            self.position
                .map(|anchor| anchor.ms / 1000)
                .unwrap_or_default()
        };
        db::QueueSnapshot {
            version: 1,
            queue: self.model.items().to_vec(),
            current_queue_index: self.model.current_position(),
            progress_secs,
            shuffle_order: self.model.shuffle_order().to_vec(),
            shuffle_enabled: self.model.shuffle(),
        }
    }

    fn cache_queue_snapshot(&self) {
        if let Ok(mut cached) = self.queue_snapshot.lock() {
            let snapshot = self.snapshot();
            let playing = self.phase == ApiPhase::Playing;
            let position_secs = if playing && self.external.is_none() {
                self.displayed_position().as_secs()
            } else {
                snapshot.progress_secs
            };
            *cached = CachedQueueSnapshot {
                snapshot,
                position_secs,
                at: Instant::now(),
                playing,
            };
        }
    }

    fn persist_async(&mut self) {
        let Some(writer) = self.queue_writer.clone() else {
            return;
        };
        let snapshot = self.snapshot();
        self.queue_dirty = false;
        let _ = writer.send((snapshot, None));
    }

    fn commit_transition_model(&mut self, token: u64) -> bool {
        let Some(pending) = self.pending_transition.take() else {
            return false;
        };
        if pending.to_token != token {
            self.pending_transition = Some(pending);
            return false;
        }
        if self.model.jump_to_position(pending.to_position).is_none() {
            tracing::warn!(
                position = pending.to_position,
                queue_len = self.model.len(),
                "the crossfade target left the queue; keeping the current pointer"
            );
        }
        true
    }

    fn commit_transition(&mut self, token: u64) -> bool {
        let Some(pending) = self
            .pending_transition
            .as_ref()
            .filter(|pending| pending.to_token == token)
        else {
            return false;
        };
        let outgoing_token = pending.from_token;
        let outgoing = self.model.current_track().cloned();
        self.armed_transition = Some(outgoing_token);
        if let Some(track) = outgoing {
            self.record_listen_track(track);
        }
        if !self.commit_transition_model(token) {
            return false;
        }
        self.player.commit_now_playing();
        true
    }

    fn start_immediate_load(&mut self, model: QueueModel, idx: usize) -> Result<(), ApiError> {
        let previous = std::mem::replace(&mut self.model, model);
        if !self.start_load(idx, false) {
            self.model = previous;
            return Err(Self::unavailable_track_error());
        }
        Ok(())
    }

    fn unavailable_track_error() -> ApiError {
        ApiError::new(
            api::ErrorCode::SourceUnreachable,
            "the selected queue track has no available playback source",
        )
    }

    /// Keep a resolving crossfade aimed at the same track after an insertion.
    /// The transition commits by position, so an unshifted target would land on
    /// whatever the inserted tracks pushed aside.
    fn shift_transition_target(&mut self, at: usize, by: usize) {
        if let Some(pending) = self.pending_transition.as_mut()
            && at <= pending.to_position
        {
            pending.to_position += by;
        }
    }

    /// Undo either a resolving crossfade or a running fade. The queue pointer
    /// never left the outgoing track, so dropping the pending target is the
    /// whole undo.
    fn revert_transition(&mut self) -> Option<u64> {
        let pending = self.pending_transition.take()?;
        if pending.stage == TransitionStage::Loading {
            self.cancel_load_task();
        }
        self.armed_transition = None;
        self.set_intent(PlaybackIntent::Committed {
            token: pending.from_token,
        });
        Some(pending.from_token)
    }

    fn cancel_load_task(&mut self) {
        if let Some((_, task)) = self.load_task.take() {
            task.abort();
        }
        self.player.cancel_pending_load();
    }

    fn allocate_token(&mut self) -> u64 {
        self.next_token = self.next_token.wrapping_add(1);
        self.next_token
    }

    /// Sole writer for playback intent and its plain token mirror.
    fn set_intent(&mut self, next: PlaybackIntent) {
        self.current_token = next.token();
        self.intent = next;
    }

    fn fail_load(&mut self, token: u64, error: impl std::fmt::Display) -> bool {
        let intent = self.intent;
        if intent.token() != token {
            return false;
        }
        self.error = Some(api::ErrorBody {
            code: api::ErrorCode::Internal,
            message: format!("couldn't load this track: {error}"),
            details: None,
        });
        self.buffered.clear();
        match intent {
            PlaybackIntent::Loading {
                crossfade: true,
                from_token,
                ..
            } => {
                self.pending_transition = None;
                self.armed_transition = None;
                self.set_intent(PlaybackIntent::Committed { token: from_token });
            }
            _ => {
                self.set_intent(PlaybackIntent::Stopped);
            }
        }
        true
    }

    fn pending_resume_seek(&self, track: &Track) -> (Option<Duration>, bool) {
        let pending = self.pending_resume.as_ref();
        let position = pending.and_then(|pending| {
            (pending.track_key == track.id.uid()).then(|| {
                Duration::from_millis(pending.position_ms.min(track.duration.saturating_mul(1000)))
            })
        });
        (position, pending.is_some())
    }

    fn store_pending_resume(&mut self) {
        if let Some(track) = self.model.current_track() {
            // The displayed progress, like the hooks progress signal: the live
            // engine position only while audibly playing; otherwise the last
            // published anchor, which is what a restore or a pause seeded.
            let position_ms = if self.phase == ApiPhase::Playing && !self.intent.is_loading() {
                self.displayed_position().as_millis() as u64
            } else {
                self.position
                    .map(|position| position.ms)
                    .unwrap_or_default()
            };
            self.pending_resume = Some(PendingResumeState {
                track_key: track.id.uid(),
                position_ms: position_ms.min(track.duration.saturating_mul(1000)),
            });
        }
    }

    fn stamp_probed_stream_info(
        &mut self,
        token: u64,
        idx: usize,
        duration_secs: Option<u64>,
        bitrate: Option<u32>,
    ) {
        // A queue edit may have shifted a crossfade candidate since its load
        // started, so probe results follow the remapped position.
        let idx = self
            .pending_transition
            .as_ref()
            .filter(|pending| pending.to_token == token)
            .map_or(idx, |pending| pending.to_position);
        if let Some(track) = self.model.track_at_mut(idx) {
            if let Some(duration) = duration_secs.filter(|duration| *duration > 0) {
                track.duration = duration;
            }
            if let Some(bitrate) = bitrate {
                track.bitrate = (bitrate / 1000) as u16;
            }
        }
    }

    fn handle_buffer_progress(
        &mut self,
        event: BufferProgressEvent,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        if event.token != self.current_token {
            return;
        }
        let Some(total) = event.total.filter(|total| *total > 0) else {
            return;
        };
        merge_buffered_range(
            &mut self.buffered,
            BufferedRange {
                start: event.start,
                end: event.end,
                total: Some(total),
            },
        );
        self.emit(ApiEvent::PlayerBuffered {
            token: event.token,
            ranges: self.buffered.clone(),
        });
        let _ = state_tx.send(self.build_state());
    }

    fn window(&mut self, page: Page) -> QueueWindow {
        let items = self
            .model
            .window(page.offset as usize, page.limit as usize)
            .into_iter()
            .map(|(position, track)| QueueItem {
                index: position as u32,
                track: crate::wire::track_info(&track, &self.config),
            })
            .collect();
        QueueWindow {
            rev: self.queue_rev,
            total: self.model.len() as u32,
            offset: page.offset,
            items,
        }
    }
}

#[derive(Clone, Copy)]
struct BufferProgressEvent {
    token: u64,
    start: u64,
    end: u64,
    total: Option<u64>,
}

fn merge_buffered_range(ranges: &mut Vec<BufferedRange>, incoming: BufferedRange) {
    let Some(total) = incoming.total.filter(|total| *total > 0) else {
        return;
    };
    if incoming.start >= incoming.end {
        return;
    }
    if ranges
        .first()
        .and_then(|range| range.total)
        .is_some_and(|old_total| old_total != total)
    {
        ranges.clear();
    }
    ranges.push(BufferedRange {
        end: incoming.end.min(total),
        ..incoming
    });
    ranges.sort_unstable_by_key(|range| range.start);

    let mut merged: Vec<BufferedRange> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
            continue;
        }
        merged.push(range);
    }
    *ranges = merged;
}

mod state;

mod local_api;
pub use local_api::LocalApi;

#[cfg(test)]
mod tests;
