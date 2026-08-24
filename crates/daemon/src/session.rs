//! The PlayerSession actor: sole owner of queue, transport, intent, and audio
//! engine state. Commands and engine events are serialized through one tokio
//! task, then projected into watch snapshots and broadcast API events.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, BufferedRange, CommandAck, FadingState, Intent, NowPlaying, Page,
    Phase as ApiPhase, PlayerCommand, PlayerState, PositionAnchor, QueueContext, QueueEdit,
    QueueItem, QueueMode, QueueSummary, QueueWindow, SetQueueRequest, TrackKind,
};
use player::engine::{Event as EngineEvent, Phase as EnginePhase, SourceFactory, Transition};
use player::player::{LoadArgs, NowPlayingMeta, Player, PlayerInitError};
use reader::Track;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use utils::playback_ref::{PlaybackItemRef, ResolvedStreamRef};

use crate::playback::network_factory;
use crate::queue_model::{NextOutcome, QueueModel};

mod reconciler;

pub const EVENT_BUFFER: usize = 512;
const POSITION_CORRECTION_INTERVAL: Duration = Duration::from_secs(10);
const MATERIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
const PERSIST_INTERVAL: Duration = Duration::from_secs(5);
const PROGRESS_STEP_SECS: u64 = 5;

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
}

impl Default for PlaybackServices {
    fn default() -> Self {
        Self {
            config: config::AppConfig::default(),
            active_source: None,
            station_registry: Arc::new(radio::registry::StationRegistry::default()),
            queue_store: None,
            recorder: None,
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
    SetQueue(
        SetQueueRequest,
        oneshot::Sender<Result<CommandAck, ApiError>>,
    ),
    Window(Page, oneshot::Sender<Result<QueueWindow, ApiError>>),
    RestoreQueue(
        Box<db::QueueSnapshot>,
        oneshot::Sender<Result<CommandAck, ApiError>>,
    ),
    SetConfig {
        config: Box<config::AppConfig>,
        changed: Vec<String>,
    },
    Persist(oneshot::Sender<()>),
    LoadPrepared(Box<Result<PreparedLoad, LoadFailure>>),
    LoadFinished(LoadFinished),
    BufferProgress(BufferProgressEvent),
}

#[derive(Clone)]
pub struct SessionHandle {
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
    state_rx: watch::Receiver<PlayerState>,
    config_rx: watch::Receiver<config::AppConfig>,
    events: broadcast::Sender<(u64, ApiEvent)>,
    seq: Arc<AtomicU64>,
    history: Arc<Mutex<VecDeque<(u64, ApiEvent)>>>,
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
        let (config_tx, config_rx) = watch::channel(services.config.clone());
        let engine_events = player.subscribe();
        player.set_volume(services.config.volume);
        player.set_channel_mode(services.config.channel_mode);
        player.set_equalizer(services.config.equalizer.clone());
        player.set_device_change_behavior(services.config.device_change_behavior);
        player.set_sample_rate_mode(services.config.sample_rate_mode);

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
            materializer,
            queue_store: services.queue_store,
            queue_dirty: false,
            recorder: services.recorder,
            last_recent_key: None,
            config_tx,
            config: services.config,
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
        self.request(|tx| SessionCmd::SetQueue(request, tx)).await
    }

    pub async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        self.request(|tx| SessionCmd::Window(page, tx)).await
    }

    pub async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError> {
        self.request(|tx| SessionCmd::Edit(edit, tx)).await
    }

    /// Restore a persisted queue: paused, with a resume point at the saved
    /// progress, exactly like the app's startup restore.
    pub async fn restore_queue(&self, snapshot: db::QueueSnapshot) -> Result<CommandAck, ApiError> {
        self.request(|tx| SessionCmd::RestoreQueue(Box::new(snapshot), tx))
            .await
    }

