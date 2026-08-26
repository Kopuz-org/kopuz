//! `GrpcApi`: the wire twin of the daemon's in-process `LocalApi`.
//!
//! Implements [`api::KopuzApi`] over the daemon's gRPC surface, so a Rust
//! frontend can swap between embedding the daemon and attaching to a remote
//! one without touching its data layer. The contract tests in the daemon
//! crate run the same assertions through both implementations.
//!
//! Transport commands ride a short-lived Attach stream each (Hello, the
//! command, its Ack); `events()` holds a long-lived Attach that reconnects
//! with the last seen sequence, surfacing a ring overrun as
//! [`api::ApiEvent::Resync`].

use api::{
    ApiError, CommandAck, ConfigView, FavoritesView, JobKind, JobRef, JobStatus, KopuzApi, Page,
    PlayerCommand, PlayerState, QueueEdit, QueueWindow, SetQueueRequest, TrackFilter, TrackPage,
};
use proto::convert;
use proto::kopuz_client::KopuzClient;
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint};

type Client = KopuzClient<InterceptedService<Channel, AuthInterceptor>>;

#[derive(Clone)]
pub struct AuthInterceptor {
    header: MetadataValue<tonic::metadata::Ascii>,
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, tonic::Status> {
        request
            .metadata_mut()
            .insert("authorization", self.header.clone());
        Ok(request)
    }
}

pub struct GrpcApi {
    addr: String,
    client: Client,
}

fn wire_error(status: tonic::Status) -> ApiError {
    proto::status::from_status(&status)
}

impl GrpcApi {
    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// `addr` is `host:port` (or a full `http://` URL); the channel connects
    /// lazily on the first call, so construction never blocks.
    pub fn new(addr: impl Into<String>, token: impl Into<String>) -> Result<Self, ApiError> {
        let mut addr = addr.into();
        if !addr.starts_with("http://") && !addr.starts_with("https://") {
            addr = format!("http://{addr}");
        }
        let header: MetadataValue<tonic::metadata::Ascii> = format!("Bearer {}", token.into())
            .parse()
            .map_err(|_| ApiError::invalid_input("token is not valid header material"))?;
        let channel = Endpoint::from_shared(addr.clone())
            .map_err(|error| ApiError::invalid_input(format!("bad daemon address: {error}")))?
            .connect_lazy();
        Ok(Self {
            addr,
            client: KopuzClient::with_interceptor(channel, AuthInterceptor { header }),
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
            .get_player_state(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::player_state_from_proto(state.get_ref()))
    }

    async fn player_command(&self, command: PlayerCommand) -> Result<CommandAck, ApiError> {
        let hello = proto::ClientMessage {
            msg: Some(proto::client_message::Msg::Hello(proto::Hello {
                last_sequence: 0,
            })),
        };
        let message = proto::ClientMessage {
            msg: Some(proto::client_message::Msg::Command(
                convert::command_to_proto(&command, 1),
            )),
        };
        let outbound = futures_util::stream::iter(vec![hello, message]);
        let mut inbound = self
            .client()
            .attach(Request::new(outbound))
            .await
            .map_err(wire_error)?
            .into_inner();
        loop {
            match inbound.message().await {
                Ok(Some(proto::ServerMessage {
                    msg: Some(proto::server_message::Msg::Ack(ack)),
                    ..
                })) if ack.request_id == 1 => {
                    return match ack.error {
                        Some(error) => Err(convert::api_error_from_proto(&error)),
                        None => Ok(CommandAck { rev: ack.rev }),
                    };
                }
                Ok(Some(_)) => continue,
                Ok(None) => {
                    return Err(ApiError::internal(
                        "the daemon closed the stream before answering",
                    ));
                }
                Err(status) => return Err(wire_error(status)),
            }
        }
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
            .get_config(Request::new(proto::Empty {}))
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
            .get_favorites(Request::new(proto::Empty {}))
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
            .get_stats(Request::new(proto::Empty {}))
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
            .get_downloads(Request::new(proto::Empty {}))
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
            .get_jobs(Request::new(proto::Empty {}))
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

    /// Holds an Attach stream, reconnecting with the last seen sequence
    /// after drops. Unknown event kinds are skipped, matching the protocol's
    /// forward-compatibility rule; a gap past the daemon's replay ring
    /// surfaces as `ApiEvent::Resync`.
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
    let hello = proto::ClientMessage {
        msg: Some(proto::client_message::Msg::Hello(proto::Hello {
            last_sequence: *last_sequence,
        })),
    };
    let outbound = futures_util::stream::once(async move { hello });
    let mut inbound = client
        .attach(Request::new(outbound))
        .await
        .map_err(wire_error)?
        .into_inner();
    loop {
        match inbound.message().await {
            Ok(Some(proto::ServerMessage {
                sequence,
                msg: Some(proto::server_message::Msg::Event(event)),
            })) => {
                if sequence > 0 {
                    *last_sequence = sequence;
                }
                if let Some(event) = convert::event_from_proto(&event)
                    && tx.send(event).is_err()
                {
                    return Ok(());
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => return Err(ApiError::internal("event stream ended")),
            Err(status) => return Err(wire_error(status)),
        }
    }
}
