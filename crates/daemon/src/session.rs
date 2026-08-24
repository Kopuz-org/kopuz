//! The PlayerSession actor: single owner of queue/transport state, driven by
//! commands and publishing [`PlayerState`] snapshots plus [`ApiEvent`]s.
//!
//! This is the daemon-side replacement for the Signal-shaped
//! `PlayerController`. The queue semantics are live; transport commands that
//! need the audio engine return `Unsupported` until the engine port lands, so
//! nothing pretends to play audio it cannot.

use std::sync::Arc;
use std::time::Instant;

use api::{
    ApiError, ApiEvent, CommandAck, NowPlaying, Page, PlayerCommand, PlayerState, QueueContext,
    QueueItem, QueueMode, QueueSummary, QueueWindow, SetQueueRequest, TrackKind,
};
use reader::Track;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::queue_model::QueueModel;

pub const EVENT_BUFFER: usize = 512;

/// Resolves a [`QueueContext`] into concrete tracks, daemon-side. The library
/// service implements this over the database; tests inject a stub.
#[async_trait::async_trait]
pub trait QueueMaterializer: Send + Sync {
    async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError>;
}

enum SessionCmd {
    Player(PlayerCommand, oneshot::Sender<Result<CommandAck, ApiError>>),
    SetQueue(
        SetQueueRequest,
        oneshot::Sender<Result<CommandAck, ApiError>>,
    ),
    Window(Page, oneshot::Sender<Result<QueueWindow, ApiError>>),
}

#[derive(Clone)]
pub struct SessionHandle {
    cmd_tx: mpsc::UnboundedSender<SessionCmd>,
    state_rx: watch::Receiver<PlayerState>,
    events: broadcast::Sender<ApiEvent>,
}

impl SessionHandle {
    pub fn spawn(materializer: Arc<dyn QueueMaterializer>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let session = Session {
            model: QueueModel::default(),
            rev: 0,
            queue_rev: 0,
            volume: 1.0,
            epoch: Instant::now(),
            events: events.clone(),
            materializer,
        };
        let (state_tx, state_rx) = watch::channel(session.build_state());
        tokio::spawn(session.run(cmd_rx, state_tx));
        Self {
            cmd_tx,
            state_rx,
            events,
        }
    }