    /// Adopt a new config (a ConfigService patch): applies live audio
    /// settings and emits `config.changed`.
    pub fn set_config(&self, config: config::AppConfig, changed: Vec<String>) {
        let _ = self.cmd_tx.send(SessionCmd::SetConfig {
            config: Box::new(config),
            changed,
        });
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

struct PendingTransition {
    model: QueueModel,
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
    materializer: Arc<dyn QueueMaterializer>,
    queue_store: Option<Arc<dyn crate::persistence::QueueStore>>,
    queue_dirty: bool,
    recorder: Option<Arc<dyn PlaybackRecorder>>,
    last_recent_key: Option<String>,
    config_tx: watch::Sender<config::AppConfig>,
    config: config::AppConfig,
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
                _ = correction.tick(), if self.phase == ApiPhase::Playing => {
                    self.publish_position_anchor(&state_tx, None, None, true);
                }
                _ = persist.tick(), if self.queue_dirty && self.queue_store.is_some() => {
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
            SessionCmd::SetQueue(request, reply) => {
                let result = self.handle_set_queue(request, state_tx).await;
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
            SessionCmd::Persist(reply) => {
                if let Some(store) = self.queue_store.clone() {
                    let snapshot = self.snapshot();
                    self.queue_dirty = false;
                    store.save(snapshot).await;
                }
                let _ = reply.send(());
            }
            SessionCmd::LoadPrepared(result) => self.handle_prepared_load(*result, state_tx),
            SessionCmd::LoadFinished(result) => self.handle_load_finished(result, state_tx),
            SessionCmd::BufferProgress(event) => self.handle_buffer_progress(event, state_tx),
        }
    }

    fn handle_player_command(
        &mut self,
        command: PlayerCommand,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
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
            PlayerCommand::Next => self.play_next(false, state_tx),
            PlayerCommand::Previous => self.play_previous(state_tx),
            PlayerCommand::Stop => self.stop(state_tx),
            PlayerCommand::Seek { position_ms } => self.seek(position_ms, state_tx)?,
            PlayerCommand::SetVolume { volume } => {
                self.volume = volume.clamp(0.0, 1.0);
                self.player.set_volume(self.volume);
            }
            PlayerCommand::SetMode { shuffle, loop_mode } => {
                queue_changed = shuffle.is_some();
                if let Some(on) = shuffle {
                    self.model.set_shuffle(on);
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
        match edit {
            QueueEdit::Jump { index } => {
                let index = index as usize;
                let position_exists = self.model.track_at(index).is_some();
                if !position_exists {
                    return Err(ApiError::invalid_input("no track at that queue position"));
                }
                let physical = self
                    .model
                    .physical_index_of(index)
                    .ok_or_else(|| ApiError::invalid_input("no track at that queue position"))?;
                let position = self.model.jump_to(physical);
                self.start_load(position, false, None);
                Ok(self.publish(state_tx, true))
            }
            QueueEdit::Move { from, to } => {
                let (from, to) = (from as usize, to as usize);
                if from >= len || to >= len {
                    return Err(ApiError::invalid_input("queue position out of range"));
                }
                self.model.move_item(from, to);
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
                self.model.remove(index);
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

    async fn handle_set_queue(
        &mut self,
        request: SetQueueRequest,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        if request.mode != QueueMode::Replace
            && (request.start_index.is_some() || request.shuffle.is_some())
        {
            return Err(ApiError::invalid_input(
                "start_index and shuffle apply to mode \"replace\" only",
            ));
        }
        // Bounded so a hanging materializer (a slow source resolve) cannot
        // wedge the whole session command loop.
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
        match request.mode {
            QueueMode::Replace => {
                self.model.replace(tracks);
                if let Some(on) = request.shuffle {
                    self.model.set_shuffle(on);
                }
                let len = self.model.len();
                if len > 0 {
                    let start = request.start_index.map(|i| i as usize).unwrap_or_else(|| {
                        if self.model.shuffle() {
                            use rand::RngExt;
                            rand::rng().random_range(0..len)
                        } else {
                            0
                        }
                    });
                    let idx = self.model.jump_to(start.min(len - 1));
                    self.start_load(idx, false, None);
                }
            }
            QueueMode::Append => self.model.add(tracks),
            QueueMode::PlayNext => self.model.insert_next(tracks),
        }
        Ok(self.publish(state_tx, true))
    }

    fn play_next(&mut self, allow_crossfade: bool, state_tx: &watch::Sender<PlayerState>) {
        if allow_crossfade {
            let mut candidate = self.model.clone();
            if let NextOutcome::Play(idx) = candidate.advance_next() {
                self.start_load(idx, true, Some(candidate));
            }
            return;
        }

        if self.pending_transition.is_some() {
            let _ = self.revert_transition();
        }
        match self.model.advance_next() {
            NextOutcome::Play(idx) => self.start_load(idx, false, None),
            NextOutcome::EndOfQueue => {
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
    }

    fn play_previous(&mut self, state_tx: &watch::Sender<PlayerState>) {
        let idx = self.model.current_position();
        if self.revert_transition().is_some() {
            self.start_load(idx, false, None);
            return;
        }

        if self.config.back_behavior == config::BackBehavior::RewindThenPrev
            && self.displayed_position().as_secs() > 3
        {
            let _ = self.seek(0, state_tx);
            return;
        }

        if let Some(idx) = self.model.previous_position() {
            self.start_load(idx, false, None);
        }
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
                self.start_load(idx, false, None);
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
        self.cancel_load_task();
        self.cancel_radio_task();
        self.pending_transition = None;
        self.armed_transition = None;
        self.pending_resume = None;
        self.set_intent(PlaybackIntent::Stopped);
        self.player.stop();
        self.phase = ApiPhase::Idle;
        self.buffered.clear();
        self.publish_position_anchor(state_tx, Some(0), Some(Duration::ZERO), false);
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

    /// Two-phase load pipeline. Classification is mutation-free except stale
    /// offline-cache eviction; only after every early bail do we cancel the
    /// old load, allocate a token, and publish Loading intent.
    fn start_load(
        &mut self,
        idx: usize,
        allow_crossfade: bool,
        transition_model: Option<QueueModel>,
    ) {
        let source_model = transition_model.as_ref().unwrap_or(&self.model);
        let Some(track) = source_model.track_at(idx).cloned() else {
            return;
        };
        let track_key = track.id.uid();
        let (restore_seek, clear_pending_resume) = self.pending_resume_seek(&track);
        let use_crossfade = allow_crossfade
            && self.should_crossfade()
            && restore_seek.is_none_or(|position| position.is_zero());
        let crossfade_duration = Duration::from_secs(self.config.crossfade_seconds as u64);
        let item_ref = PlaybackItemRef::parse(&track_key);
        let is_radio = item_ref.is_radio();
        let is_server = item_ref.is_server();
        let item_id = item_ref.primary_id().unwrap_or_default().to_string();
        let stream_id = item_ref.stream_id().unwrap_or_default().to_string();

        let factory_override = self
            .factory_override
            .as_ref()
            .and_then(|provider| provider(&track));

        let offline_path = if factory_override.is_none() && is_server {
            let raw = self
                .config
                .offline_tracks
                .get(&item_id)
                .map(PathBuf::from)
                .filter(|path| path.exists());
            if let Some(path) = raw.as_ref() {
                let bad_ext = matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("audio") | Some("bin")
                );
                if bad_ext {
                    // Imported config paths are untrusted. Remove only the
                    // stale mapping; deleting the path could remove user data.
                    self.config.offline_tracks.remove(&item_id);
                    None
                } else {
                    raw
                }
            } else {
                raw
            }
        } else {
            None
        };

        let mut use_icy = false;
        let remote_ref = if factory_override.is_some() || offline_path.is_some() {
            None
        } else if is_radio {
            let station = self.station_registry.get(&item_id);
            use_icy = station.is_some_and(|station| !station.has_live_metadata());
            let cover = station
                .and_then(|station| match &station.metadata {
                    Some(radio::manifest::MetadataSourceDef::Static(static_meta)) => {
                        static_meta.resolve(&stream_id).2.map(str::to_string)
                    }
                    _ => None,
                })
                .unwrap_or_default();
            station
                .and_then(|station| station.streams.iter().find(|stream| stream.id == stream_id))
                .map(|stream| (stream.url.clone(), cover))
        } else if is_server && self.config.server.is_some() && self.active_source.is_some() {
            let cover = server::cover::track(&self.config, &track, 800)
                .map(|cover| cover.as_ref().to_string())
                .unwrap_or_default();
            Some((ResolvedStreamRef::pending_marker(&item_id), cover))
        } else {
            None
        };

        let local_path = if factory_override.is_none() && !is_server && !is_radio {
            track.id.local_path().map(PathBuf::from)
        } else {
            None
        };

        if factory_override.is_none()
            && offline_path.is_none()
            && local_path.is_none()
            && remote_ref.is_none()
        {
            return;
        }

        self.error = None;
        self.cancel_load_task();
        self.cancel_radio_task();
        if !use_crossfade {
            self.pending_transition = None;
        }
        let from_token = self.intent.token();
        let token = self.allocate_token();
        self.set_intent(PlaybackIntent::Loading {
            token,
            idx,
            crossfade: use_crossfade,
            from_token,
        });
        self.buffered.clear();

        if is_radio
            && let Some(station) = self
                .station_registry
                .get(&item_id)
                .filter(|station| station.has_live_metadata())
                .cloned()
        {
            use radio::provider::RadioMetadataProvider;
            let provider = radio::provider::DynamicProvider::new(station);
            let mut metadata_rx = provider.start(&stream_id);
            let cmd_tx = self.cmd_tx.clone();
            let handle = tokio::spawn(async move {
                while let Some(meta) = metadata_rx.recv().await {
                    let _ = cmd_tx.send(SessionCmd::RadioMetadata {
                        token,
                        title: meta.title,
                        artist: Some(meta.artist).filter(|artist| !artist.is_empty()),
                    });
                }
            });
            self.radio_task = Some(handle);
        }

        if use_crossfade {
            let Some(model) = transition_model else {
                self.fail_load(token, "crossfade transition has no queue candidate");
                return;
            };
            self.pending_transition = Some(PendingTransition {
                model,
                to_token: token,
                from_token,
                stage: TransitionStage::Loading,
            });
        }

        let cover_url = if offline_path.is_some() {
            server::cover::track(&self.config, &track, 800)
                .map(|cover| cover.as_ref().to_string())
                .unwrap_or_default()
        } else {
            remote_ref
                .as_ref()
                .map(|(_, cover)| cover.clone())
                .unwrap_or_default()
        };
        let artwork = if is_server || is_radio {
            Some(cover_url)
        } else {
            track.cover.clone()
        };

        if !use_crossfade {
            if is_server || is_radio {
                // Remote resolution deliberately silences the old session;
                // local files switch seamlessly inside the engine.
                self.player.stop_for_transition();
                self.phase = ApiPhase::Idle;
            }
            self.position = Some(PositionAnchor {
                ms: restore_seek.unwrap_or_default().as_millis() as u64,
                at_ms: self.now_ms(),
                playing: false,
            });
        }

        let classified = ClassifiedLoad {
            token,
            idx,
            track,
            is_radio,
            item_id,
            use_icy,
            factory_override,
            offline_path,
            local_path,
            remote_ref,
            active_source: self.active_source.clone(),
            artwork,
            transition: if use_crossfade {
                Transition::Crossfade(crossfade_duration)
            } else {
                Transition::Immediate
            },
            start_at: restore_seek.filter(|position| !position.is_zero()),
            clear_pending_resume,
            cmd_tx: self.cmd_tx.clone(),
        };
        let tx = self.cmd_tx.clone();
        let task = tokio::spawn(async move {
            let result = classified.prepare().await;
            let _ = tx.send(SessionCmd::LoadPrepared(Box::new(result)));
        });
        self.load_task = Some((token, task));
    }

    fn handle_prepared_load(
        &mut self,
        result: Result<PreparedLoad, LoadFailure>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(failure) => {
                if self.fail_load(failure.token, failure.message) {
                    self.publish(state_tx, false);
                }
                return;
            }
        };
        if self.intent.token() != prepared.token {
            return;
        }
        self.load_task = None;
        self.stamp_probed_stream_info(
            prepared.token,
            prepared.idx,
            prepared.duration_secs,
            prepared.bitrate,
        );

        let (reply_tx, reply_rx) = oneshot::channel();
        self.player.load(LoadArgs {
            token: prepared.token,
            factory: prepared.factory,
            meta: NowPlayingMeta {
                title: prepared.track.title,
                artist: prepared.track.artist,
                album: prepared.track.album,
                duration: Duration::from_secs(prepared.track.duration),
                artwork: prepared.artwork,
            },
            transition: prepared.transition,
            start_at: prepared.start_at,
            reply: Some(reply_tx),
        });
        let token = prepared.token;
        let tx = self.cmd_tx.clone();
        let task = tokio::spawn(async move {
            let result = reply_rx.await.ok();
            let _ = tx.send(SessionCmd::LoadFinished(LoadFinished {
                token,
                result,
                clear_pending_resume: prepared.clear_pending_resume,
            }));
        });
        self.load_task = Some((token, task));
        self.publish(state_tx, false);
    }

    fn handle_load_finished(
        &mut self,
        finished: LoadFinished,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        if self.intent.token() != finished.token {
            return;
        }
        self.load_task = None;
        match finished.result {
            Some(Ok(outcome)) => {
                self.set_intent(PlaybackIntent::Committed {
                    token: finished.token,
                });
                if finished.clear_pending_resume {
                    self.pending_resume = None;
                }
                self.maybe_record_recent();
                let matching_transition = self
                    .pending_transition
                    .as_ref()
                    .is_some_and(|pending| pending.to_token == finished.token);
                if matching_transition {
                    if outcome.crossfaded {
                        if let Some(pending) = self.pending_transition.as_mut() {
                            // Keep the visible queue/track outgoing until the
                            // authoritative TrackSwitched event.
                            pending.stage = TransitionStage::Fading;
                        }
                    } else {
                        self.commit_transition_model(finished.token);
                    }
                }
                self.publish(state_tx, false);
                if self.pending_transition.is_none()
                    && self.phase != ApiPhase::Idle
                    && self.position_token != Some(finished.token)
                {
                    self.publish_position_anchor(
                        state_tx,
                        Some(finished.token),
                        None,
                        self.phase == ApiPhase::Playing,
                    );
                }
            }
            Some(Err(error)) => {
                tracing::error!(error = %error, "playback failed");
                if self.fail_load(finished.token, error) {
                    self.publish(state_tx, false);
                }
            }
            None => {
                // Engine-side cancellation is owned by the command that
                // cancelled it; token guards reject any late completion.
            }
        }
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

    /// Record the committed track as recently played, once per session track.
    /// The invalidation event lets clients refresh recents immediately even
    /// though the durable write is fire-and-forget.
    fn maybe_record_recent(&mut self) {
        let Some(recorder) = self.recorder.clone() else {
            return;
        };
        let Some(track) = self.model.current_track() else {
            return;
        };
        let uid = track.id.uid();
        if self.last_recent_key.as_deref() == Some(uid.as_str()) {
            return;
        }
        self.last_recent_key = Some(uid);
        let track = track.clone();
        tokio::spawn(async move {
            recorder.record_recent(&track).await;
        });
        self.emit(ApiEvent::LibraryInvalidated {
            table: api::Table::Recents,
            generation: self.rev,
        });
    }

    /// Count a completed listen (auto-advance or crossfade arm), mirroring
    /// the pump: bumped for the outgoing track, never for radio.
    pub(super) fn record_listen_of_current(&mut self) {
        let Some(recorder) = self.recorder.clone() else {
            return;
        };
        let Some(track) = self.model.current_track() else {
            return;
        };
        if track.duration == u64::MAX {
            return;
        }
        let track = track.clone();
        tokio::spawn(async move {
            recorder.bump_listen_count(&track).await;
        });
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
        let progress_secs = if self.phase == ApiPhase::Playing {
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

    /// Fire-and-forget save off the actor thread; overlapping writes are
    /// last-write-wins on one SQLite row.
    fn persist_async(&mut self) {
        let Some(store) = self.queue_store.clone() else {
            return;
        };
        let snapshot = self.snapshot();
        self.queue_dirty = false;
        tokio::spawn(async move {
            store.save(snapshot).await;
        });
    }

    fn commit_transition_model(&mut self, token: u64) -> bool {
        let Some(pending) = self.pending_transition.take() else {
            return false;
        };
        if pending.to_token != token {
            self.pending_transition = Some(pending);
            return false;
        }
        self.model = pending.model;
        true
    }

    fn commit_transition(&mut self, token: u64) -> bool {
        if !self.commit_transition_model(token) {
            return false;
        }
        self.player.commit_now_playing();
        true
    }

    /// Undo either a resolving crossfade or a running fade. The queue model is
    /// still outgoing until commit, so discarding the candidate also undoes
    /// its history/index mutation.
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
        let model = self
            .pending_transition
            .as_mut()
            .filter(|pending| pending.to_token == token)
            .map(|pending| &mut pending.model)
            .unwrap_or(&mut self.model);
        if let Some(track) = model.track_at_mut(idx) {
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
                track,
            })
            .collect();
        QueueWindow {
            rev: self.queue_rev,
            total: self.model.len() as u32,
            offset: page.offset,
            items,
        }
    }

    fn publish(
        &mut self,
        state_tx: &watch::Sender<PlayerState>,
        queue_changed: bool,
    ) -> CommandAck {
        self.rev += 1;
        if queue_changed {
            self.queue_rev = self.rev;
        }
        self.queue_dirty = true;
        let state = self.build_state();
        if queue_changed {
            self.emit(ApiEvent::QueueChanged {
                rev: self.queue_rev,
                length: state.queue.length,
                index: state.queue.index,
            });
        }
        let _ = state_tx.send(state.clone());
        self.emit(ApiEvent::PlayerState(Box::new(state)));
        CommandAck { rev: self.rev }
    }

    /// Sole event egress: stamps the monotonic sequence, records the event in
    /// the replay ring, then broadcasts. SSE ids and `Last-Event-ID` resume
    /// both key off these sequences.
    fn emit(&self, event: ApiEvent) {
        let sequence = self.seq.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut history) = self.history.lock() {
            if history.len() >= EVENT_BUFFER {
                history.pop_front();
            }
            history.push_back((sequence, event.clone()));
        }
        let _ = self.events.send((sequence, event));
    }

    fn publish_position_anchor(
        &mut self,
        state_tx: &watch::Sender<PlayerState>,
        token: Option<u64>,
        position: Option<Duration>,
        playing: bool,
    ) {
        let token = token.unwrap_or_else(|| self.visible_token());
        let position = position.unwrap_or_else(|| self.displayed_position());
        let anchor = PositionAnchor {
            ms: position.as_millis() as u64,
            at_ms: self.now_ms(),
            playing,
        };
        self.position = Some(anchor);
        self.queue_dirty = true;
        self.position_token = Some(token);
        let _ = state_tx.send(self.build_state());
        self.emit(ApiEvent::PlayerPosition {
            token,
            position_ms: anchor.ms,
            at_ms: anchor.at_ms,
            playing,
        });
    }

    fn visible_token(&self) -> u64 {
        self.pending_transition
            .as_ref()
            .map(|pending| pending.from_token)
            .unwrap_or(self.current_token)
    }

    fn displayed_position(&self) -> Duration {
        if self.pending_transition.is_some()
            && let Some(position) = self.player.fading_position()
        {
            return position;
        }
        self.player.get_position()
    }

    fn current_track_is_radio(&self) -> bool {
        self.model
            .current_track()
            .is_some_and(|track| track.duration == u64::MAX)
    }

    fn should_crossfade(&self) -> bool {
        self.config.crossfade_seconds > 0
            && self.phase == ApiPhase::Playing
            && self.player.can_resume()
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn build_state(&self) -> PlayerState {
        let track = self.model.current_track().map(now_playing_from);
        let fading = self.pending_transition.as_ref().and_then(|pending| {
            (pending.stage == TransitionStage::Fading).then(|| FadingState {
                from_token: pending.from_token,
                track: track.clone().unwrap_or_default(),
                position_ms: self
                    .player
                    .fading_position()
                    .unwrap_or_default()
                    .as_millis() as u64,
            })
        });
        PlayerState {
            rev: self.rev,
            now_ms: self.now_ms(),
            phase: self.phase,
            intent: self.intent.into(),
            track,
            position: self.position,
            queue: QueueSummary {
                rev: self.queue_rev,
                length: self.model.len() as u32,
                index: (!self.model.is_empty()).then(|| self.model.current_position() as u32),
                shuffle: self.model.shuffle(),
                loop_mode: self.model.loop_mode(),
            },
            volume: self.volume,
            buffered: self.buffered.clone(),
            fading,
            error: self.error.clone(),
            ..Default::default()
        }
    }
}

enum ClassifiedSource {
    Factory(SourceFactory),
    Local(PathBuf),
    Cached {
        path: PathBuf,
        source: Option<server::source::ActiveSource>,
        item_id: String,
    },
    Remote {
        stream_ref: String,
        source: Option<server::source::ActiveSource>,
    },
}

struct ClassifiedLoad {
    token: u64,
    idx: usize,
    track: Track,
    is_radio: bool,
    item_id: String,
    use_icy: bool,
    factory_override: Option<SourceFactory>,
    offline_path: Option<PathBuf>,
    local_path: Option<PathBuf>,
    remote_ref: Option<(String, String)>,
    active_source: Option<server::source::ActiveSource>,
    artwork: Option<String>,
    transition: Transition,
    start_at: Option<Duration>,
    clear_pending_resume: bool,
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
}

impl ClassifiedLoad {
    async fn prepare(mut self) -> Result<PreparedLoad, LoadFailure> {
        let source = if let Some(factory) = self.factory_override.take() {
            ClassifiedSource::Factory(factory)
        } else if let Some(path) = self.local_path.take() {
            ClassifiedSource::Local(path)
        } else if let Some(path) = self.offline_path.take() {
            ClassifiedSource::Cached {
                path,
                source: self.active_source.clone(),
                item_id: self.item_id.clone(),
            }
        } else {
            let (stream_ref, _) = self.remote_ref.take().ok_or_else(|| LoadFailure {
                token: self.token,
                message: "classified load has no source".to_string(),
            })?;
            ClassifiedSource::Remote {
                stream_ref,
                source: self.active_source.clone(),
            }
        };

        let buffer_progress = (!self.is_radio).then(|| {
            let tx = self.cmd_tx.clone();
            let token = self.token;
            Arc::new(move |start, end, total| {
                let _ = tx.send(SessionCmd::BufferProgress(BufferProgressEvent {
                    token,
                    start,
                    end,
                    total,
                }));
            }) as utils::stream_buffer::BufferProgressCallback
        });

        let icy_tx = if self.is_radio && self.use_icy {
            let (tx, mut rx) = watch::channel(utils::icy::IcyMeta::default());
            let cmd_tx = self.cmd_tx.clone();
            let token = self.token;
            tokio::spawn(async move {
                while rx.changed().await.is_ok() {
                    let meta = rx.borrow_and_update().clone();
                    if meta.title.trim().is_empty() {
                        continue;
                    }
                    let (artist, title) = utils::icy::split_artist_title(&meta.title);
                    let _ = cmd_tx.send(SessionCmd::RadioMetadata {
                        token,
                        title,
                        artist,
                    });
                }
            });
            Some(tx)
        } else {
            None
        };

        let mut duration_secs = None;
        let mut bitrate = None;
        let factory: SourceFactory = match source {
            ClassifiedSource::Factory(factory) => factory,
            ClassifiedSource::Local(path) => Box::new(move || {
                player::decoder::open_file(&path).map_err(|error| error.to_string())
            }),
            ClassifiedSource::Cached {
                path,
                source,
                item_id,
            } => {
                // The fallback resolve blocks on the runtime captured here;
                // this closure executes on the runtime-less decode worker.
                let rt_handle = tokio::runtime::Handle::current();
                Box::new(move || match player::decoder::open_file(&path) {
                    Ok(parts) => Ok(parts),
                    Err(error) => {
                        tracing::warn!(error = %error, "cached file failed to open; falling back to the server stream");
                        let source = source
                            .as_ref()
                            .ok_or_else(|| "no active source for cache fallback".to_string())?;
                        let info = rt_handle
                            .block_on(source.resolve_stream(&item_id))
                            .map_err(|error| error.to_string())?;
                        network_factory(
                            info.url,
                            info.format,
                            info.user_agent,
                            false,
                            None,
                            rt_handle.clone(),
                            buffer_progress.clone(),
                        )()
                    }
                })
            }
            ClassifiedSource::Remote { stream_ref, source } => {
                let (stream_url, format, user_agent) = match ResolvedStreamRef::parse(&stream_ref) {
                    ResolvedStreamRef::Pending(item_id) => {
                        let source = source.ok_or_else(|| LoadFailure {
                            token: self.token,
                            message: "no active source for remote track".to_string(),
                        })?;
                        let info = source.resolve_stream(item_id).await.map_err(|error| {
                            tracing::error!(error = %error, "stream URL resolve failed");
                            LoadFailure {
                                token: self.token,
                                message: error.to_string(),
                            }
                        })?;
                        duration_secs = info.duration_secs;
                        bitrate = info.bitrate;
                        (info.url, info.format, info.user_agent)
                    }
                    ResolvedStreamRef::SoundCloudHls(_)
                    | ResolvedStreamRef::AppleMusicFmp4(_)
                    | ResolvedStreamRef::Direct(_) => (stream_ref, None, None),
                };

                // The factory runs on the decode worker (no runtime), so hand
                // every blocking stream/decrypt path this task's handle.
                let rt_handle = tokio::runtime::Handle::current();
                network_factory(
                    stream_url,
                    format,
                    user_agent,
                    self.is_radio,
                    icy_tx,
                    rt_handle,
                    buffer_progress,
                )
            }
        };

        if let Some(duration) = duration_secs.filter(|duration| *duration > 0) {
            self.track.duration = duration;
        }
        if let Some(bits_per_second) = bitrate {
            self.track.bitrate = (bits_per_second / 1000) as u16;
        }

        Ok(PreparedLoad {
            token: self.token,
            idx: self.idx,
            track: self.track,
            factory,
            artwork: self.artwork,
            transition: self.transition,
            start_at: self.start_at,
            clear_pending_resume: self.clear_pending_resume,
            duration_secs,
            bitrate,
        })
    }
}

struct PreparedLoad {
    token: u64,
    idx: usize,
    track: Track,
    factory: SourceFactory,
    artwork: Option<String>,
    transition: Transition,
    start_at: Option<Duration>,
    clear_pending_resume: bool,
    duration_secs: Option<u64>,
    bitrate: Option<u32>,
}

struct LoadFailure {
    token: u64,
    message: String,
}

struct LoadFinished {
    token: u64,
    result: Option<Result<player::engine::LoadOutcome, String>>,
    clear_pending_resume: bool,
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

fn engine_phase(phase: EnginePhase) -> ApiPhase {
    match phase {
        EnginePhase::Idle => ApiPhase::Idle,
        EnginePhase::Playing => ApiPhase::Playing,
        EnginePhase::Paused => ApiPhase::Paused,
        EnginePhase::Ended => ApiPhase::Ended,
    }
}

/// Translate the internal track model to the wire summary. The radio duration
/// sentinel is contained at this boundary.
fn now_playing_from(track: &Track) -> NowPlaying {
    let radio = track.duration == u64::MAX;
    NowPlaying {
        key: track.id.uid(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration_ms: (!radio).then(|| track.duration.saturating_mul(1000)),
        khz: track.khz,
        bitrate: track.bitrate,
        artwork: None,
        kind: if radio {
            TrackKind::Radio
        } else {
            TrackKind::Normal
        },
        seekable: !radio,
    }
}

/// In-process implementation of [`api::KopuzApi`] over a running session.
pub struct LocalApi {
    session: SessionHandle,
    library: Option<Arc<crate::library::LibraryService>>,
    config: Option<Arc<crate::config_service::ConfigService>>,
}

impl LocalApi {
    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            library: None,
            config: None,
        }
    }

    pub fn with_library(mut self, library: Arc<crate::library::LibraryService>) -> Self {
        self.library = Some(library);
        self
    }

    pub fn with_config(mut self, config: Arc<crate::config_service::ConfigService>) -> Self {
        self.config = Some(config);
        self
    }
}

#[async_trait::async_trait]
impl api::KopuzApi for LocalApi {
    async fn player_state(&self) -> Result<PlayerState, ApiError> {
        Ok(self.session.state())
    }

    async fn player_command(&self, command: PlayerCommand) -> Result<CommandAck, ApiError> {
        self.session.player_command(command).await
    }

    async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        self.session.queue_window(page).await
    }

    async fn set_queue(&self, request: SetQueueRequest) -> Result<CommandAck, ApiError> {
        self.session.set_queue(request).await
    }

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError> {
        self.session.queue_edit(edit).await
    }

    async fn tracks(
        &self,
        filter: api::TrackFilter,
        page: Page,
    ) -> Result<api::TrackPage, ApiError> {
        match &self.library {
            Some(library) => library.tracks(filter, page).await,
            None => Err(ApiError::unsupported(
                "this daemon runs without a library service",
            )),
        }
    }

    async fn config(&self) -> Result<api::ConfigView, ApiError> {
        match &self.config {
            Some(service) => service.view().await,
            None => Err(ApiError::unsupported(
                "this daemon runs without a config service",
            )),
        }
    }

    async fn patch_config(&self, patch: serde_json::Value) -> Result<api::ConfigView, ApiError> {
        let Some(service) = &self.config else {
            return Err(ApiError::unsupported(
                "this daemon runs without a config service",
            ));
        };
        let (view, updated, changed) = service.patch(patch).await?;
        self.session.set_config(updated, changed);
        Ok(view)
    }

    fn events(&self) -> api::EventStream {
        use futures_util::StreamExt;
        let rx = self.session.subscribe();
        futures_util::stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Ok((_, event)) => Some((event, rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => Some((ApiEvent::Resync, rx)),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex};

    use api::{ErrorCode, KopuzApi, LoopMode};
    use futures_util::StreamExt;
    use player::engine::{AudioSink, DataCallback, DataCallbackFactory, SinkConfig};

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
        let player = Player::try_with_sink(Box::new(FakeSink(sink.clone())))
            .expect("headless player starts");
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
            state.phase == ApiPhase::Playing
                && matches!(state.intent, Intent::Committed { token: 1 })
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
        let player = Player::try_with_sink(Box::new(FakeSink(sink.clone())))
            .expect("headless player starts");
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
        let player = Player::try_with_sink(Box::new(FakeSink(sink.clone())))
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
}
