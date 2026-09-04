//! The tonic shell over a running session: the gRPC wire.
//!
//! One bidirectional `Attach` stream is the control plane (commands in,
//! acks and every event out, with sequence-based resume against the replay
//! ring); reads and non-transport mutations are unary.
//! Bearer auth rides metadata and is checked constant-time; the reflection
//! services are served unauthenticated so `grpcurl` can list the schema,
//! which is public in the repository anyway.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use api::{ApiError, ApiEvent, KopuzApi};
use futures_util::Stream;
use proto::convert;
use proto::kopuz_server::{Kopuz, KopuzServer};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use crate::session::SessionHandle;

pub struct GrpcState {
    pub api: Arc<dyn KopuzApi>,
    /// Entity-addressed artwork; `None` makes GetArtwork answer unsupported.
    pub artwork: Option<Arc<crate::artwork::ArtworkService>>,
    /// Event source with sequence numbers and the replay ring; the trait's
    /// `events()` strips ids, and Attach resume needs them.
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

fn event_message(sequence: u64, event: &ApiEvent) -> proto::ServerMessage {
    proto::ServerMessage {
        sequence,
        msg: Some(proto::server_message::Msg::Event(convert::event_to_proto(
            event,
        ))),
    }
}

fn resync_message() -> proto::ServerMessage {
    event_message(0, &ApiEvent::Resync)
}

#[tonic::async_trait]
impl Kopuz for KopuzGrpc {
    type AttachStream = ServerStream<proto::ServerMessage>;
    type GetArtworkStream = ServerStream<proto::ArtworkChunk>;

    async fn attach(
        &self,
        request: Request<Streaming<proto::ClientMessage>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let mut inbound = request.into_inner();
        let state = self.0.clone();
        let (tx, rx) = mpsc::channel::<Result<proto::ServerMessage, Status>>(64);
        tokio::spawn(async move {
            let hello = match inbound.message().await {
                Ok(Some(proto::ClientMessage {
                    msg: Some(proto::client_message::Msg::Hello(hello)),
                })) => hello,
                Ok(_) => {
                    let _ = tx
                        .send(Err(Status::invalid_argument(
                            "the first Attach message must be Hello",
                        )))
                        .await;
                    return;
                }
                Err(_) => return,
            };

            let mut live = state.session.subscribe();
            let (needs_resync, replayed) = if hello.last_sequence > 0 {
                state.session.replay_since(hello.last_sequence)
            } else {
                (false, Vec::new())
            };
            let mut floor = replayed
                .last()
                .map(|(sequence, _)| *sequence)
                .or(if needs_resync {
                    None
                } else if hello.last_sequence > 0 {
                    Some(hello.last_sequence)
                } else {
                    None
                })
                .unwrap_or(0);
            if needs_resync && tx.send(Ok(resync_message())).await.is_err() {
                return;
            }
            for (sequence, event) in &replayed {
                if tx.send(Ok(event_message(*sequence, event))).await.is_err() {
                    return;
                }
            }

            let (command_tx, mut commands) = mpsc::channel::<proto::Command>(64);
            let api = state.api.clone();
            let ack_tx = tx.clone();
            tokio::spawn(async move {
                while let Some(command) = commands.recv().await {
                    let request_id = command.request_id;
                    let ack = match convert::command_from_proto(&command) {
                        Some(command) => match api.player_command(command).await {
                            Ok(ack) => proto::Ack {
                                request_id,
                                rev: ack.rev,
                                error: None,
                            },
                            Err(error) => proto::Ack {
                                request_id,
                                rev: 0,
                                error: Some(convert::api_error_to_proto(&error)),
                            },
                        },
                        None => proto::Ack {
                            request_id,
                            rev: 0,
                            error: Some(convert::api_error_to_proto(&ApiError::invalid_input(
                                "unknown command",
                            ))),
                        },
                    };
                    let message = proto::ServerMessage {
                        sequence: 0,
                        msg: Some(proto::server_message::Msg::Ack(ack)),
                    };
                    if ack_tx.send(Ok(message)).await.is_err() {
                        return;
                    }
                }
            });

            // An events-only client half-closes after Hello; that ends the
            // command lane, not the stream.
            let mut inbound_done = false;
            loop {
                tokio::select! {
                    received = live.recv() => match received {
                        Ok((sequence, event)) => {
                            if sequence <= floor {
                                continue;
                            }
                            floor = sequence;
                            if tx.send(Ok(event_message(sequence, &event))).await.is_err() {
                                return;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if tx.send(Ok(resync_message())).await.is_err() {
                                return;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    },
                    incoming = inbound.message(), if !inbound_done => match incoming {
                        Ok(Some(proto::ClientMessage {
                            msg: Some(proto::client_message::Msg::Command(command)),
                        })) => {
                            if command_tx.send(command).await.is_err() {
                                return;
                            }
                        }
                        Ok(Some(_)) => continue,
                        Ok(None) => inbound_done = true,
                        Err(_) => return,
                    },
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
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
    ) -> Result<Response<proto::Ack>, Status> {
        let request = convert::set_queue_from_proto(request.get_ref())
            .ok_or_else(|| Status::invalid_argument("missing queue context"))?;
        let ack = self.0.api.set_queue(request).await.map_err(failed)?;
        Ok(Response::new(proto::Ack {
            request_id: 0,
            rev: ack.rev,
            error: None,
        }))
    }

    async fn edit_queue(
        &self,
        request: Request<proto::QueueEditRequest>,
    ) -> Result<Response<proto::Ack>, Status> {
        let edit = convert::queue_edit_from_proto(request.get_ref())
            .ok_or_else(|| Status::invalid_argument("missing queue edit op"))?;
        let ack = self.0.api.queue_edit(edit).await.map_err(failed)?;
        Ok(Response::new(proto::Ack {
            request_id: 0,
            rev: ack.rev,
            error: None,
        }))
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
    let bind_addr = listener.local_addr()?;
    validate_plaintext_bind(bind_addr)?;
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

fn validate_plaintext_bind(bind_addr: std::net::SocketAddr) -> std::io::Result<()> {
    if bind_addr.ip().is_loopback() {
        return Ok(());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("plaintext gRPC may only bind to a loopback address, not {bind_addr}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::validate_plaintext_bind;

    #[test]
    fn plaintext_bind_requires_loopback() {
        assert!(validate_plaintext_bind("127.0.0.1:1".parse().expect("IPv4 address")).is_ok());
        assert!(validate_plaintext_bind("[::1]:1".parse().expect("IPv6 address")).is_ok());
        let error = validate_plaintext_bind("0.0.0.0:1".parse().expect("wildcard address"))
            .expect_err("wildcard plaintext listener refused");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