    pub fn state(&self) -> PlayerState {
        self.state_rx.borrow().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ApiEvent> {
        self.events.subscribe()
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

    pub async fn player_command(&self, cmd: PlayerCommand) -> Result<CommandAck, ApiError> {
        self.request(|tx| SessionCmd::Player(cmd, tx)).await
    }

    pub async fn set_queue(&self, req: SetQueueRequest) -> Result<CommandAck, ApiError> {
        self.request(|tx| SessionCmd::SetQueue(req, tx)).await
    }

    pub async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        self.request(|tx| SessionCmd::Window(page, tx)).await
    }
}

struct Session {
    model: QueueModel,
    rev: u64,
    queue_rev: u64,
    volume: f32,
    epoch: Instant,
    events: broadcast::Sender<ApiEvent>,
    materializer: Arc<dyn QueueMaterializer>,
}

impl Session {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<SessionCmd>,
        state_tx: watch::Sender<PlayerState>,
    ) {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCmd::Player(cmd, reply) => {
                    let result = self.handle_player_command(cmd, &state_tx);
                    let _ = reply.send(result);
                }
                SessionCmd::SetQueue(req, reply) => {
                    let result = self.handle_set_queue(req, &state_tx).await;
                    let _ = reply.send(result);
                }
                SessionCmd::Window(page, reply) => {
                    let _ = reply.send(Ok(self.window(page)));
                }
            }
        }
    }

    fn handle_player_command(
        &mut self,
        cmd: PlayerCommand,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        match cmd {
            PlayerCommand::Next => {
                self.model.advance_next();
                Ok(self.publish(state_tx, false))
            }
            PlayerCommand::Previous => {
                self.model.previous_position();
                Ok(self.publish(state_tx, false))
            }
            PlayerCommand::SetVolume { volume } => {
                self.volume = volume.clamp(0.0, 1.0);
                Ok(self.publish(state_tx, false))
            }
            PlayerCommand::SetMode { shuffle, loop_mode } => {
                if let Some(on) = shuffle {
                    self.model.set_shuffle(on);
                }
                if let Some(mode) = loop_mode {
                    self.model.set_loop_mode(mode);
                }
                Ok(self.publish(state_tx, shuffle.is_some()))
            }
            PlayerCommand::Play
            | PlayerCommand::Pause
            | PlayerCommand::Toggle
            | PlayerCommand::Stop
            | PlayerCommand::Seek { .. } => Err(ApiError::unsupported(
                "transport commands land with the engine port",
            )),
        }
    }

    async fn handle_set_queue(
        &mut self,
        req: SetQueueRequest,
        state_tx: &watch::Sender<PlayerState>,
    ) -> Result<CommandAck, ApiError> {
        let tracks = self.materializer.materialize(&req.context).await?;
        match req.mode {
            QueueMode::Replace => {
                self.model.replace(tracks);
                if let Some(on) = req.shuffle {
                    self.model.set_shuffle(on);
                }
                let len = self.model.len();
                if len > 0 {
                    let start = req.start_index.map(|i| i as usize).unwrap_or_else(|| {
                        if self.model.shuffle() {
                            use rand::RngExt;
                            rand::rng().random_range(0..len)
                        } else {
                            0
                        }
                    });
                    self.model.jump_to(start.min(len - 1));
                }
            }
            QueueMode::Append => self.model.add(tracks),
            QueueMode::PlayNext => self.model.insert_next(tracks),
        }
        Ok(self.publish(state_tx, true))
    }

    fn window(&mut self, page: Page) -> QueueWindow {
        let items = self
            .model
            .window(page.offset as usize, page.limit as usize)
            .into_iter()
            .map(|(pos, track)| QueueItem {
                index: pos as u32,
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
        let state = self.build_state();
        if queue_changed {
            let _ = self.events.send(ApiEvent::QueueChanged {
                rev: self.queue_rev,
                length: state.queue.length,
                index: state.queue.index,
            });
        }
        let _ = state_tx.send(state.clone());
        let _ = self.events.send(ApiEvent::PlayerState(Box::new(state)));
        CommandAck { rev: self.rev }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn build_state(&self) -> PlayerState {
        let track = self.model.current_track().map(now_playing_from);
        PlayerState {
            rev: self.rev,
            now_ms: self.now_ms(),
            track,
            queue: QueueSummary {
                rev: self.queue_rev,
                length: self.model.len() as u32,
                index: (!self.model.is_empty()).then(|| self.model.current_position() as u32),
                shuffle: self.model.shuffle(),
                loop_mode: self.model.loop_mode(),
            },
            volume: self.volume,
            ..Default::default()
        }
    }
}

/// Boundary translation from the internal track model to the wire summary:
/// the `u64::MAX` radio sentinel becomes an explicit kind + non-seekable flag
/// here and never reaches a client.
fn now_playing_from(track: &Track) -> NowPlaying {
    let radio = track.duration == u64::MAX;
    NowPlaying {
        key: track.id.uid().to_string(),
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
/// Android and all-in-one desktop link this directly; `HttpApi` in the client
/// crate is its wire twin, and the contract tests must hold for both.
pub struct LocalApi {
    session: SessionHandle,
}

impl LocalApi {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

#[async_trait::async_trait]
impl api::KopuzApi for LocalApi {
    async fn player_state(&self) -> Result<PlayerState, ApiError> {
        Ok(self.session.state())
    }

    async fn player_command(&self, cmd: PlayerCommand) -> Result<CommandAck, ApiError> {
        self.session.player_command(cmd).await
    }

    async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        self.session.queue_window(page).await
    }

    async fn set_queue(&self, req: SetQueueRequest) -> Result<CommandAck, ApiError> {
        self.session.set_queue(req).await
    }

    async fn tracks(
        &self,
        _filter: api::TrackFilter,
        _page: Page,
    ) -> Result<api::TrackPage, ApiError> {
        Err(ApiError::unsupported(
            "library reads land with the library service",
        ))
    }

    fn events(&self) -> api::EventStream {
        use futures_util::StreamExt;
        let rx = self.session.subscribe();
        futures_util::stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Ok(event) => Some((event, rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => Some((ApiEvent::Resync, rx)),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{ErrorCode, KopuzApi, LoopMode};
    use futures_util::StreamExt;

    struct StubLibrary;

    #[async_trait::async_trait]
    impl QueueMaterializer for StubLibrary {
        async fn materialize(&self, context: &QueueContext) -> Result<Vec<Track>, ApiError> {
            match context {
                QueueContext::Tracks { keys } => Ok(keys
                    .iter()
                    .map(|key| Track {
                        id: reader::models::TrackId::Local(std::path::PathBuf::from(key)),
                        cover: None,
                        album_id: String::new(),
                        title: key.clone(),
                        artist: String::new(),
                        album: String::new(),
                        duration: 100,
                        khz: 44,
                        bitrate: 320,
                        track_number: None,
                        disc_number: None,
                        musicbrainz_release_id: None,
                        musicbrainz_recording_id: None,
                        musicbrainz_track_id: None,
                        playlist_item_id: None,
                        artists: vec![],
                    })
                    .collect()),
                _ => Err(ApiError::unsupported("stub resolves raw tracks only")),
            }
        }
    }

    fn keys(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("/t/{i}.mp3")).collect()
    }

    fn local() -> LocalApi {
        LocalApi::new(SessionHandle::spawn(Arc::new(StubLibrary)))
    }

    fn replace(n: usize) -> SetQueueRequest {
        SetQueueRequest {
            mode: QueueMode::Replace,
            context: QueueContext::Tracks { keys: keys(n) },
            start_index: Some(0),
            shuffle: None,
        }
    }

    #[tokio::test]
    async fn set_queue_then_window_round_trips() {
        let local = local();
        let ack = local.set_queue(replace(3)).await.expect("set_queue");
        assert!(ack.rev > 0);

        let window = local.queue_window(Page::default()).await.expect("window");
        assert_eq!(window.total, 3);
        assert_eq!(window.items.len(), 3);
        assert_eq!(window.items[0].track.title, "/t/0.mp3");
        assert_eq!(window.rev, ack.rev);

        let state = local.player_state().await.expect("state");
        assert_eq!(state.queue.length, 3);
        assert_eq!(state.queue.index, Some(0));
        assert_eq!(
            state.track.as_ref().map(|t| t.title.as_str()),
            Some("/t/0.mp3")
        );
    }

    #[tokio::test]
    async fn next_and_previous_move_the_queue_position() {
        let local = local();
        local.set_queue(replace(3)).await.expect("set_queue");

        local
            .player_command(PlayerCommand::Next)
            .await
            .expect("next");
        let state = local.player_state().await.expect("state");
        assert_eq!(state.queue.index, Some(1));

        local
            .player_command(PlayerCommand::Previous)
            .await
            .expect("previous");
        let state = local.player_state().await.expect("state");
        assert_eq!(state.queue.index, Some(0));
    }

    #[tokio::test]
    async fn set_mode_and_events_flow_through() {
        let local = local();
        let mut events = local.events();

        local.set_queue(replace(4)).await.expect("set_queue");
        let first = events.next().await.expect("event");
        assert!(matches!(first, ApiEvent::QueueChanged { length: 4, .. }));
        let second = events.next().await.expect("event");
        assert!(matches!(second, ApiEvent::PlayerState(_)));

        local
            .player_command(PlayerCommand::SetMode {
                shuffle: Some(true),
                loop_mode: Some(LoopMode::Queue),
            })
            .await
            .expect("set_mode");
        let state = local.player_state().await.expect("state");
        assert!(state.queue.shuffle);
        assert_eq!(state.queue.loop_mode, LoopMode::Queue);
    }

    #[tokio::test]
    async fn transport_commands_report_unsupported_until_engine_lands() {
        let local = local();
        let err = local
            .player_command(PlayerCommand::Toggle)
            .await
            .expect_err("unsupported");
        assert_eq!(err.code, ErrorCode::Unsupported);
    }

    #[tokio::test]
    async fn radio_sentinel_becomes_wire_kind() {
        let track = Track {
            id: reader::models::TrackId::Local(std::path::PathBuf::from("radio:x:y")),
            cover: None,
            album_id: String::new(),
            title: "Station".into(),
            artist: String::new(),
            album: String::new(),
            duration: u64::MAX,
            khz: 0,
            bitrate: 0,
            track_number: None,
            disc_number: None,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            artists: vec![],
        };
        let now = now_playing_from(&track);
        assert_eq!(now.kind, TrackKind::Radio);
        assert_eq!(now.duration_ms, None);
        assert!(!now.seekable);
    }
}
