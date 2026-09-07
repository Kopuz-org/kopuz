//! `GrpcApi`: the wire twin of the daemon's in-process `LocalApi`.
//!
//! Implements [`api::KopuzApi`] over the daemon's gRPC surface, so a Rust
//! frontend can swap between embedding the daemon and attaching to one over
//! the socket without touching its data layer. The contract tests in the
//! daemon crate run the same assertions through both implementations.
//!
//! The transport is a Unix domain socket in the user's runtime dir. There
//! are no credentials: the socket's file mode is the access control, so the
//! kernel decides who may connect.
//!
//! Playback mutations use typed unary RPCs. `events()` owns a reconnecting
//! server-streaming subscription and resumes with the last sequence it
//! applied, surfacing a ring overrun as [`api::ApiEvent::Resync`].

use std::path::{Path, PathBuf};

use api::{
    ApiError, CommandAck, ConfigView, FavoritesView, JobKind, JobRef, JobStatus, KopuzApi, Page,
    PlayerCommand, PlayerState, QueueEdit, QueueWindow, SetQueueRequest, TrackFilter, TrackPage,
};
use hyper_util::rt::TokioIo;
use proto::convert;
use proto::kopuz_client::KopuzClient;
use tokio::net::UnixStream;
use tonic::Request;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

type Client = KopuzClient<Channel>;

pub struct GrpcApi {
    path: PathBuf,
    client: Client,
}

fn wire_error(status: tonic::Status) -> ApiError {
    proto::status::from_status(&status)
}

impl GrpcApi {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `path` is the daemon's socket. The connector dials it lazily, so
    /// construction never blocks and never fails on a daemon that has not
    /// started yet -- the first call reports that instead.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, ApiError> {
        let path = path.into();
        let dial = path.clone();
        // tonic still needs a syntactically valid URI to fill the HTTP/2
        // :authority header. The connector below ignores it; nothing
        // resolves this name.
        let channel = Endpoint::from_static("http://kopuz.invalid").connect_with_connector_lazy(
            service_fn(move |_: Uri| {
                let dial = dial.clone();
                async move { UnixStream::connect(dial).await.map(TokioIo::new) }
            }),
        );
        Ok(Self {
            path,
            client: KopuzClient::new(channel),
        })
    }

    fn client(&self) -> Client {
        self.client.clone()
    }
}

