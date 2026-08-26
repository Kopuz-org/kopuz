//! The tonic shell over a running session: the gRPC wire.
//!
//! Reads and mutations are unary RPCs. Event delivery is a server-streaming
//! subscription with a replay cursor, so the wire follows gRPC's native
//! request/response and streaming semantics instead of simulating an HTTP
//! request/response protocol inside a bidirectional stream.
//! Bearer auth rides metadata and is checked constant-time; the reflection
//! services are served unauthenticated so `grpcurl` can list the schema,
//! which is public in the repository anyway.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use api::{ApiError, ApiEvent, KopuzApi};
use futures_util::Stream;
use proto::convert;
use proto::kopuz_server::{Kopuz, KopuzServer};
use tokio::sync::broadcast;
use tonic::{Request, Response, Status};

use crate::session::SessionHandle;

pub struct GrpcState {
    pub api: Arc<dyn KopuzApi>,
    /// Entity-addressed artwork; `None` makes GetArtwork answer unsupported.
    pub artwork: Option<Arc<crate::artwork::ArtworkService>>,
    /// Event source with sequence numbers and the replay ring; the trait's
    /// `events()` strips ids, and Subscribe cursors need them.
    pub session: SessionHandle,
    pub token: String,
    pub started: Instant,
}

pub struct KopuzGrpc(Arc<GrpcState>);

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn failed(error: ApiError) -> Status {
    proto::status::to_status(error)
}

const ARTWORK_CHUNK: usize = 256 * 1024;

type ServerStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

fn event_message(sequence: u64, event: &ApiEvent) -> proto::EventEnvelope {
    proto::EventEnvelope {
        sequence,
        event: Some(convert::event_to_proto(event)),
    }
}

fn resync_message() -> proto::EventEnvelope {
    event_message(0, &ApiEvent::Resync)
}

impl KopuzGrpc {
    async fn player_mutation(
        &self,
        command: api::PlayerCommand,
    ) -> Result<Response<proto::MutationResult>, Status> {
        let ack = self.0.api.player_command(command).await.map_err(failed)?;
        Ok(Response::new(proto::MutationResult { rev: ack.rev }))
    }
}

/// Replay backlog first, then the live broadcast. `floor` drops the live
/// events the replay snapshot already covered.
struct EventSubscription {
    pending: VecDeque<proto::EventEnvelope>,
    live: broadcast::Receiver<(u64, ApiEvent)>,
    floor: u64,
}

impl EventSubscription {
    async fn next(mut self) -> Option<(Result<proto::EventEnvelope, Status>, Self)> {
        if let Some(message) = self.pending.pop_front() {
            return Some((Ok(message), self));
        }

        loop {
            match self.live.recv().await {
                Ok((sequence, event)) => {
                    if sequence <= self.floor {
                        continue;
                    }
                    self.floor = sequence;
                    return Some((Ok(event_message(sequence, &event)), self));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    return Some((Ok(resync_message()), self));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

#[tonic::async_trait]
impl Kopuz for KopuzGrpc {
    type SubscribeStream = ServerStream<proto::EventEnvelope>;
    type GetArtworkStream = ServerStream<proto::ArtworkChunk>;

    async fn subscribe(
        &self,
        request: Request<proto::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let after_sequence = request.into_inner().after_sequence;
        // Subscribe before taking the replay snapshot so events emitted while
        // this RPC is being set up remain buffered for the live phase.
        let live = self.0.session.subscribe();
        let (needs_resync, replayed) = if after_sequence > 0 {
            self.0.session.replay_since(after_sequence)
        } else {
            (false, Vec::new())
        };
        let floor = replayed
            .last()
            .map(|(sequence, _)| *sequence)
            .or_else(|| (!needs_resync && after_sequence > 0).then_some(after_sequence))
            .unwrap_or(0);
        let mut pending = VecDeque::new();
        if needs_resync {
            pending.push_back(resync_message());
        }
        pending.extend(
            replayed
                .iter()
                .map(|(sequence, event)| event_message(*sequence, event)),
        );
        let subscription = EventSubscription {
            pending,
            live,
            floor,
        };
        let stream = futures_util::stream::unfold(subscription, |subscription| subscription.next());
        Ok(Response::new(Box::pin(stream)))
    }

    async fn play(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Play).await
    }

    async fn pause(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Pause).await
    }

    async fn toggle(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Toggle).await
    }

    async fn next(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Next).await
    }

    async fn previous(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Previous).await
    }

