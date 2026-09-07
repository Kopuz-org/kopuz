//! The tonic shell over a running session: the gRPC wire.
//!
//! Reads and mutations are unary RPCs. Event delivery is a server-streaming
//! subscription with a replay cursor, so the wire follows gRPC's native
//! request/response and streaming semantics instead of simulating an HTTP
//! request/response protocol inside a bidirectional stream.
//! The reflection services are registered so `grpcurl` can list the schema,
//! which is public in the repository anyway.

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
    pub started: Instant,
}

pub struct KopuzGrpc(Arc<GrpcState>);

fn failed(error: ApiError) -> Status {
    proto::status::to_status(error)
}

const ARTWORK_CHUNK: usize = 256 * 1024;

type ServerStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

fn event_message(event: &ApiEvent) -> proto::EventEnvelope {
    proto::EventEnvelope {
        event: Some(convert::event_to_proto(event)),
    }
}

fn resync_message() -> proto::EventEnvelope {
    event_message(&ApiEvent::Resync)
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

/// The live broadcast, nothing else. A subscriber that falls behind is
/// told to resync; there is no backlog to replay, because a peer that lost
/// this stream lost the process that owns it.
struct EventSubscription {
    live: broadcast::Receiver<ApiEvent>,
}

impl EventSubscription {
    async fn next(mut self) -> Option<(Result<proto::EventEnvelope, Status>, Self)> {
        match self.live.recv().await {
            Ok(event) => Some((Ok(event_message(&event)), self)),
            Err(broadcast::error::RecvError::Lagged(_)) => Some((Ok(resync_message()), self)),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

#[tonic::async_trait]
impl Kopuz for KopuzGrpc {
    type SubscribeStream = ServerStream<proto::EventEnvelope>;
    type GetArtworkStream = ServerStream<proto::ArtworkChunk>;

    async fn subscribe(
        &self,
        _request: Request<proto::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let live = self.0.session.subscribe();
        let stream = futures_util::stream::unfold(EventSubscription { live }, |subscription| {
            subscription.next()
        });
        Ok(Response::new(Box::pin(stream)))
    }

    async fn play(
        &self,
        _request: Request<proto::PlayRequest>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Play).await
    }

    async fn pause(
        &self,
        _request: Request<proto::PauseRequest>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Pause).await
    }

    async fn toggle(
        &self,
        _request: Request<proto::ToggleRequest>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Toggle).await
    }

    async fn next(
        &self,
        _request: Request<proto::NextRequest>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Next).await
    }

    async fn previous(
        &self,
        _request: Request<proto::PreviousRequest>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::Previous).await
    }

    async fn stop(
        &self,
        _request: Request<proto::StopRequest>,
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
        _request: Request<proto::GetStatusRequest>,
    ) -> Result<Response<proto::DaemonStatus>, Status> {
        Ok(Response::new(proto::DaemonStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: self.0.started.elapsed().as_secs(),
        }))
    }

    async fn get_player_state(
        &self,
        _request: Request<proto::GetPlayerStateRequest>,
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
        _request: Request<proto::GetStatsRequest>,
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
        _request: Request<proto::GetFavoritesRequest>,
    ) -> Result<Response<proto::Favorites>, Status> {
        let favorites = self.0.api.favorites().await.map_err(failed)?;
        Ok(Response::new(convert::favorites_to_proto(&favorites)))
    }

    async fn get_jobs(
        &self,
        _request: Request<proto::GetJobsRequest>,
    ) -> Result<Response<proto::JobList>, Status> {
        let jobs = self.0.api.jobs().await.map_err(failed)?;
        Ok(Response::new(proto::JobList {
            jobs: jobs.iter().map(convert::job_status_to_proto).collect(),
        }))
    }

    async fn get_downloads(
        &self,
        _request: Request<proto::GetDownloadsRequest>,
    ) -> Result<Response<proto::DownloadList>, Status> {
        let keys = self.0.api.downloads().await.map_err(failed)?;
        Ok(Response::new(proto::DownloadList { keys }))
    }

    async fn get_config(
        &self,
        _request: Request<proto::GetConfigRequest>,
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
    ) -> Result<Response<proto::SetFavoriteResponse>, Status> {
        let request = request.get_ref();
        self.0
            .api
            .set_favorite(request.key.clone(), request.favorite)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::SetFavoriteResponse {}))
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
    ) -> Result<Response<proto::CancelJobResponse>, Status> {
        self.0
            .api
            .cancel_job(request.get_ref().id.clone())
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::CancelJobResponse {}))
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
    ) -> Result<Response<proto::RemoveDownloadResponse>, Status> {
        self.0
            .api
            .remove_download(request.get_ref().key.clone())
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::RemoveDownloadResponse {}))
    }

    async fn set_config(
        &self,
        request: Request<proto::SetConfigRequest>,
    ) -> Result<Response<proto::ConfigView>, Status> {
        let config = request
            .get_ref()
            .config
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("SetConfig needs a config"))?;
        let view = self
            .0
            .api
            .set_config(convert::config_from_proto(config))
            .await
            .map_err(failed)?;
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
/// Bind the socket the frontend dials. A leftover file from a crashed
/// daemon has no listener behind it, so a failed connect is the signal that
/// it is stale -- clear it and take the path. The mode is the access
/// control: 0600 means only this user can open the channel.
pub fn bind_socket(path: &std::path::Path) -> std::io::Result<tokio::net::UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("a kopuzd is already serving {}", path.display()),
                ));
            }
            Err(_) => std::fs::remove_file(path)?,
        }
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(listener)
}

pub async fn serve(
    listener: tokio::net::UnixListener,
    state: Arc<GrpcState>,
) -> std::io::Result<()> {
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
        .add_service(KopuzServer::new(KopuzGrpc(state)))
        .serve_with_incoming(tokio_stream::wrappers::UnixListenerStream::new(listener))
        .await
        .map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::bind_socket;

    #[tokio::test]
    async fn the_socket_is_private_to_this_user() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("kopuzd.sock");
        let _listener = bind_socket(&path).expect("bind");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the socket mode is the access control");
    }

    #[tokio::test]
    async fn a_stale_socket_is_reclaimed_but_a_live_one_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("kopuzd.sock");

        // No listener behind it: a crashed daemon's leftover, take the path.
        std::fs::write(&path, b"").expect("leftover");
        let live = bind_socket(&path).expect("stale socket reclaimed");

        // Now one is really serving, so a second daemon must refuse.
        let error = bind_socket(&path).expect_err("live socket refused");
        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        drop(live);
    }
}
