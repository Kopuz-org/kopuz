//! The PlayerSession actor: sole owner of queue, transport, intent, and audio
//! engine state. Commands and engine events are serialized through one tokio
//! task, then projected into watch snapshots and broadcast API events.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::{
    ApiError, ApiEvent, BufferedRange, CommandAck, FadingState, Intent, Page, Phase as ApiPhase,
    PlayerCommand, PlayerState, PositionAnchor, QueueContext, QueueEdit, QueueItem, QueueMode,
    QueueSummary, QueueWindow, SetQueueRequest,
};
use player::engine::{Event as EngineEvent, Phase as EnginePhase, SourceFactory, Transition};
use player::player::{LoadArgs, NowPlayingMeta, Player, PlayerInitError};
use reader::Track;
use server::playback_ref::{PlaybackItemRef, ResolvedStreamRef};
use server::source::RadioPage;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::playback::network_factory;
use crate::queue_model::{NextOutcome, QueueModel};

mod load;
mod origin;
mod radio_feed;
mod reconciler;
mod stations;
mod versions;
pub(crate) use stations::has_unrecorded_station;

use load::{LoadFailure, LoadFinished, PreparedLoad};
use radio_feed::RadioFeed;

/// Events buffered per subscriber before it is considered lagged.
pub const EVENT_BUFFER: usize = 512;
const POSITION_CORRECTION_INTERVAL: Duration = Duration::from_secs(10);
const MATERIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
const PERSIST_INTERVAL: Duration = Duration::from_secs(5);
const REMOTE_PUSH_TIMEOUT: Duration = Duration::from_secs(3);
const PROGRESS_STEP_SECS: u64 = 5;

/// The queue as `api::QueueSnapshot` reports it: tracks in play order, `position` a play-order index.
#[derive(Debug, Clone, Default)]
pub(crate) struct QueueMirrorSnapshot {
    pub tracks: Vec<Track>,
    /// Unshuffled index of each play-order position; empty while shuffle is off.
    pub shuffle_order: Vec<usize>,
    pub position: usize,
    pub shuffle: bool,
    pub context: Option<QueueContext>,
}

/// Resolves a wire queue context into concrete tracks daemon-side.
#[async_trait::async_trait]
pub trait QueueMaterializer: Send + Sync {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError>;

    /// The tracks, and for a radio that can go on, the cursor of its next page.
    async fn materialize_queue(&self, context: &QueueContext) -> Result<RadioPage, ApiError> {
        Ok(RadioPage::last(self.materialize(context).await?))
    }

    async fn more_radio(&self, _cursor: &str) -> Result<RadioPage, ApiError> {
        Ok(RadioPage::default())
    }

    /// Make a restored queue's rows addressable by key, since some were listed once and never stored.
    fn register_restored(&self, _tracks: &[Track]) {}
}

/// Durable playback bookkeeping: recents on commit, listen counts once a
/// track has played long enough to scrobble, credited to the source that was active
/// when it played. Implemented over that source; tests inject a stub.
#[async_trait::async_trait]
pub trait PlaybackRecorder: Send + Sync {
    async fn record_recent(&self, source: Option<&server::source::ActiveSource>, track: &Track);
    async fn bump_listen_count(&self, source: Option<&server::source::ActiveSource>, track: &Track);
}

/// Playback dependencies that will eventually be owned by daemon services.
/// Keeping them together lets the actor land before ConfigService and source
/// lifecycle extraction do.
pub struct PlaybackServices {
    pub settings: config::Settings,
    /// The level and mute the engine was left at, as the database stored them.
    pub volume: f32,
    pub muted: bool,
    pub sources: Arc<server::sources::Sources>,
    pub active_source: Option<server::source::ActiveSource>,
    pub station_registry: Arc<radio::registry::StationRegistry>,
    pub queue_store: Option<Arc<dyn crate::persistence::QueueStore>>,
    pub recorder: Option<Arc<dyn PlaybackRecorder>>,
    pub scrobbler: Option<Arc<crate::scrobbler::Scrobbler>>,
    pub offline: crate::offline::OfflineIndex,
}

impl Default for PlaybackServices {
    fn default() -> Self {
        Self {
            settings: config::Settings::default(),
            volume: 1.0,
            muted: false,
            sources: Arc::default(),
            active_source: None,
            station_registry: Arc::new(radio::registry::StationRegistry::default()),
            queue_store: None,
            recorder: None,
            scrobbler: None,
            offline: crate::offline::OfflineIndex::default(),
        }
    }
}

/// Only a move to another track or a stop is worth a network call.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RemoteQueueMark {
    /// What the server stores of the queue, its track ids in play order, so a cover or counterpart learned is no edit.
    tracks: u64,
    current_key: Option<String>,
    playing: bool,
}

/// The mark of the last push the server took, set by the writer once it has.
type SyncedMark = Arc<std::sync::Mutex<Option<RemoteQueueMark>>>;

struct RemoteQueuePush {
    source: server::source::ActiveSource,
    /// Whose server this is, for the fingerprint the adopt at boot compares against.
    store_key: config::Source,
    queue: Vec<Track>,
    current_index: usize,
    position_ms: u64,
    mark: RemoteQueueMark,
}

impl RemoteQueuePush {
    async fn send(self, synced: &SyncedMark, store: Option<&dyn crate::persistence::QueueStore>) {
        let saved = match self
            .source
            .save_play_queue(&self.queue, self.current_index, self.position_ms)
            .await
        {
            Ok(saved) => saved,
            Err(error) => {
                tracing::debug!(%error, "play queue push failed");
                return;
            }
        };
        if saved && let Some(store) = store {
            let current = self
                .queue
                .get(self.current_index)
                .map(|track| track.id.key());
            let fingerprint = match self.queue.is_empty() {
                true => crate::persistence::empty_queue_fingerprint(),
                false => crate::persistence::queue_fingerprint(
                    self.queue.iter().map(|track| track.id.key()),
                    current.as_deref(),
                    self.position_ms,
                ),
            };
            store.set_synced_remote(&self.store_key, &fingerprint).await;
        }
        if let Ok(mut synced) = synced.lock() {
            *synced = Some(self.mark);
        }
    }
}

enum RemoteQueueJob {
    Push(RemoteQueuePush),
    /// Answered once every push sent before it has been tried.
    Flush(oneshot::Sender<()>),
}