#[async_trait::async_trait]
impl KopuzApi for GrpcApi {
    async fn player_state(&self) -> Result<PlayerState, ApiError> {
        let state = self
            .client()
            .get_player_state(Request::new(proto::GetPlayerStateRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::player_state_from_proto(state.get_ref()))
    }

    async fn player_command(&self, command: PlayerCommand) -> Result<CommandAck, ApiError> {
        let response = match command {
            PlayerCommand::Play => {
                self.client()
                    .play(Request::new(proto::PlayRequest {}))
                    .await
            }
            PlayerCommand::Pause => {
                self.client()
                    .pause(Request::new(proto::PauseRequest {}))
                    .await
            }
            PlayerCommand::Toggle => {
                self.client()
                    .toggle(Request::new(proto::ToggleRequest {}))
                    .await
            }
            PlayerCommand::Next => {
                self.client()
                    .next(Request::new(proto::NextRequest {}))
                    .await
            }
            PlayerCommand::Previous => {
                self.client()
                    .previous(Request::new(proto::PreviousRequest {}))
                    .await
            }
            PlayerCommand::Stop => {
                self.client()
                    .stop(Request::new(proto::StopRequest {}))
                    .await
            }
            PlayerCommand::Seek { position_ms } => {
                self.client()
                    .seek(Request::new(proto::Seek { position_ms }))
                    .await
            }
            PlayerCommand::SetVolume { volume } => {
                self.client()
                    .set_volume(Request::new(proto::SetVolume { volume }))
                    .await
            }
            PlayerCommand::SetMode { shuffle, loop_mode } => {
                self.client()
                    .set_mode(Request::new(proto::SetMode {
                        shuffle,
                        r#loop: loop_mode.map(|mode| convert::loop_to_proto(mode) as i32),
                    }))
                    .await
            }
        }
        .map_err(wire_error)?;
        Ok(CommandAck {
            rev: response.get_ref().rev,
        })
    }

    async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError> {
        let window = self
            .client()
            .get_queue(Request::new(convert::page_to_proto(page)))
            .await
            .map_err(wire_error)?;
        Ok(convert::queue_window_from_proto(window.get_ref()))
    }

    async fn set_queue(&self, request: SetQueueRequest) -> Result<CommandAck, ApiError> {
        let ack = self
            .client()
            .set_queue(Request::new(convert::set_queue_to_proto(&request)))
            .await
            .map_err(wire_error)?;
        Ok(CommandAck {
            rev: ack.get_ref().rev,
        })
    }

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError> {
        let ack = self
            .client()
            .edit_queue(Request::new(convert::queue_edit_to_proto(&edit)))
            .await
            .map_err(wire_error)?;
        Ok(CommandAck {
            rev: ack.get_ref().rev,
        })
    }

    async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError> {
        let tracks = self
            .client()
            .get_tracks(Request::new(proto::TracksRequest {
                filter: Some(convert::track_filter_to_proto(&filter)),
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn config(&self) -> Result<ConfigView, ApiError> {
        let view = self
            .client()
            .get_config(Request::new(proto::GetConfigRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::config_view_from_proto(view.get_ref()))
    }

    async fn patch_config(&self, patch: serde_json::Value) -> Result<ConfigView, ApiError> {
        let view = self
            .client()
            .patch_config(Request::new(proto::ConfigPatch {
                merge_patch_json: patch.to_string(),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::config_view_from_proto(view.get_ref()))
    }

    async fn favorites(&self) -> Result<FavoritesView, ApiError> {
        let favorites = self
            .client()
            .get_favorites(Request::new(proto::GetFavoritesRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::favorites_from_proto(favorites.get_ref()))
    }

    async fn set_favorite(&self, key: String, favorite: bool) -> Result<(), ApiError> {
        self.client()
            .set_favorite(Request::new(proto::FavoriteRequest { key, favorite }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn start_job(&self, kind: JobKind) -> Result<JobRef, ApiError> {
        let job = self
            .client()
            .start_job(Request::new(proto::StartJobRequest {
                kind: convert::job_kind_to_proto(kind) as i32,
            }))
            .await
            .map_err(wire_error)?;
        Ok(JobRef {
            job_id: job.get_ref().job_id.clone(),
        })
    }

    async fn folder_tracks(&self, prefix: String, page: Page) -> Result<api::TrackPage, ApiError> {
        let tracks = self
            .client()
            .get_folder_tracks(Request::new(proto::FolderRequest {
                prefix,
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn lyrics(&self, key: String) -> Result<api::LyricsView, ApiError> {
        let lyrics = self
            .client()
            .get_lyrics(Request::new(proto::TrackRef { key }))
            .await
            .map_err(wire_error)?;
        Ok(convert::lyrics_from_proto(lyrics.get_ref()))
    }

    async fn stats(&self) -> Result<api::StatsView, ApiError> {
        let stats = self
            .client()
            .get_stats(Request::new(proto::GetStatsRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::stats_from_proto(stats.get_ref()))
    }

    async fn download(&self, keys: Vec<String>) -> Result<JobRef, ApiError> {
        let job = self
            .client()
            .start_downloads(Request::new(proto::DownloadRequest { keys }))
            .await
            .map_err(wire_error)?;
        Ok(JobRef {
            job_id: job.get_ref().job_id.clone(),
        })
    }

    async fn downloads(&self) -> Result<Vec<String>, ApiError> {
        let list = self
            .client()
            .get_downloads(Request::new(proto::GetDownloadsRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(list.get_ref().keys.clone())
    }

    async fn remove_download(&self, key: String) -> Result<(), ApiError> {
        self.client()
            .remove_download(Request::new(proto::TrackRef { key }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn jobs(&self) -> Result<Vec<JobStatus>, ApiError> {
        let jobs = self
            .client()
            .get_jobs(Request::new(proto::GetJobsRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(jobs
            .get_ref()
            .jobs
            .iter()
            .map(convert::job_status_from_proto)
            .collect())
    }

    async fn cancel_job(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .cancel_job(Request::new(proto::JobId { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    /// Holds a server-streaming subscription, reconnecting with the last seen
    /// sequence after drops. Unknown event kinds are skipped, matching the
    /// protocol's forward-compatibility rule; a gap past the daemon's replay
    /// ring surfaces as `ApiEvent::Resync`.
    fn events(&self) -> api::EventStream {
        use futures_util::StreamExt;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let client = self.client.clone();
        tokio::spawn(run_event_loop(client, tx));
        futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        })
        .boxed()
    }
}

async fn run_event_loop(client: Client, tx: tokio::sync::mpsc::UnboundedSender<api::ApiEvent>) {
    let mut last_sequence: u64 = 0;
    loop {
        match stream_once(client.clone(), &tx, &mut last_sequence).await {
            Ok(()) => return,
            Err(error) => {
                tracing::debug!(%error, "event stream dropped; reconnecting");
            }
        }
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn stream_once(
    mut client: Client,
    tx: &tokio::sync::mpsc::UnboundedSender<api::ApiEvent>,
    last_sequence: &mut u64,
) -> Result<(), ApiError> {
    let mut inbound = client
        .subscribe(Request::new(proto::SubscribeRequest {
            after_sequence: *last_sequence,
        }))
        .await
        .map_err(wire_error)?
        .into_inner();
    loop {
        match inbound.message().await {
            Ok(Some(proto::EventEnvelope { sequence, event })) => {
                if sequence > 0 {
                    *last_sequence = sequence;
                }
                if let Some(event) = event.and_then(|event| convert::event_from_proto(&event))
                    && tx.send(event).is_err()
                {
                    return Ok(());
                }
            }
            Ok(None) => return Err(ApiError::internal("event stream ended")),
            Err(status) => return Err(wire_error(status)),
        }
    }
}