    async fn stop(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Stop).await
    }

    async fn seek(
        &self,
        request: Request<proto::Seek>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Seek {
            position_ms: request.get_ref().position_ms,
        })
        .await
    }

    async fn set_volume(
        &self,
        request: Request<proto::SetVolume>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::SetVolume {
            volume: request.get_ref().volume,
        })
        .await
    }

    async fn set_mode(
        &self,
        request: Request<proto::SetMode>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::SetMode {
            shuffle: request.get_ref().shuffle,
            loop_mode: request.get_ref().r#loop.map(convert::loop_from_proto),
        })
        .await
    }

    async fn get_status(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::DaemonStatus>, Status> {
        Ok(Response::new(proto::DaemonStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            api_version: api::API_VERSION,
            uptime_secs: self.0.started.elapsed().as_secs(),
        }))
    }

    async fn get_player_state(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::PlayerState>, Status> {
        let state = self.0.api.player_state().await.map_err(failed)?;
        Ok(Response::new(convert::player_state_to_proto(&state)))
    }

    async fn get_queue(
        &self,
        request: Request<proto::Page>,
    ) -> Result<Response<proto::QueueWindow>, Status> {
        let page = convert::page_from_proto(Some(request.get_ref()));
        let window = self.0.api.queue_window(page).await.map_err(failed)?;
        Ok(Response::new(convert::queue_window_to_proto(&window)))
    }

    async fn get_tracks(
        &self,
        request: Request<proto::TracksRequest>,
    ) -> Result<Response<proto::TrackPage>, Status> {
        let request = request.get_ref();
        let filter = request
            .filter
            .as_ref()
            .map(convert::track_filter_from_proto)
            .unwrap_or_default();
        let page = convert::page_from_proto(request.page.as_ref());
        let tracks = self.0.api.tracks(filter, page).await.map_err(failed)?;
        Ok(Response::new(convert::track_page_to_proto(&tracks)))
    }

    async fn get_folder_tracks(
        &self,
        request: Request<proto::FolderRequest>,
    ) -> Result<Response<proto::TrackPage>, Status> {
        let request = request.get_ref();
        let page = convert::page_from_proto(request.page.as_ref());
        let tracks = self
            .0
            .api
            .folder_tracks(request.prefix.clone(), page)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::track_page_to_proto(&tracks)))
    }

    async fn get_stats(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::Stats>, Status> {
        let stats = self.0.api.stats().await.map_err(failed)?;
        Ok(Response::new(convert::stats_to_proto(&stats)))
    }

    async fn get_lyrics(
        &self,
        request: Request<proto::TrackRef>,
    ) -> Result<Response<proto::Lyrics>, Status> {
        let lyrics = self
            .0
            .api
            .lyrics(request.get_ref().key.clone())
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::lyrics_to_proto(&lyrics)))
    }

    async fn get_favorites(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::Favorites>, Status> {
        let favorites = self.0.api.favorites().await.map_err(failed)?;
        Ok(Response::new(convert::favorites_to_proto(&favorites)))
    }

    async fn get_jobs(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::JobList>, Status> {
        let jobs = self.0.api.jobs().await.map_err(failed)?;
        Ok(Response::new(proto::JobList {
            jobs: jobs.iter().map(convert::job_status_to_proto).collect(),
        }))
    }

    async fn get_downloads(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::DownloadList>, Status> {
        let keys = self.0.api.downloads().await.map_err(failed)?;
        Ok(Response::new(proto::DownloadList { keys }))
    }

    async fn get_config(
        &self,
        _request: Request<proto::Empty>,
    ) -> Result<Response<proto::ConfigView>, Status> {
        let view = self.0.api.config().await.map_err(failed)?;
        Ok(Response::new(convert::config_view_to_proto(&view)))
    }

    async fn set_queue(
        &self,
        request: Request<proto::SetQueueRequest>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        let request = convert::set_queue_from_proto(request.get_ref())
            .ok_or_else(|| Status::invalid_argument("missing queue context"))?;
        let ack = self.0.api.set_queue(request).await.map_err(failed)?;
        Ok(Response::new(proto::MutationResult { rev: ack.rev }))
    }

    async fn edit_queue(
        &self,
        request: Request<proto::QueueEditRequest>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        let edit = convert::queue_edit_from_proto(request.get_ref())
            .ok_or_else(|| Status::invalid_argument("missing queue edit op"))?;
        let ack = self.0.api.queue_edit(edit).await.map_err(failed)?;
        Ok(Response::new(proto::MutationResult { rev: ack.rev }))
    }

    async fn set_favorite(
        &self,
        request: Request<proto::FavoriteRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let request = request.get_ref();
        self.0
            .api
            .set_favorite(request.key.clone(), request.favorite)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn start_job(
        &self,
        request: Request<proto::StartJobRequest>,
    ) -> Result<Response<proto::JobRef>, Status> {
        let kind = convert::job_kind_from_proto(request.get_ref().kind);
        let job = self.0.api.start_job(kind).await.map_err(failed)?;
        Ok(Response::new(proto::JobRef { job_id: job.job_id }))
    }

    async fn cancel_job(
        &self,
        request: Request<proto::JobId>,
    ) -> Result<Response<proto::Empty>, Status> {
        self.0
            .api
            .cancel_job(request.get_ref().id.clone())
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn start_downloads(
        &self,
        request: Request<proto::DownloadRequest>,
    ) -> Result<Response<proto::JobRef>, Status> {
        let job = self
            .0
            .api
            .download(request.get_ref().keys.clone())
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::JobRef { job_id: job.job_id }))
    }

    async fn remove_download(
        &self,
        request: Request<proto::TrackRef>,
    ) -> Result<Response<proto::Empty>, Status> {
        self.0
            .api
            .remove_download(request.get_ref().key.clone())
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn patch_config(
        &self,
        request: Request<proto::ConfigPatch>,
    ) -> Result<Response<proto::ConfigView>, Status> {
        let patch: serde_json::Value = serde_json::from_str(&request.get_ref().merge_patch_json)
            .map_err(|error| Status::invalid_argument(format!("invalid merge patch: {error}")))?;
        let view = self.0.api.patch_config(patch).await.map_err(failed)?;
        Ok(Response::new(convert::config_view_to_proto(&view)))
    }

    #[allow(clippy::result_large_err)]
    async fn get_artwork(
        &self,
        request: Request<proto::ArtworkRequest>,
    ) -> Result<Response<Self::GetArtworkStream>, Status> {
        use crate::artwork::ArtworkEntity;
        let Some(service) = &self.0.artwork else {
            return Err(failed(ApiError::unsupported(
                "this daemon runs without artwork",
            )));
        };
        let request = request.get_ref();
        let entity = match request.entity.as_ref() {
            Some(proto::artwork_request::Entity::Track(track)) => ArtworkEntity::Track(track),
            Some(proto::artwork_request::Entity::Album(album)) => ArtworkEntity::Album(album),
            Some(proto::artwork_request::Entity::Artist(artist)) => ArtworkEntity::Artist(artist),
            None => {
                return Err(Status::invalid_argument(
                    "pass one of track, album, or artist",
                ));
            }
        };
        let payload = service.fetch(entity, request.hq).await.map_err(failed)?;
        let content_type = payload.content_type.to_string();
        let chunks: Vec<Result<proto::ArtworkChunk, Status>> = payload
            .bytes
            .chunks(ARTWORK_CHUNK)
            .enumerate()
            .map(|(index, chunk)| {
                Ok(proto::ArtworkChunk {
                    content_type: if index == 0 {
                        content_type.clone()
                    } else {
                        String::new()
                    },
                    data: chunk.to_vec(),
                })
            })
            .collect();
        Ok(Response::new(Box::pin(futures_util::stream::iter(chunks))))
    }
}

/// Serve the daemon on `listener` until the future is dropped. Reflection
/// (v1 and v1alpha) is registered so `grpcurl` works out of the box.
/// `result_large_err` is tonic's own Status type; nothing to shrink here.
#[allow(clippy::result_large_err)]
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: Arc<GrpcState>,
) -> std::io::Result<()> {
    let token = state.token.clone();
    let auth = move |request: Request<()>| -> Result<Request<()>, Status> {
        let provided = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if provided.is_some_and(|value| constant_time_eq(value.as_bytes(), token.as_bytes())) {
            Ok(request)
        } else {
            Err(Status::unauthenticated("missing or invalid bearer token"))
        }
    };
    let reflection_v1 = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .map_err(std::io::Error::other)?;
    let reflection_v1alpha = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1alpha()
        .map_err(std::io::Error::other)?;
    tonic::transport::Server::builder()
        .add_service(reflection_v1)
        .add_service(reflection_v1alpha)
        .add_service(KopuzServer::with_interceptor(KopuzGrpc(state), auth))
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
        .await
        .map_err(std::io::Error::other)
}