/// The only task that writes the server's queue, so a slow push can never land after a newer one.
async fn write_remote_queue(
    mut jobs: mpsc::UnboundedReceiver<RemoteQueueJob>,
    synced: SyncedMark,
    store: Option<Arc<dyn crate::persistence::QueueStore>>,
) {
    while let Some(job) = jobs.recv().await {
        // Pushes that queued up behind a slow one are stale; only the newest is sent.
        let mut latest = None;
        let mut flushes = Vec::new();
        let mut next = Some(job);
        while let Some(job) = next {
            match job {
                RemoteQueueJob::Push(push) => latest = Some(push),
                RemoteQueueJob::Flush(done) => flushes.push(done),
            }
            next = jobs.try_recv().ok();
        }
        if let Some(push) = latest {
            push.send(&synced, store.as_deref()).await;
        }
        for done in flushes {
            let _ = done.send(());
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
    RadioTopUp {
        feed: u64,
        cursor: String,
        result: Box<Result<RadioPage, ApiError>>,
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
    AdoptQueue {
        snapshot: Box<db::QueueSnapshot>,
        source: config::Source,
        queue_rev: u64,
        reply: oneshot::Sender<Result<Option<CommandAck>, ApiError>>,
    },
    SetSettings {
        settings: Box<config::Settings>,
        changed: Vec<config::SettingKey>,
        revision: u64,
    },
    SetSources {
        sources: Arc<server::sources::Sources>,
        adopted: Option<oneshot::Sender<()>>,
    },
    QueueUnread,
    /// Fetched off the actor.
    AutoplayRadio {
        queue_rev: u64,
        seed: String,
        result: Result<RadioPage, ApiError>,
    },
    PreviewEqualizer(config::EqualizerSettings),
    PreviewReplayGain(config::ReplayGainSettings),
    Emit(Box<ApiEvent>),
    AttachExternal(crate::external::SharedExternalPlayer),
    /// The integration that can play what the engine cannot, offered
    /// once at boot and consulted on every load.
    SetExternalSink(crate::external::SharedExternalPlayer),
    DetachExternal,
    ExternalReport(Box<crate::external::ExternalReport>),
    ExternalArtworkFetched(String),
    SetStationRegistry(Arc<radio::registry::StationRegistry>),
    SetActiveSource(Option<server::source::ActiveSource>),
    ClearError,
    QueueMirror(oneshot::Sender<QueueMirrorSnapshot>),
    Persist(oneshot::Sender<()>),
    LoadPrepared(Box<Result<PreparedLoad, LoadFailure>>),
    LoadFinished(LoadFinished),
    BufferProgress(BufferProgressEvent),
    TrackMetadata {
        source: config::Source,
        tracks: Vec<Track>,
    },
    AlbumCovers {
        source: config::Source,
        covers: std::collections::HashMap<String, String>,
    },
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    ArtworkFetched {
        token: u64,
        meta: Box<NowPlayingMeta>,
    },
    CounterpartFound {
        uid: String,
        counterpart: Box<reader::Counterpart>,
    },
}

#[derive(Clone)]
pub struct SessionHandle {
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
    state_rx: watch::Receiver<PlayerState>,
    settings_rx: watch::Receiver<config::Settings>,
    sources_rx: watch::Receiver<Arc<server::sources::Sources>>,
    events: broadcast::Sender<ApiEvent>,
    materializer: Arc<dyn QueueMaterializer>,
    offline: crate::offline::OfflineIndex,
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
        let (settings_tx, settings_rx) = watch::channel(services.settings.clone());
        let (sources_tx, sources_rx) = watch::channel(services.sources.clone());
        let engine_events = player.subscribe();
        let volume = services.volume.clamp(0.0, 1.0);
        player.set_volume(if services.muted { 0.0 } else { volume });
        player.set_channel_mode(services.settings.playback.channel_mode);
        player.set_equalizer(services.settings.playback.equalizer.clone());
        player.set_replay_gain(services.settings.playback.replay_gain);
        player.set_device_change_behavior(services.settings.playback.device_change_behavior);
        player.set_sample_rate_mode(services.settings.playback.sample_rate_mode);
        let remote_synced = SyncedMark::default();
        let (remote_writer, remote_jobs) = mpsc::unbounded_channel();
        tokio::spawn(write_remote_queue(
            remote_jobs,
            remote_synced.clone(),
            services.queue_store.clone(),
        ));

        let mut session = Session {
            model: QueueModel::default(),
            player,
            intent: PlaybackIntent::Stopped,
            next_token: 0,
            current_token: 0,
            pending_resume: None,
            engine_released: false,
            pending_transition: None,
            armed_transition: None,
            load_task: None,
            radio_task: None,
            radio_feed: None,
            next_feed_id: 0,
            autoplay_task: None,
            phase: ApiPhase::Idle,
            position: None,
            position_token: None,
            stalled: None,
            buffered: Vec::new(),
            error: None,
            rev: 0,
            queue_rev: 0,
            volume,
            muted: services.muted,
            origin: None,
            epoch: Instant::now(),
            events: events.clone(),
            materializer: materializer.clone(),
            queue_store: services.queue_store,
            queue_dirty: false,
            queue_unread: false,
            persisting: None,
            remote_synced,
            remote_loaded: None,
            remote_writer,
            recorder: services.recorder,
            published: None,
            scrobbler: services.scrobbler,
            last_recent_key: None,
            external: None,
            sink: None,
            settings_tx,
            settings: services.settings,
            sources_tx,
            sources: services.sources,
            active_source: services.active_source,
            station_registry: services.station_registry,
            queue_stations: Default::default(),
            offline: services.offline.clone(),
            cmd_tx: cmd_tx.clone(),
            factory_override,
            counterparts: std::collections::HashMap::new(),
        };
        // A fresh start is not a clear, and must not wipe what another device saved before the boot adopt reads it.
        session.remote_loaded = Some(session.remote_mark());
        let (state_tx, state_rx) = watch::channel(session.build_state());
        tokio::spawn(session.run(cmd_rx, engine_events, state_tx));
        Self {
            cmd_tx,
            state_rx,
            settings_rx,
            sources_rx,
            events,
            materializer,
            offline: services.offline,
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

    /// Resolve a track the database has never seen: live search results live
    /// only in the library's transient cache, which the materializer can read.
    pub async fn materialize_track(&self, key: String) -> Result<Track, ApiError> {
        self.materializer
            .materialize(&QueueContext::Tracks { keys: vec![key] })
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::not_found("unknown track key"))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ApiEvent> {
        self.events.subscribe()
    }

    /// The session's live settings copy, seeded at spawn and updated by `set_settings`, which integration tasks watch instead of polling.
    pub fn settings_watch(&self) -> watch::Receiver<config::Settings> {
        self.settings_rx.clone()
    }

    /// The session's live sources copy: seeded at spawn, updated by `set_sources`.
    pub fn sources_watch(&self) -> watch::Receiver<Arc<server::sources::Sources>> {
        self.sources_rx.clone()
    }

    /// The sources as the session last adopted them.
    pub fn sources(&self) -> Arc<server::sources::Sources> {
        self.sources_rx.borrow().clone()
    }

    /// The downloaded copies every wire row and offline playback reads.
    pub fn offline(&self) -> &crate::offline::OfflineIndex {
        &self.offline
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

    /// Hand playback to an integration the engine cannot drive. The engine is
    /// stopped and transport commands route to it until [`Self::detach_external`].
    pub fn attach_external(&self, player: crate::external::SharedExternalPlayer) {
        let _ = self.cmd_tx.send(SessionCmd::AttachExternal(player));
    }

    pub fn detach_external(&self) {
        let _ = self.cmd_tx.send(SessionCmd::DetachExternal);
    }

    /// Offer the integration that plays what the engine cannot. Consulted
    /// on every load, so a Spotify track in the queue reaches it.
    pub fn set_external_sink(&self, player: crate::external::SharedExternalPlayer) {
        let _ = self.cmd_tx.send(SessionCmd::SetExternalSink(player));
    }

    /// Report what the attached integration is playing.
    pub fn report_external(&self, report: crate::external::ExternalReport) {
        let _ = self
            .cmd_tx
            .send(SessionCmd::ExternalReport(Box::new(report)));
    }

    /// Restore a persisted queue: paused, with a resume point at the saved
    /// progress, exactly like the app's startup restore.
    pub async fn restore_queue(&self, snapshot: db::QueueSnapshot) -> Result<CommandAck, ApiError> {
        self.request(|tx| SessionCmd::RestoreQueue(Box::new(snapshot), tx))
            .await
    }

    /// Restore `snapshot`, fetched from `source`, only while that source is
    /// still active and the queue is still at `queue_rev`, the revision read
    /// before the fetch; `None` when either moved on.
    pub async fn adopt_queue(
        &self,
        snapshot: db::QueueSnapshot,
        source: &config::Source,
        queue_rev: u64,
    ) -> Result<Option<CommandAck>, ApiError> {
        self.request(|reply| SessionCmd::AdoptQueue {
            snapshot: Box::new(snapshot),
            source: source.clone(),
            queue_rev,
            reply,
        })
        .await
    }

    /// Emit an event through the session's sequenced stream (services use
    /// this for invalidations, job progress, and notices).
    pub fn emit_event(&self, event: ApiEvent) {
        let _ = self.cmd_tx.send(SessionCmd::Emit(Box::new(event)));
    }

    /// Tell clients a table changed. Every service that writes the library
    /// goes through here, so there is one place to look for who dirties what.
    pub fn invalidate(&self, table: api::Table) {
        self.emit_event(ApiEvent::LibraryInvalidated { table });
    }

    pub(crate) fn update_track_metadata(&self, source: &config::Source, tracks: Vec<Track>) {
        let _ = self.cmd_tx.send(SessionCmd::TrackMetadata {
            source: source.clone(),
            tracks,
        });
    }

    pub fn update_album_covers(&self, source: &config::Source, albums: &[reader::Album]) {
        let covers = albums
            .iter()
            .filter_map(|album| {
                album
                    .cover_path
                    .as_ref()
                    .map(|path| (album.id.clone(), path.to_string_lossy().into_owned()))
            })
            .collect();
        let _ = self.cmd_tx.send(SessionCmd::AlbumCovers {
            source: source.clone(),
            covers,
        });
    }

    /// Announce whether a source is reachable, so every client shows the same
    /// connection state rather than each probing on its own.
    pub fn publish_source_status(&self, source: &str, state: api::SourceState) {
        self.emit_event(ApiEvent::SourceStatus {
            source: source.to_string(),
            state,
        });
    }

    /// Forget the last playback failure. A failure belongs to the source that
    /// produced it, so switching away is what makes it stale rather than
    /// something a frontend has to decide by looking at the message.
    pub fn clear_error(&self) {
        let _ = self.cmd_tx.send(SessionCmd::ClearError);
    }

    /// Stop and empty the queue. What is loaded belongs to the source that
    /// was active, so switching away has to leave nothing playing from it.
    pub async fn reset_playback(&self) -> Result<(), ApiError> {
        self.player_command(PlayerCommand::Stop).await?;
        self.set_queue(SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks { keys: Vec::new() },
            start_index: None,
            shuffle: None,
        })
        .await?;
        Ok(())
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

    /// The whole queue in play order plus its permutation, behind `queue_snapshot`.
    pub(crate) async fn queue_mirror(&self) -> QueueMirrorSnapshot {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(SessionCmd::QueueMirror(tx)).is_err() {
            return QueueMirrorSnapshot::default();
        }
        rx.await.unwrap_or_default()
    }

    /// A queued track by key. The queue is where a row the database has never
    /// seen still exists -- a catalog listing played straight from the
    /// network -- so artwork and metadata for it resolve from here.
    pub async fn queued_track(&self, key: &str) -> Option<Track> {
        self.queue_mirror()
            .await
            .tracks
            .into_iter()
            .find(|track| track.id.key() == key)
    }

    /// The stored queue could not be read, so nothing may be saved over it until a new queue is built.
    pub fn hold_unread_queue(&self) {
        let _ = self.cmd_tx.send(SessionCmd::QueueUnread);
    }

    /// Let the engine play `equalizer` without storing it or touching the settings.
    pub fn preview_equalizer(&self, equalizer: config::EqualizerSettings) {
        let _ = self.cmd_tx.send(SessionCmd::PreviewEqualizer(equalizer));
    }

    /// Let the engine play `replay_gain` without storing it or touching the settings.
    pub fn preview_replay_gain(&self, replay_gain: config::ReplayGainSettings) {
        let _ = self.cmd_tx.send(SessionCmd::PreviewReplayGain(replay_gain));
    }

    /// Adopt new settings (a ConfigService change): applies live audio settings and emits `config.changed`.
    pub(crate) fn set_settings(
        &self,
        settings: config::Settings,
        changed: Vec<config::SettingKey>,
        revision: u64,
    ) {
        let _ = self.cmd_tx.send(SessionCmd::SetSettings {
            settings: Box::new(settings),
            changed,
            revision,
        });
    }

    /// Adopt new sources (a SourceService change), parking the queue under the source left when the active one moves; the answer comes once every reader sees them.
    pub(crate) fn adopt_sources(
        &self,
        sources: Arc<server::sources::Sources>,
    ) -> oneshot::Receiver<()> {
        let (adopted, done) = oneshot::channel();
        let _ = self.cmd_tx.send(SessionCmd::SetSources {
            sources,
            adopted: Some(adopted),
        });
        done
    }

    #[cfg(test)]
    pub(crate) fn set_sources(&self, sources: Arc<server::sources::Sources>) {
        let _ = self.cmd_tx.send(SessionCmd::SetSources {
            sources,
            adopted: None,
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
        /// A crossfade or gapless candidate; the outgoing track keeps playing.
        deferred: bool,
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
    Queued,
}

/// A crossfade in flight. The queue itself is not forked: only the logical
/// position the pointer will move to once the engine reports the switch, so
/// queue edits arriving mid-fade land on the one live model and survive the
/// commit (issue #656). Structural edits remap `to_position` the same way they
/// remap `current`.
struct PendingTransition {
    to_position: usize,
    to_key: String,
    to_token: u64,
    from_token: u64,
    stage: TransitionStage,
    /// Nothing is audible before the switch, so a queue edit re-arms it instead.
    gapless: bool,
}

struct Session {
    model: QueueModel,
    player: Player,
    intent: PlaybackIntent,
    next_token: u64,
    current_token: u64,
    pending_resume: Option<PendingResumeState>,
    /// A paused version switch stopped the engine for good: whatever session
    /// it still reports is the old cut, so resuming loads the swapped one.
    engine_released: bool,
    pending_transition: Option<PendingTransition>,
    armed_transition: Option<u64>,
    load_task: Option<(u64, JoinHandle<()>)>,
    radio_task: Option<JoinHandle<()>>,
    radio_feed: Option<RadioFeed>,
    next_feed_id: u64,
    autoplay_task: Option<JoinHandle<()>>,
    phase: ApiPhase,
    position: Option<PositionAnchor>,
    position_token: Option<u64>,
    /// The session the engine reports stalled: its anchor stands still until audio flows again.
    stalled: Option<u64>,
    buffered: Vec<BufferedRange>,
    error: Option<api::ErrorBody>,
    rev: u64,
    queue_rev: u64,
    /// The level unmuting returns to; the engine plays at zero while `muted`.
    volume: f32,
    muted: bool,
    /// The container the queue was built from; None for a raw track list.
    origin: Option<QueueContext>,
    epoch: Instant,
    events: broadcast::Sender<ApiEvent>,
    materializer: Arc<dyn QueueMaterializer>,
    queue_store: Option<Arc<dyn crate::persistence::QueueStore>>,
    queue_dirty: bool,
    /// The active source's stored queue could not be read, so nothing is saved over it until a new queue is built.
    queue_unread: bool,
    /// The last queue save in flight; each waits for the one before, so awaiting this awaits them all.
    persisting: Option<tokio::task::JoinHandle<()>>,
    remote_synced: SyncedMark,
    /// The queue and place a restore left, owed to no server until they move; cleared by the first push after.
    remote_loaded: Option<(RemoteQueueMark, u64)>,
    remote_writer: mpsc::UnboundedSender<RemoteQueueJob>,
    recorder: Option<Arc<dyn PlaybackRecorder>>,
    /// What the session last published, for listen watchers that outlive a command.
    published: Option<watch::Receiver<PlayerState>>,
    scrobbler: Option<Arc<crate::scrobbler::Scrobbler>>,
    last_recent_key: Option<String>,
    /// Set while an integration the engine cannot drive owns playback.
    external: Option<ExternalState>,
    /// The integration that can play tracks the engine cannot, whether or
    /// not it currently owns playback.
    sink: Option<crate::external::SharedExternalPlayer>,
    settings_tx: watch::Sender<config::Settings>,
    settings: config::Settings,
    sources_tx: watch::Sender<Arc<server::sources::Sources>>,
    sources: Arc<server::sources::Sources>,
    active_source: Option<server::source::ActiveSource>,
    station_registry: Arc<radio::registry::StationRegistry>,
    /// The records of queued stations the registry may no longer list.
    queue_stations: std::collections::HashMap<String, radio::manifest::StationManifest>,
    offline: crate::offline::OfflineIndex,
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
    factory_override: Option<FactoryOverride>,
    /// The source's answer per track, `None` included, so a requeued track is not asked twice.
    counterparts: std::collections::HashMap<String, Option<reader::Counterpart>>,
}

impl Session {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<SessionCmd>,
        mut engine_events: mpsc::UnboundedReceiver<EngineEvent>,
        state_tx: watch::Sender<PlayerState>,
    ) {
        self.published = Some(state_tx.subscribe());
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
                _ = persist.tick(), if self.queue_dirty => {
                    self.persist_async();
                    self.push_remote_queue(false);
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
                let result = self.handle_queue_edit(edit, state_tx).await;
                let _ = reply.send(result);
            }
            SessionCmd::RadioMetadata {
                token,
                title,
                artist,
            } => self.apply_radio_metadata(token, title, artist, state_tx),
            SessionCmd::RadioTopUp {
                feed,
                cursor,
                result,
            } => self.apply_radio_page(feed, cursor, *result, state_tx),
            SessionCmd::RestoreQueue(snapshot, reply) => {
                let result = self.handle_restore(*snapshot, state_tx);
                let _ = reply.send(result);
            }
            SessionCmd::AdoptQueue {
                snapshot,
                source,
                queue_rev,
                reply,
            } => {
                let current = self.queue_rev == queue_rev && self.sources.active == source;
                let result = match current {
                    true => self.handle_restore(*snapshot, state_tx).map(|ack| {
                        // Unlike a restore from disk, nothing has stored this queue here yet.
                        self.queue_dirty = true;
                        Some(ack)
                    }),
                    false => Ok(None),
                };
                let _ = reply.send(result);
            }
            SessionCmd::SetSettings {
                settings,
                changed,
                revision,
            } => self.apply_settings(*settings, changed, revision, state_tx),
            SessionCmd::SetSources { sources, adopted } => {
                if sources.active != self.sources.active {
                    self.swap_queue(&sources, state_tx).await;
                }
                self.sources = sources;
                let _ = self.sources_tx.send(self.sources.clone());
                if let Some(adopted) = adopted {
                    let _ = adopted.send(());
                }
            }
            SessionCmd::QueueUnread => self.queue_unread = true,
            SessionCmd::AutoplayRadio {
                queue_rev,
                seed,
                result,
            } => self.handle_autoplay_radio(queue_rev, &seed, result, state_tx),
            SessionCmd::PreviewEqualizer(equalizer) => self.player.set_equalizer(equalizer),
            SessionCmd::PreviewReplayGain(replay_gain) => self.player.set_replay_gain(replay_gain),
            SessionCmd::Emit(event) => self.emit(*event),
            SessionCmd::AttachExternal(player) => {
                self.attach_external_now(player);
                self.publish(state_tx, false);
            }
            SessionCmd::SetExternalSink(player) => self.sink = Some(player),
            SessionCmd::DetachExternal => {
                if self.external.take().is_some() {
                    self.stop_playback();
                    self.last_recent_key = None;
                    self.push_external_now_playing();
                    self.publish(state_tx, false);
                }
            }
            SessionCmd::ExternalReport(report) => self.apply_external_report(*report, state_tx),
            SessionCmd::ExternalArtworkFetched(path) => {
                if let Some(external) = self.external.as_mut()
                    && external.track.is_some()
                {
                    external.artwork = Some(path);
                    self.push_external_now_playing();
                }
            }
            SessionCmd::SetStationRegistry(registry) => {
                self.keep_queued_stations();
                self.station_registry = registry;
            }
            SessionCmd::SetActiveSource(source) => self.active_source = source,
            SessionCmd::ClearError => {
                self.error = None;
                self.publish(state_tx, false);
            }
            SessionCmd::QueueMirror(reply) => {
                let (tracks, shuffle_order) = self.model.play_order();
                let _ = reply.send(QueueMirrorSnapshot {
                    tracks,
                    shuffle_order,
                    position: self.model.current_position(),
                    shuffle: self.model.shuffle(),
                    context: self.origin.clone(),
                });
            }
            SessionCmd::Persist(reply) => {
                self.settle_persists().await;
                if let Some(store) = self.queue_store.clone()
                    && !self.queue_unread
                {
                    let snapshot = self.snapshot();
                    self.queue_dirty = false;
                    store.save(&self.sources.active, snapshot).await;
                }
                if self.push_remote_queue(true) {
                    let (done, flushed) = oneshot::channel();
                    let _ = self.remote_writer.send(RemoteQueueJob::Flush(done));
                    // A quit waits on this, so a dead server costs only the timeout.
                    let _ = tokio::time::timeout(REMOTE_PUSH_TIMEOUT, flushed).await;
                }
                let _ = reply.send(());
            }
            SessionCmd::LoadPrepared(result) => self.handle_prepared_load(*result, state_tx),
            SessionCmd::LoadFinished(result) => self.handle_load_finished(result, state_tx),
            SessionCmd::BufferProgress(event) => self.handle_buffer_progress(event, state_tx),
            SessionCmd::TrackMetadata { source, tracks } => {
                if source == self.sources.active {
                    for index in 0..self.model.len() {
                        if let Some(queued) = self.model.track_at_mut(index)
                            && let Some(updated) = tracks.iter().find(|track| track.id == queued.id)
                        {
                            *queued = updated.clone();
                        }
                    }
                    let token = self.intent.token();
                    if let Some(track) = self.model.current_track() {
                        self.player.update_metadata(
                            token,
                            NowPlayingMeta {
                                title: track.title.clone(),
                                artist: track.artist.clone(),
                                album: track.album.clone(),
                                duration: Duration::from_secs(track.duration),
                                artwork: load::now_playing_artwork(
                                    self.sources.server.as_ref(),
                                    track,
                                ),
                            },
                        );
                    }
                    self.publish(state_tx, true);
                }
            }
            SessionCmd::AlbumCovers { source, covers } => {
                self.apply_album_covers(source, covers, state_tx)
            }
            SessionCmd::ArtworkFetched { token, meta } => {
                if self.intent.token() == token || self.is_outgoing(token) {
                    self.player.update_metadata(token, *meta);
                }
            }
            SessionCmd::CounterpartFound { uid, counterpart } => {
                self.counterparts
                    .insert(uid.clone(), Some(counterpart.as_ref().clone()));
                if self.learn_counterpart(&uid, *counterpart) {
                    self.publish(state_tx, true);
                }
            }
        }
    }

    fn handle_player_command(
        &mut self,
        command: PlayerCommand,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        // Media keys, MPRIS and every frontend send the same commands whoever
        // is playing; while an integration owns playback they go to it, since
        // the engine has nothing to act on. Toggle resolves here because only
        // the daemon knows the current phase.
        // Next and previous are the queue's, whoever is playing: an
        // integration holds one track and knows nothing about what follows it.
        if self.external.is_some()
            && !matches!(command, PlayerCommand::Next | PlayerCommand::Previous)
        {
            let command = match command {
                PlayerCommand::Toggle if self.phase == ApiPhase::Playing => PlayerCommand::Pause,
                PlayerCommand::Toggle => PlayerCommand::Play,
                other => other,
            };
            if let PlayerCommand::SetMode { shuffle, loop_mode } = command {
                if let Some(on) = shuffle {
                    self.model.set_shuffle(on);
                }
                if let Some(mode) = loop_mode {
                    self.model.set_loop_mode(mode);
                }
                return Ok(self.publish(state_tx, shuffle.is_some()));
            }
            let command = match command {
                PlayerCommand::SetVolume { volume } => {
                    self.set_volume(volume);
                    PlayerCommand::SetVolume {
                        volume: self.output_volume(),
                    }
                }
                PlayerCommand::SetMuted { muted } => {
                    self.muted = muted;
                    PlayerCommand::SetVolume {
                        volume: self.output_volume(),
                    }
                }
                other => other,
            };
            self.dispatch_external(command);
            return Ok(self.publish(state_tx, false));
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
                self.set_volume(volume);
                self.player.set_volume(self.output_volume());
            }
            PlayerCommand::SetMuted { muted } => {
                self.muted = muted;
                self.player.set_volume(self.output_volume());
            }
            PlayerCommand::SetVersion { version } => {
                queue_changed = self.set_version(version, state_tx)?;
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
                self.rearm_gapless();
            }
        }
        Ok(self.publish(state_tx, queue_changed))
    }

    /// First play-order position the upcoming edits own: past the playing track, and past the one a crossfade is already fading into.
    fn upcoming_from(&mut self) -> usize {
        if self.model.is_empty() {
            return 0;
        }
        let current = self.model.current_position();
        match self
            .pending_transition
            .as_ref()
            .filter(|pending| !pending.gapless)
            .map(|pending| (pending.to_position, pending.stage))
        {
            Some((to, _)) if to > current => to + 1,
            // A loop back to the top is not after the playing track, so undo it while it is still inaudible.
            Some((_, TransitionStage::Loading)) => {
                let _ = self.revert_transition();
                current + 1
            }
            _ => current + 1,
        }
    }

    async fn materialize_keys(&self, keys: Vec<String>) -> Result<Vec<reader::Track>, ApiError> {
        let tracks = tokio::time::timeout(
            MATERIALIZE_TIMEOUT,
            self.materializer
                .materialize(&QueueContext::Tracks { keys }),
        )
        .await
        .map_err(|_| {
            ApiError::new(
                api::ErrorCode::SourceUnreachable,
                "timed out resolving the tracks to queue",
            )
        })??;
        if tracks.is_empty() {
            return Err(ApiError::not_found("none of those keys are in the library"));
        }
        let (tracks, _) = self.allowed(tracks, None)?;
        Ok(tracks)
    }

    async fn handle_queue_edit(
        &mut self,
        edit: QueueEdit,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        let len = self.model.len();
        match edit {
            QueueEdit::JumpPhysical { index } => {
                let index = index as usize;
                if self.model.items().get(index).is_none() {
                    return Err(ApiError::invalid_input("no track at that queue position"));
                }
                self.admit_offline(self.model.items().get(index))?;
                let position = self.model.jump_to(index);
                self.start_load(position, false);
                Ok(self.publish(state_tx, true))
            }
            QueueEdit::Insert { index, keys } => {
                let index = index as usize;
                if index > len {
                    return Err(ApiError::invalid_input("queue position out of range"));
                }
                let tracks = self.materialize_keys(keys).await?;
                self.model.insert_at(index, tracks);
                self.rearm_gapless();
                Ok(self.publish(state_tx, true))
            }
            QueueEdit::ClearUpcoming => {
                let from = self.upcoming_from();
                if !self.model.clear_from(from) {
                    return Ok(CommandAck { rev: self.rev });
                }
                self.rearm_gapless();
                Ok(self.publish(state_tx, true))
            }
            QueueEdit::ReplaceUpcoming { keys } => {
                let from = self.upcoming_from();
                if keys.is_empty() {
                    if !self.model.clear_from(from) {
                        return Ok(CommandAck { rev: self.rev });
                    }
                    self.rearm_gapless();
                    return Ok(self.publish(state_tx, true));
                }
                let tracks = self.materialize_keys(keys).await?;
                self.model.replace_from(from, tracks);
                self.rearm_gapless();
                Ok(self.publish(state_tx, true))
            }
            QueueEdit::ShuffleUpcoming => {
                let from = self.upcoming_from();
                if len <= from + 1 {
                    return Ok(CommandAck { rev: self.rev });
                }
                self.model.shuffle_from(from);
                self.rearm_gapless();
                Ok(self.publish(state_tx, true))
            }
            QueueEdit::Jump { index } => {
                let mut candidate = self.model.clone();
                let position = candidate
                    .jump_to_position(index as usize)
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
                self.rearm_gapless();
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
                    .is_some_and(|pending| !pending.gapless && pending.to_position == index)
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
                self.rearm_gapless();
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
        let RadioPage { tracks, more } = tokio::time::timeout(
            MATERIALIZE_TIMEOUT,
            self.materializer.materialize_queue(&request.context),
        )
        .await
        .map_err(|_| {
            ApiError::new(
                api::ErrorCode::SourceUnreachable,
                "queue materialization timed out",
            )
        })??;
        let (tracks, start_index) =
            self.allowed(tracks, request.start_index.map(|index| index as usize))?;
        let origin = request
            .context
            .names_container()
            .then(|| request.context.clone());
        let was_empty = self.model.is_empty();
        match request.mode {
            QueueMode::Replace => {
                let mut candidate = self.model.clone();
                candidate.replace(tracks);
                if let Some(on) = request.shuffle {
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
                    let idx = candidate.jump_to(start.min(len - 1));
                    if self.radio_keeps_playing(&request.context, &candidate, idx) {
                        self.model = candidate;
                        self.prefetch_next();
                    } else {
                        self.start_immediate_load(candidate, idx)?;
                    }
                    self.set_radio_feed(more);
                    self.origin = origin.clone();
                } else {
                    self.model = candidate;
                    self.stop_playback();
                    self.set_radio_feed(None);
                    self.origin = None;
                }
            }
            // Appending lands past every existing position, so a pending
            // crossfade target needs no remap.
            QueueMode::Append => self.model.add(tracks),
            QueueMode::PlayNext => match self.pending_transition.as_ref() {
                // Mid-crossfade the queue still reads as the outgoing track,
                // but "next" means after the one already fading in, or the
                // insertion would be skipped the moment the fade commits.
                Some(pending) if !pending.gapless => {
                    self.model.insert_at(pending.to_position + 1, tracks)
                }
                _ => self.model.insert_next(tracks),
            },
        }
        if request.mode != QueueMode::Replace {
            self.rearm_gapless();
        }
        // Adding to a queue is an edit that keeps its origin; only an empty one becomes what was added.
        if request.mode != QueueMode::Replace && was_empty && !self.model.is_empty() {
            self.origin = origin;
        }
        Ok(self.publish(state_tx, true))
    }

    /// A radio seeded from the song already playing grows around it instead of starting it over.
    fn radio_keeps_playing(
        &self,
        context: &QueueContext,
        candidate: &QueueModel,
        idx: usize,
    ) -> bool {
        matches!(
            context,
            QueueContext::TrackRadio { .. } | QueueContext::PlaylistRadio { .. }
        ) && self.pending_transition.is_none()
            && matches!(self.intent, PlaybackIntent::Committed { .. })
            && self
                .model
                .current_track()
                .zip(candidate.track_at(idx))
                .is_some_and(|(playing, first)| playing.id.uid() == first.id.uid())
    }

    fn play_next(
        &mut self,
        allow_crossfade: bool,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<(), ApiError> {
        let gate = self.offline_gate();
        let playable = playable(self.settings.playback.skip_explicit, gate.as_ref());
        if allow_crossfade {
            if let NextOutcome::Play(idx) = self.model.peek_next_where(playable)
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
        match candidate.advance_next_where(playable) {
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
                // A radio tops itself up; autoplay starts one only for a
                // queue that is not one already.
                if self.radio_feed.is_some() {
                    self.resume_radio_when_extended();
                } else {
                    self.start_autoplay();
                }
            }
            NextOutcome::Empty => {}
        }
        Ok(())
    }

    /// Refused when nothing is left, since an empty queue would read as a stop
    /// the caller never asked for.
    fn allowed(
        &self,
        tracks: Vec<Track>,
        start: Option<usize>,
    ) -> Result<(Vec<Track>, Option<usize>), ApiError> {
        let skip = self.settings.playback.skip_explicit;
        let gate = self.offline_gate();
        if (!skip && gate.is_none()) || tracks.is_empty() {
            return Ok((tracks, start));
        }
        let nothing_offline = gate
            .as_ref()
            .is_some_and(|gate| !tracks.iter().any(|track| gate.admits(track)));
        // Dropping the chosen song would start whichever one follows it in its place.
        let chosen_offline = start
            .and_then(|index| tracks.get(index))
            .is_none_or(|track| gate.as_ref().is_none_or(|gate| gate.admits(track)));
        let (kept, start) =
            crate::queue_model::keep_where(tracks, start, playable(skip, gate.as_ref()));
        if nothing_offline || !chosen_offline {
            return Err(offline_refusal());
        }
        if kept.is_empty() {
            return Err(ApiError::invalid_input(
                "every track here is explicit, and explicit tracks are skipped",
            ));
        }
        Ok((kept, start))
    }

    /// Refuse a jump to a song offline mode cannot play, before anything playing is touched.
    fn admit_offline(&self, track: Option<&Track>) -> Result<(), ApiError> {
        match (self.offline_gate(), track) {
            (Some(gate), Some(track)) if !gate.admits(track) => Err(offline_refusal()),
            _ => Ok(()),
        }
    }

    /// What offline mode keeps the queue to, or `None` while it is off.
    pub(super) fn offline_gate(&self) -> Option<OfflineGate> {
        self.offline.offline_mode().then(|| OfflineGate {
            index: self.offline.clone(),
            needs_internet: crate::network::needs_internet(&self.sources),
        })
    }

    /// Keep the song after this one in the offline cache ahead of time, in play order, so shuffle's.
    pub(super) fn prefetch_next(&mut self) {
        let cache = self.offline.cache().clone();
        if !cache.enabled()
            || self.offline.offline_mode()
            || self.external.is_some()
            || !crate::network::cacheable(&self.sources)
        {
            return;
        }
        let skip = self.settings.playback.skip_explicit;
        let NextOutcome::Play(idx) = self.model.peek_next_where(playable(skip, None)) else {
            return;
        };
        if idx == self.model.current_position() {
            return;
        }
        let Some(track) = self.model.track_at(idx) else {
            return;
        };
        if self.sink_for(track).is_some() {
            return;
        }
        let uid = track.id.uid();
        let item_ref = PlaybackItemRef::parse(&uid);
        let Some(item_id) = item_ref.primary_id().filter(|_| item_ref.is_server()) else {
            return;
        };
        if self.offline.has_copy(item_id) {
            return;
        }
        cache.prefetch(crate::offline_cache::PrefetchRequest {
            item_id: item_id.to_string(),
            source_id: self.sources.active.as_str().to_string(),
            source: self.active_source.clone(),
            quality: self.settings.playback.stream_quality,
        });
    }

    /// The mix is a network round trip, so it is fetched off the actor and
    /// lands as a command.
    fn start_autoplay(&mut self) {
        let radio = self
            .active_source
            .as_ref()
            .is_some_and(|source| source.capabilities().radio.track);
        if !self.settings.playback.autoplay_radio
            || !radio
            || self.offline.offline_mode()
            || self.current_track_is_radio()
            || self
                .autoplay_task
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let Some(seed) = self
            .model
            .current_track()
            .map(|track| track.id.key().into_owned())
        else {
            return;
        };
        let materializer = self.materializer.clone();
        let tx = self.cmd_tx.clone();
        let queue_rev = self.queue_rev;
        self.autoplay_task = Some(tokio::spawn(async move {
            let context = QueueContext::TrackRadio { key: seed.clone() };
            let result = match tokio::time::timeout(
                MATERIALIZE_TIMEOUT,
                materializer.materialize_queue(&context),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(ApiError::new(
                    api::ErrorCode::SourceUnreachable,
                    "timed out building the autoplay radio",
                )),
            };
            let _ = tx.send(SessionCmd::AutoplayRadio {
                queue_rev,
                seed,
                result,
            });
        }));
    }

    /// Dropped when the queue changed or something started playing meanwhile.
    fn handle_autoplay_radio(
        &mut self,
        queue_rev: u64,
        seed: &str,
        result: Result<RadioPage, ApiError>,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        self.autoplay_task = None;
        if queue_rev != self.queue_rev
            || self.intent != PlaybackIntent::Stopped
            || self
                .model
                .current_track()
                .map(|track| track.id.key())
                .as_deref()
                != Some(seed)
        {
            return;
        }
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, "autoplay radio failed; the queue stays ended");
                return;
            }
        };
        let skip = self.settings.playback.skip_explicit;
        let tracks: Vec<Track> = page
            .tracks
            .into_iter()
            .filter(|track| track.id.key() != seed && !(skip && track.explicit))
            .collect();
        if tracks.is_empty() {
            return;
        }
        self.model.add(tracks);
        // From here it tops itself up like any radio queue.
        self.set_radio_feed(page.more);
        let mut candidate = self.model.clone();
        if let NextOutcome::Play(idx) = candidate.advance_next_where(playable(skip, None))
            && let Err(error) = self.start_immediate_load(candidate, idx)
        {
            tracing::warn!(%error, "autoplay radio would not start");
        }
        self.publish(state_tx, true);
    }

    fn play_previous(&mut self, state_tx: &watch::Sender<PlayerState>) -> Result<(), ApiError> {
        let idx = self.model.current_position();
        let gapless = self
            .pending_transition
            .as_ref()
            .is_some_and(|pending| pending.gapless);
        if self.revert_transition().is_some() && !gapless {
            let candidate = self.model.clone();
            self.start_immediate_load(candidate, idx)?;
            return Ok(());
        }

        if self.settings.playback.back_behavior == config::BackBehavior::RewindThenPrev
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
        let queued = self
            .pending_transition
            .as_ref()
            .is_some_and(|pending| pending.stage == TransitionStage::Queued);
        if (self.intent.is_loading() || queued) && self.revert_transition().is_none() {
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
        if is_radio || !self.player.can_resume() || self.engine_released {
            if self.model.track_at(idx).is_some() {
                // A released engine already left the swapped cut's place.
                if !is_radio && !self.engine_released {
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

    /// Two players cannot both be playing: ours stops before the integration
    /// starts, and what it plays arrives on its report stream.
    fn attach_external_now(&mut self, player: crate::external::SharedExternalPlayer) {
        self.stop_playback();
        self.external = Some(ExternalState {
            player,
            device: None,
            track: None,
            artwork: None,
            completed_key: None,
        });
        // An integration starts at its own level, so it learns ours (and a mute) as it takes over.
        self.dispatch_external(PlayerCommand::SetVolume {
            volume: self.output_volume(),
        });
    }

    /// Hand playback back to the engine: the integration is told to stop, and
    /// the caller is about to load into the engine instead.
    fn release_external(&mut self) {
        let Some(external) = self.external.take() else {
            return;
        };
        self.last_recent_key = None;
        let player = external.player;
        tokio::spawn(async move {
            if let Err(error) = player.stop().await {
                tracing::warn!(%error, "external playback would not stop");
            }
        });
    }

    fn stop_playback(&mut self) {
        self.cancel_load_task();
        self.cancel_radio_task();
        if let Some(task) = self.autoplay_task.take() {
            task.abort();
        }
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

    fn apply_settings(
        &mut self,
        settings: config::Settings,
        changed: Vec<config::SettingKey>,
        revision: u64,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        use config::{PlaybackKey, SettingKey};
        let mut skip_explicit_changed = false;
        for key in changed {
            match key {
                SettingKey::Playback(PlaybackKey::Equalizer) => self
                    .player
                    .set_equalizer(settings.playback.equalizer.clone()),
                SettingKey::Playback(PlaybackKey::ReplayGain) => {
                    self.player.set_replay_gain(settings.playback.replay_gain)
                }
                SettingKey::Playback(PlaybackKey::ChannelMode) => {
                    self.player.set_channel_mode(settings.playback.channel_mode)
                }
                SettingKey::Playback(PlaybackKey::SampleRateMode) => self
                    .player
                    .set_sample_rate_mode(settings.playback.sample_rate_mode),
                SettingKey::Playback(PlaybackKey::DeviceChangeBehavior) => {
                    self.player
                        .set_device_change_behavior(settings.playback.device_change_behavior);
                }
                SettingKey::Playback(PlaybackKey::SkipExplicit) => skip_explicit_changed = true,
                _ => {}
            }
        }
        self.offline.cache().set_limits(&settings);
        self.settings = settings;
        if skip_explicit_changed {
            // A gapless switch armed before the change may target a track it now skips.
            self.rearm_gapless();
        }
        let _ = self.settings_tx.send(self.settings.clone());
        self.emit(ApiEvent::ConfigChanged { revision });
        self.publish(state_tx, false);
    }

    /// Fold an integration's report into the same state the engine would have
    /// produced: player state, recents, listen counts, scrobbles, OS widget.
    fn apply_external_report(
        &mut self,
        report: crate::external::ExternalReport,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        let Some(external) = self.external.as_mut() else {
            return;
        };
        external.device = report.device;
        // The tab reports only what its SDK knows (no album id, sometimes no art); when it is the track kopuz queued, show the library's row.
        let reported = report.track.map(|track| match self.model.current_track() {
            Some(queued) if queued.id.uid() == track.id.uid() => queued.clone(),
            _ => track,
        });
        let changed = reported.as_ref().map(|track| track.id.uid())
            != external.track.as_ref().map(|track| track.id.uid());
        let mut committed = None;
        if changed {
            external.completed_key = None;
            external.artwork = None;
            external.track = reported;
            self.last_recent_key = None;
            match self.external.as_ref().and_then(|it| it.track.clone()) {
                Some(track) => {
                    let token = self.allocate_token();
                    self.set_intent(PlaybackIntent::Committed { token });
                    self.record_recent_track(track.clone());
                    committed = Some((track, token));
                }
                None => self.set_intent(PlaybackIntent::Stopped),
            }
            self.fetch_external_artwork(report.cover_url);
        }

        let playing_track = self.external.as_ref().and_then(|it| it.track.clone());
        self.phase = match (&playing_track, report.playing) {
            (None, _) => ApiPhase::Idle,
            (Some(_), true) => ApiPhase::Playing,
            (Some(_), false) => ApiPhase::Paused,
        };
        self.position = playing_track.as_ref().map(|_| PositionAnchor {
            ms: report.position_ms,
            at_ms: self.now_ms(),
            playing: report.playing,
        });
        self.position_token = playing_track.as_ref().map(|_| self.current_token);

        if report.completed
            && let Some(track) = playing_track
            && self
                .external
                .as_ref()
                .is_some_and(|it| it.completed_key.as_deref() != Some(track.id.uid().as_str()))
        {
            if let Some(external) = self.external.as_mut() {
                external.completed_key = Some(track.id.uid());
            }
            // The queue is kopuz's, so its end-of-track is kopuz's too: the
            // next item may be another Spotify track or a local file, and
            // either way the load path decides who plays it.
            if let Err(error) = self.play_next(false, state_tx) {
                tracing::warn!(%error, "advancing after an external track failed");
            }
        }
        self.push_external_now_playing();
        self.publish(state_tx, false);
        if let Some((track, token)) = committed {
            self.track_committed(track, token);
        }
    }

    /// The OS widgets want a local file, not a URL, so the cover is fetched in
    /// the background and the metadata re-pushed when it lands.
    fn fetch_external_artwork(&self, cover_url: Option<String>) {
        let Some(url) = cover_url.filter(|url| url.starts_with("http")) else {
            return;
        };
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            if let Some(path) = load::fetch_cover_to_temp(&url, &url).await {
                let _ = tx.send(SessionCmd::ExternalArtworkFetched(path));
            }
        });
    }

    /// Drive the OS media widget for external playback. Engine playback pushes
    /// from `Player::push_now_playing`, which is silent while the engine is
    /// stopped, so the daemon feeds it from the report instead -- a frontend
    /// never reaches for the system integration itself.
    fn push_external_now_playing(&self) {
        let Some(track) = self.external.as_ref().and_then(|it| it.track.as_ref()) else {
            player::systemint::update_now_playing("", "", "", 0.0, 0.0, false, None);
            return;
        };
        let artwork = self.external.as_ref().and_then(|it| it.artwork.clone());
        player::systemint::update_now_playing(
            &track.title,
            &track.artist,
            &track.album,
            track.duration as f64,
            self.position
                .map(|anchor| anchor.ms as f64 / 1000.0)
                .unwrap_or_default(),
            self.phase == ApiPhase::Playing,
            artwork.as_deref(),
        );
    }

    /// A new level also unmutes.
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = false;
    }

    fn output_volume(&self) -> f32 {
        match self.muted {
            true => 0.0,
            false => self.volume,
        }
    }

    /// Send a transport command to the integration that owns playback. The
    /// call is a network round-trip, so it runs off the actor loop; the answer
    /// comes back as the integration's next report.
    fn dispatch_external(&self, command: PlayerCommand) {
        let Some(player) = self.external.as_ref().map(|it| it.player.clone()) else {
            return;
        };
        tokio::spawn(async move {
            let result = match command {
                PlayerCommand::Play => player.resume().await,
                PlayerCommand::Pause => player.pause().await,
                PlayerCommand::Stop => player.stop().await,
                PlayerCommand::Seek { position_ms } => player.seek(position_ms).await,
                PlayerCommand::SetVolume { volume } => player.set_volume(volume).await,
                PlayerCommand::Next
                | PlayerCommand::Previous
                | PlayerCommand::Toggle
                | PlayerCommand::SetMuted { .. }
                | PlayerCommand::SetMode { .. }
                | PlayerCommand::SetVersion { .. } => Ok(()),
            };
            if let Err(error) = result {
                tracing::warn!(%error, "external playback command failed");
            }
        });
    }

    /// Record an externally played track as recently played.
    fn record_recent_track(&mut self, track: Track) {
        let uid = track.id.uid();
        if self.last_recent_key.as_deref() == Some(uid.as_str()) {
            return;
        }
        self.last_recent_key = Some(uid);
        if let Some(recorder) = self.recorder.clone() {
            let events = self.events.clone();
            let source = self.active_source.clone();
            tokio::spawn(async move {
                recorder.record_recent(source.as_ref(), &track).await;
                announce_history(&events);
            });
        }
    }

    /// Record the committed track as recently played, once per session track.
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
        let events = self.events.clone();
        let source = self.active_source.clone();
        tokio::spawn(async move {
            recorder.record_recent(source.as_ref(), &track).await;
            announce_history(&events);
        });
    }

    /// Count a listen once the committed track has played long enough, by the
    /// scrobbler's rule, so a skip after most of a song still counts and a
    /// skip after a few seconds does not. Never for radio.
    fn count_listen_after_playtime(&self, track: Track, token: u64) {
        let (Some(recorder), Some(published)) = (self.recorder.clone(), self.published.clone())
        else {
            return;
        };
        if track.duration == u64::MAX {
            return;
        }
        let threshold = crate::scrobbler::listen_threshold(track.duration);
        let events = self.events.clone();
        let source = self.active_source.clone();
        tokio::spawn(async move {
            let state = || published.borrow().clone();
            let uid = track.id.uid();
            if !crate::scrobbler::accumulate_playtime(state, threshold, token, &uid).await {
                return;
            }
            recorder.bump_listen_count(source.as_ref(), &track).await;
            announce_history(&events);
        });
    }

    /// Port of the app's `restore_queue_state`: stop, restore the model,
    /// seed a paused resume point at the saved progress.
    fn handle_restore(
        &mut self,
        mut snapshot: db::QueueSnapshot,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        // A restore replaces whatever is playing, an integration's device included.
        self.release_external();
        self.stop_playback();
        self.position = None;
        self.position_token = None;
        self.last_recent_key = None;

        self.adopt_station_records(&mut snapshot);
        self.set_radio_feed(snapshot.radio_cursor);
        let restored = self.model.restore(
            snapshot.queue,
            snapshot.current_queue_index,
            snapshot.shuffle_order,
            snapshot.shuffle_enabled,
        );
        self.origin = snapshot
            .origin
            .as_deref()
            .filter(|_| !self.model.is_empty())
            .and_then(origin::decode);
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
        self.remote_loaded = Some(self.remote_mark());
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
            radio_cursor: self.radio_cursor(),
            origin: self.origin.as_ref().and_then(origin::encode),
            stations: self.station_records(),
        }
    }

    /// Fire-and-forget save off the actor thread; overlapping writes are
    /// last-write-wins on one SQLite row.
    fn persist_async(&mut self) {
        self.queue_dirty = false;
        let Some(store) = self.queue_store.clone() else {
            return;
        };
        if self.queue_unread {
            return;
        }
        let snapshot = self.snapshot();
        let source = self.sources.active.clone();
        let before = self.persisting.take();
        self.persisting = Some(tokio::spawn(async move {
            if let Some(before) = before {
                let _ = before.await;
            }
            store.save(&source, snapshot).await;
        }));
    }

    /// Wait out every queue save already started.
    async fn settle_persists(&mut self) {
        if let Some(pending) = self.persisting.take() {
            let _ = pending.await;
        }
    }

    /// Park the queue under the source being left, or drop it when that source is gone, and resume the one being entered.
    async fn swap_queue(
        &mut self,
        next: &server::sources::Sources,
        state_tx: &watch::Sender<PlayerState>,
    ) {
        let (resumed, unread) = match self.queue_store.clone() {
            Some(store) => {
                self.settle_persists().await;
                let leaving = self.sources.active.clone();
                if !next.has_source(&self.settings.library.local_sources, &leaving) {
                    store.forget(&leaving).await;
                } else if !self.queue_unread {
                    store.save(&leaving, self.snapshot()).await;
                }
                match store.load(&next.active).await {
                    Ok(resumed) => (resumed, false),
                    Err(error) => {
                        tracing::warn!(%error, "the queue of the source switched to could not be read");
                        (db::QueueSnapshot::default(), true)
                    }
                }
            }
            None => (db::QueueSnapshot::default(), false),
        };
        self.materializer.register_restored(&resumed.queue);
        let _ = self.handle_restore(resumed, state_tx);
        self.queue_unread = unread;
    }

    /// The queue as the server would store it, and the place in it.
    fn remote_mark(&self) -> (RemoteQueueMark, u64) {
        use std::hash::{Hash, Hasher};
        let mut tracks = std::hash::DefaultHasher::new();
        for position in 0..self.model.len() {
            if let Some(track) = self.model.track_at(position) {
                track.id.key().hash(&mut tracks);
            }
        }
        let mark = RemoteQueueMark {
            tracks: tracks.finish(),
            current_key: self.model.current_track().map(|track| track.id.uid()),
            playing: self.phase == ApiPhase::Playing,
        };
        let position_ms =
            crate::persistence::whole_seconds(self.displayed_position().as_millis() as u64) * 1000;
        (mark, position_ms)
    }

    /// Hand the queue to the remote writer, unless the server already took this
    /// mark or nothing moved since a restore; `force` skips the first check, for
    /// the flush on quit. Whether it was sent.
    fn push_remote_queue(&mut self, force: bool) -> bool {
        let Some(source) = self.active_source.clone() else {
            return false;
        };
        if !source.capabilities().play_queue {
            return false;
        }
        let (mark, position_ms) = self.remote_mark();
        // Between ticks only the mark counts, as for a synced push; the flush on quit also owes a moved place.
        let untouched = self
            .remote_loaded
            .as_ref()
            .is_some_and(|(loaded, at)| *loaded == mark && (!force || *at == position_ms));
        let synced = self
            .remote_synced
            .lock()
            .is_ok_and(|synced| synced.as_ref() == Some(&mark));
        if untouched || (!force && synced) {
            return false;
        }
        self.remote_loaded = None;
        // Play order, not storage order: another client reading this queue
        // plays it top to bottom and has no shuffle permutation to apply.
        let queue: Vec<Track> = (0..self.model.len())
            .filter_map(|position| self.model.track_at(position).cloned())
            .collect();
        let push = RemoteQueuePush {
            source,
            store_key: self.sources.active.clone(),
            queue,
            current_index: self.model.current_position(),
            position_ms,
            mark,
        };
        self.remote_writer.send(RemoteQueueJob::Push(push)).is_ok()
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
        self.armed_transition = Some(outgoing_token);
        if !self.commit_transition_model(token) {
            return false;
        }
        self.player.commit_now_playing(token);
        true
    }

    /// A track shorter than the preload never reaches the next position event.
    pub(super) fn arm_gapless_now(&mut self) -> bool {
        let arm =
            self.should_play_gapless() && self.should_arm_transition(self.player.get_position());
        if arm {
            self.arm_transition();
        }
        arm
    }

    fn rearm_gapless(&mut self) {
        let Some(pending) = self
            .pending_transition
            .as_ref()
            .filter(|pending| pending.gapless)
        else {
            return;
        };
        let (position, key) = (pending.to_position, pending.to_key.clone());
        let gate = self.offline_gate();
        let next = self.model.peek_next_where(playable(
            self.settings.playback.skip_explicit,
            gate.as_ref(),
        ));
        let still_next = matches!(next, NextOutcome::Play(next) if next == position)
            && self
                .model
                .track_at(position)
                .is_some_and(|track| track.id.uid() == key);
        if still_next {
            return;
        }
        let _ = self.revert_transition();
        if self.should_arm_transition(self.player.get_position()) {
            self.arm_transition();
        }
    }

    /// A committed track is scrobbled and, once it has played long enough by
    /// the same rule, counted as a listen for the source active now.
    pub(super) fn track_committed(&self, track: Track, token: u64) {
        self.count_listen_after_playtime(track.clone(), token);
        if let Some(scrobbler) = self.scrobbler.clone() {
            scrobbler.track_committed(track, token);
        }
    }

    fn start_immediate_load(&mut self, model: QueueModel, idx: usize) -> Result<(), ApiError> {
        self.admit_offline(model.track_at(idx))?;
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

    /// Undo either a resolving crossfade or a running fade. The queue pointer
    /// never left the outgoing track, so dropping the pending target is the
    /// whole undo.
    fn revert_transition(&mut self) -> Option<u64> {
        let pending = self.pending_transition.take()?;
        if pending.stage != TransitionStage::Fading {
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

    fn fail_load(
        &mut self,
        token: u64,
        code: api::ErrorCode,
        error: impl std::fmt::Display,
    ) -> bool {
        let intent = self.intent;
        if intent.token() != token {
            return false;
        }
        self.error = Some(api::ErrorBody {
            code,
            message: format!("couldn't load this track: {error}"),
            reason: None,
        });
        self.buffered.clear();
        match intent {
            PlaybackIntent::Loading {
                deferred: true,
                from_token,
                ..
            } => {
                self.armed_transition = self
                    .pending_transition
                    .take()
                    .filter(|pending| pending.gapless)
                    .map(|_| from_token);
                self.set_intent(PlaybackIntent::Committed { token: from_token });
            }
            _ => {
                self.set_intent(PlaybackIntent::Stopped);
                if self.phase == ApiPhase::Ended {
                    self.player.pause();
                }
            }
        }
        true
    }

    fn pending_resume_seek(&self, track: &Track) -> (Option<Duration>, bool) {
        let pending = self.pending_resume.as_ref();
        let position = pending.and_then(|pending| {
            (pending.track_key == track.id.uid()).then(|| {
                // Workaround: When the last track gets replayed/resumed after it ended,
                // it should show progress starting from the start instead of leaving it
                // at the end of the song, while audio is playing in the background.
                if self.phase == ApiPhase::Ended {
                    Duration::ZERO
                } else {
                    let duration = track.duration.saturating_mul(1_000);
                    Duration::from_millis(pending.position_ms.min(duration))
                }
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
                track: crate::wire::track_info(&track, &self.offline),
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

/// The attached integration plus what it last reported.
struct ExternalState {
    player: crate::external::SharedExternalPlayer,
    device: Option<String>,
    track: Option<Track>,
    artwork: Option<String>,
    completed_key: Option<String>,
}

fn offline_refusal() -> ApiError {
    ApiError::new(
        api::ErrorCode::SourceUnreachable,
        "offline mode is on, and what was asked for has no copy that plays offline",
    )
    .because(api::ErrorReason::OfflineMode)
}

/// What offline mode lets the queue play: files, songs with a copy here, and every song of a source that needs no internet.
#[derive(Clone)]
pub(super) struct OfflineGate {
    index: crate::offline::OfflineIndex,
    needs_internet: bool,
}

impl OfflineGate {
    fn admits(&self, track: &Track) -> bool {
        self.index.plays_offline(track, self.needs_internet)
    }
}

/// What the queue may move on to: no explicit track the settings skip, and in offline mode none without a copy that plays offline.
fn playable(
    skip_explicit: bool,
    gate: Option<&OfflineGate>,
) -> impl Fn(&Track) -> bool + Copy + '_ {
    move |track| !(skip_explicit && track.explicit) && gate.is_none_or(|gate| gate.admits(track))
}

/// Tell clients play history moved, sent after the write so a re-read sees it.
fn announce_history(events: &broadcast::Sender<ApiEvent>) {
    let _ = events.send(ApiEvent::LibraryInvalidated {
        table: api::Table::Recents,
    });
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
