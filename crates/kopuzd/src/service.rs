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

use daemon::SessionHandle;

pub struct GrpcState {
    pub api: Arc<dyn KopuzApi>,
    /// Entity-addressed artwork; `None` makes GetArtwork answer unsupported.
    pub artwork: Option<Arc<daemon::ArtworkService>>,
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
    /// Sent before anything else, so a subscriber knows the stream is live.
    greeting: Option<proto::EventEnvelope>,
    live: broadcast::Receiver<ApiEvent>,
}

impl EventSubscription {
    async fn next(mut self) -> Option<(Result<proto::EventEnvelope, Status>, Self)> {
        if let Some(greeting) = self.greeting.take() {
            return Some((Ok(greeting), self));
        }
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
        let subscription = EventSubscription {
            greeting: Some(resync_message()),
            live,
        };
        let stream = futures_util::stream::unfold(subscription, |subscription| subscription.next());
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

    async fn get_queue_snapshot(
        &self,
        _: Request<proto::GetQueueSnapshotRequest>,
    ) -> Result<Response<proto::QueueSnapshot>, Status> {
        let snapshot = self.0.api.queue_snapshot().await.map_err(failed)?;
        Ok(Response::new(convert::queue_snapshot_to_proto(&snapshot)))
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

    async fn get_tracks_by_keys(
        &self,
        request: Request<proto::TracksByKeysRequest>,
    ) -> Result<Response<proto::TrackList>, Status> {
        let tracks = self
            .0
            .api
            .tracks_by_keys(request.into_inner().keys)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::TrackList {
            items: tracks.iter().map(convert::track_info_to_proto).collect(),
        }))
    }

    async fn get_albums(
        &self,
        request: Request<proto::Page>,
    ) -> Result<Response<proto::AlbumPage>, Status> {
        let page = convert::page_from_proto(Some(request.get_ref()));
        let albums = self.0.api.albums(page).await.map_err(failed)?;
        Ok(Response::new(convert::album_page_to_proto(&albums)))
    }

    async fn get_recently_added_albums(
        &self,
        request: Request<proto::Page>,
    ) -> Result<Response<proto::AlbumPage>, Status> {
        let page = convert::page_from_proto(Some(request.get_ref()));
        let albums = self
            .0
            .api
            .albums_recently_added(page)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::album_page_to_proto(&albums)))
    }

    async fn get_album(
        &self,
        request: Request<proto::AlbumRef>,
    ) -> Result<Response<proto::AlbumResponse>, Status> {
        let album = self
            .0
            .api
            .album(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::AlbumResponse {
            album: album.as_ref().map(convert::album_info_to_proto),
        }))
    }

    async fn get_album_tracks(
        &self,
        request: Request<proto::AlbumTracksRequest>,
    ) -> Result<Response<proto::TrackPage>, Status> {
        let request = request.get_ref();
        let page = convert::page_from_proto(request.page.as_ref());
        let tracks = self
            .0
            .api
            .album_tracks(request.id.clone(), page)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::track_page_to_proto(&tracks)))
    }

    async fn get_artists(
        &self,
        request: Request<proto::Page>,
    ) -> Result<Response<proto::ArtistPage>, Status> {
        let page = convert::page_from_proto(Some(request.get_ref()));
        let artists = self.0.api.artists(page).await.map_err(failed)?;
        Ok(Response::new(convert::artist_page_to_proto(&artists)))
    }

    async fn get_artist_tracks(
        &self,
        request: Request<proto::ArtistTracksRequest>,
    ) -> Result<Response<proto::TrackPage>, Status> {
        let request = request.get_ref();
        let page = convert::page_from_proto(request.page.as_ref());
        let tracks = self
            .0
            .api
            .artist_tracks(request.artist.clone(), page)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::track_page_to_proto(&tracks)))
    }

    async fn get_artist_sample_tracks(
        &self,
        request: Request<proto::Page>,
    ) -> Result<Response<proto::TrackPage>, Status> {
        let page = convert::page_from_proto(Some(request.get_ref()));
        let tracks = self
            .0
            .api
            .artist_sample_tracks(page)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::track_page_to_proto(&tracks)))
    }

    async fn get_genres(
        &self,
        _request: Request<proto::GetGenresRequest>,
    ) -> Result<Response<proto::GenreList>, Status> {
        let genres = self.0.api.genres().await.map_err(failed)?;
        Ok(Response::new(proto::GenreList { genres }))
    }

    async fn get_top_genre(
        &self,
        _request: Request<proto::GetTopGenreRequest>,
    ) -> Result<Response<proto::TopGenreResponse>, Status> {
        let genre = self.0.api.top_genre().await.map_err(failed)?;
        Ok(Response::new(proto::TopGenreResponse { genre }))
    }

    async fn get_genre_tracks(
        &self,
        request: Request<proto::GenreTracksRequest>,
    ) -> Result<Response<proto::TrackPage>, Status> {
        let request = request.get_ref();
        let page = convert::page_from_proto(request.page.as_ref());
        let tracks = self
            .0
            .api
            .genre_tracks(request.genre.clone(), page)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::track_page_to_proto(&tracks)))
    }

    async fn get_recent_tracks(
        &self,
        request: Request<proto::Page>,
    ) -> Result<Response<proto::TrackPage>, Status> {
        let page = convert::page_from_proto(Some(request.get_ref()));
        let tracks = self.0.api.recent_tracks(page).await.map_err(failed)?;
        Ok(Response::new(convert::track_page_to_proto(&tracks)))
    }

    async fn search(
        &self,
        request: Request<proto::SearchRequest>,
    ) -> Result<Response<proto::SearchResults>, Status> {
        let results = self
            .0
            .api
            .search(request.into_inner().query)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::search_results_to_proto(&results)))
    }

    async fn get_track_web_url(
        &self,
        request: Request<proto::TrackWebUrlRequest>,
    ) -> Result<Response<proto::WebUrl>, Status> {
        let url = self
            .0
            .api
            .track_web_url(request.into_inner().key)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::WebUrl { url }))
    }

    async fn get_album_web_url(
        &self,
        request: Request<proto::AlbumWebUrlRequest>,
    ) -> Result<Response<proto::WebUrl>, Status> {
        let url = self
            .0
            .api
            .album_web_url(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::WebUrl { url }))
    }

    async fn get_catalog(
        &self,
        request: Request<proto::CatalogRequest>,
    ) -> Result<Response<proto::CatalogPage>, Status> {
        let page = self
            .0
            .api
            .catalog(request.into_inner().continuation)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::catalog_page_to_proto(&page)))
    }

    async fn get_catalog_detail(
        &self,
        request: Request<proto::CatalogDetailRequest>,
    ) -> Result<Response<proto::CatalogDetail>, Status> {
        let detail = self
            .0
            .api
            .catalog_detail(convert::catalog_detail_request_from_proto(
                request.get_ref(),
            ))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::catalog_detail_to_proto(&detail)))
    }

    async fn get_radio_stations(
        &self,
        _: Request<proto::GetRadioStationsRequest>,
    ) -> Result<Response<proto::RadioStationList>, Status> {
        let stations = self.0.api.radio_stations().await.map_err(failed)?;
        Ok(Response::new(proto::RadioStationList {
            stations: stations
                .iter()
                .map(convert::radio_station_to_proto)
                .collect(),
        }))
    }

    async fn search_radio(
        &self,
        request: Request<proto::SearchRadioRequest>,
    ) -> Result<Response<proto::RadioStationList>, Status> {
        let request = request.into_inner();
        let stations = self
            .0
            .api
            .search_radio(request.query, request.limit)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::RadioStationList {
            stations: stations
                .iter()
                .map(convert::radio_station_to_proto)
                .collect(),
        }))
    }

    async fn pin_radio_station(
        &self,
        request: Request<proto::PinRadioStationRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .pin_radio_station(request.id, request.pinned)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn validate_radio_registry(
        &self,
        request: Request<proto::ValidateRadioRegistryRequest>,
    ) -> Result<Response<proto::RadioRegistryInfo>, Status> {
        let stations = self
            .0
            .api
            .validate_radio_registry(request.into_inner().url)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::RadioRegistryInfo { stations }))
    }

    async fn update_track_metadata(
        &self,
        request: Request<proto::TrackMetadataPatch>,
    ) -> Result<Response<proto::TrackInfo>, Status> {
        let track = self
            .0
            .api
            .update_track_metadata(convert::track_patch_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::track_info_to_proto(&track)))
    }

    async fn delete_tracks(
        &self,
        request: Request<proto::DeleteTracksRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .delete_tracks(request.keys, request.from_disk)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn delete_album(
        &self,
        request: Request<proto::DeleteAlbumRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .delete_album(request.id, request.from_disk)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn upload_artwork(
        &self,
        request: Request<proto::ArtworkUpload>,
    ) -> Result<Response<proto::Unit>, Status> {
        let upload = convert::artwork_upload_from_proto(request.get_ref())
            .ok_or_else(|| Status::invalid_argument("artwork target is required"))?;
        self.0.api.upload_artwork(upload).await.map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn remove_artwork(
        &self,
        request: Request<proto::ArtworkTarget>,
    ) -> Result<Response<proto::Unit>, Status> {
        let target = convert::artwork_target_from_proto(request.get_ref())
            .ok_or_else(|| Status::invalid_argument("artwork target is required"))?;
        self.0.api.remove_artwork(target).await.map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn refresh_artist_artwork(
        &self,
        request: Request<proto::RefreshArtistArtworkRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .refresh_artist_artwork(request.into_inner().names)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn get_playlists(
        &self,
        _request: Request<proto::GetPlaylistsRequest>,
    ) -> Result<Response<proto::PlaylistCatalog>, Status> {
        let catalog = self.0.api.playlists().await.map_err(failed)?;
        Ok(Response::new(convert::playlist_catalog_to_proto(&catalog)))
    }

    async fn create_playlist(
        &self,
        request: Request<proto::CreatePlaylistRequest>,
    ) -> Result<Response<proto::PlaylistId>, Status> {
        let request = request.into_inner();
        let id = self
            .0
            .api
            .create_playlist(request.name, request.keys)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::PlaylistId { id }))
    }

    async fn rename_playlist(
        &self,
        request: Request<proto::RenamePlaylistRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .rename_playlist(request.id, request.name)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn delete_playlist(
        &self,
        request: Request<proto::PlaylistId>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .delete_playlist(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn add_playlist_tracks(
        &self,
        request: Request<proto::AddPlaylistTracksRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .add_playlist_tracks(request.id, request.keys)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn remove_playlist_track(
        &self,
        request: Request<proto::RemovePlaylistTrackRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .remove_playlist_track(request.id, request.index)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn reorder_playlist(
        &self,
        request: Request<proto::ReorderPlaylistRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .reorder_playlist(
                request.id,
                api::PlaylistReorder {
                    from: request.from,
                    to: request.to,
                },
            )
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn refresh_playlist(
        &self,
        request: Request<proto::PlaylistId>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .refresh_playlist(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn create_playlist_folder(
        &self,
        request: Request<proto::CreatePlaylistFolderRequest>,
    ) -> Result<Response<proto::PlaylistFolderId>, Status> {
        let id = self
            .0
            .api
            .create_playlist_folder(request.into_inner().name)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::PlaylistFolderId { id }))
    }

    async fn rename_playlist_folder(
        &self,
        request: Request<proto::RenamePlaylistFolderRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .rename_playlist_folder(request.id, request.name)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn delete_playlist_folder(
        &self,
        request: Request<proto::PlaylistFolderId>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .delete_playlist_folder(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn move_playlist(
        &self,
        request: Request<proto::MovePlaylistRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .move_playlist(request.playlist_id, request.folder_id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
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

    async fn get_external_devices(
        &self,
        request: Request<proto::ExternalDevicesRequest>,
    ) -> Result<Response<proto::ExternalDeviceList>, Status> {
        let devices = self
            .0
            .api
            .external_devices(request.into_inner().source_id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::ExternalDeviceList {
            devices: devices
                .iter()
                .map(convert::external_device_to_proto)
                .collect(),
        }))
    }

    async fn select_external_device(
        &self,
        request: Request<proto::SelectExternalDeviceRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .select_external_device(request.source_id, request.device_id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
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

    async fn download_url(
        &self,
        request: Request<proto::DownloadUrlRequest>,
    ) -> Result<Response<proto::JobRef>, Status> {
        let request = request.into_inner();
        let job = self
            .0
            .api
            .download_url(request.url, request.format)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::JobRef { job_id: job.job_id }))
    }

    async fn get_download_formats(
        &self,
        _: Request<proto::GetDownloadFormatsRequest>,
    ) -> Result<Response<proto::DownloadFormats>, Status> {
        let formats = self.0.api.download_formats().await.map_err(failed)?;
        Ok(Response::new(proto::DownloadFormats {
            formats: formats
                .iter()
                .map(convert::choice_option_to_proto)
                .collect(),
        }))
    }

    async fn get_artwork_settings(
        &self,
        _: Request<proto::GetArtworkSettingsRequest>,
    ) -> Result<Response<proto::ArtworkSettings>, Status> {
        let fields = self.0.api.artwork_settings().await.map_err(failed)?;
        Ok(Response::new(proto::ArtworkSettings {
            fields: fields.iter().map(convert::field_spec_to_proto).collect(),
        }))
    }

    async fn set_artwork_settings(
        &self,
        request: Request<proto::SetArtworkSettingsRequest>,
    ) -> Result<Response<proto::ArtworkSettings>, Status> {
        let values = request
            .into_inner()
            .values
            .iter()
            .map(convert::field_value_from_proto)
            .collect();
        let fields = self
            .0
            .api
            .set_artwork_settings(values)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::ArtworkSettings {
            fields: fields.iter().map(convert::field_spec_to_proto).collect(),
        }))
    }

    async fn get_downloader_settings(
        &self,
        _: Request<proto::GetDownloaderSettingsRequest>,
    ) -> Result<Response<proto::DownloaderSettings>, Status> {
        let fields = self.0.api.downloader_settings().await.map_err(failed)?;
        Ok(Response::new(proto::DownloaderSettings {
            fields: fields.iter().map(convert::field_spec_to_proto).collect(),
        }))
    }

    async fn set_downloader_settings(
        &self,
        request: Request<proto::SetDownloaderSettingsRequest>,
    ) -> Result<Response<proto::DownloaderSettings>, Status> {
        let values = request
            .into_inner()
            .values
            .iter()
            .map(convert::field_value_from_proto)
            .collect();
        let fields = self
            .0
            .api
            .set_downloader_settings(values)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::DownloaderSettings {
            fields: fields.iter().map(convert::field_spec_to_proto).collect(),
        }))
    }

    async fn get_downloader_history(
        &self,
        _: Request<proto::GetDownloaderHistoryRequest>,
    ) -> Result<Response<proto::DownloadHistory>, Status> {
        let entries = self.0.api.downloader_history().await.map_err(failed)?;
        Ok(Response::new(proto::DownloadHistory {
            entries: entries
                .iter()
                .map(convert::download_history_entry_to_proto)
                .collect(),
        }))
    }

    async fn clear_downloader_history(
        &self,
        _: Request<proto::ClearDownloaderHistoryRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .clear_downloader_history()
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn get_download_statuses(
        &self,
        _: Request<proto::GetDownloadStatusesRequest>,
    ) -> Result<Response<proto::DownloadStatusList>, Status> {
        let items = self.0.api.download_statuses().await.map_err(failed)?;
        Ok(Response::new(proto::DownloadStatusList {
            items: items
                .iter()
                .map(convert::download_status_to_proto)
                .collect(),
        }))
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

    async fn preview_equalizer(
        &self,
        request: Request<proto::EqualizerSettings>,
    ) -> Result<Response<proto::Unit>, Status> {
        let equalizer = convert::equalizer_from_proto(Some(request.get_ref()));
        self.0
            .api
            .preview_equalizer(equalizer)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    #[allow(clippy::result_large_err)]
    async fn get_artwork(
        &self,
        request: Request<proto::ArtworkRequest>,
    ) -> Result<Response<Self::GetArtworkStream>, Status> {
        let Some(service) = &self.0.artwork else {
            return Err(failed(ApiError::unsupported(
                "this daemon runs without artwork",
            )));
        };
        let request = request.get_ref();
        let request = convert::artwork_request_from_proto(request).ok_or_else(|| {
            Status::invalid_argument("pass one of track, album, artist, playlist, catalog, station")
        })?;
        let payload = service
            .fetch(&request.target, request.hq)
            .await
            .map_err(failed)?;
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

    async fn get_sources(
        &self,
        _: Request<proto::GetSourcesRequest>,
    ) -> Result<Response<proto::SourceList>, Status> {
        let sources = self.0.api.sources().await.map_err(failed)?;
        Ok(Response::new(proto::SourceList {
            sources: sources.iter().map(convert::source_info_to_proto).collect(),
        }))
    }

    async fn get_services(
        &self,
        _: Request<proto::GetServicesRequest>,
    ) -> Result<Response<proto::ServiceList>, Status> {
        let services = self.0.api.services().await.map_err(failed)?;
        Ok(Response::new(proto::ServiceList {
            services: services
                .iter()
                .map(convert::service_info_to_proto)
                .collect(),
        }))
    }

    async fn select_source(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let info = self
            .0
            .api
            .switch_source(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn upsert_local_source(
        &self,
        request: Request<proto::LocalSourceDraft>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let info = self
            .0
            .api
            .upsert_local_source(convert::local_draft_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn delete_local_source(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .delete_local_source(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn set_source_directories(
        &self,
        request: Request<proto::SetSourceDirectoriesRequest>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let request = request.into_inner();
        let info = self
            .0
            .api
            .set_source_directories(request.id, request.directories)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn set_source_settings(
        &self,
        request: Request<proto::SetSourceSettingsRequest>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let request = request.into_inner();
        let values = request
            .values
            .iter()
            .map(convert::field_value_from_proto)
            .collect();
        let info = self
            .0
            .api
            .set_source_settings(request.id, values)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn check_server_draft(
        &self,
        request: Request<proto::ServerDraft>,
    ) -> Result<Response<proto::DraftCheck>, Status> {
        let check = self
            .0
            .api
            .check_server_draft(convert::server_draft_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::draft_check_to_proto(&check)))
    }

    async fn upsert_server(
        &self,
        request: Request<proto::ServerDraft>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let info = self
            .0
            .api
            .upsert_server(convert::server_draft_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn delete_server(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .delete_server(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn provision_credentials(
        &self,
        request: Request<proto::CredentialProvision>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let info = self
            .0
            .api
            .provision_credentials(convert::credential_provision_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn login_source(
        &self,
        request: Request<proto::SourceLoginRequest>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let request = request.into_inner();
        let info = self
            .0
            .api
            .login_source(api::SourceLoginRequest {
                server_id: request.server_id,
                username: request.username,
                password: request.password,
            })
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn clear_credentials(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .clear_credentials(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn authenticate_source(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let info = self
            .0
            .api
            .authenticate_source(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn browse_source(
        &self,
        request: Request<proto::BrowseSourceRequest>,
    ) -> Result<Response<proto::SourceFolderList>, Status> {
        let request = request.into_inner();
        let entries = self
            .0
            .api
            .browse_source(request.id, request.path)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::SourceFolderList {
            entries: entries
                .into_iter()
                .map(|entry| proto::SourceFolderEntry {
                    path: entry.path,
                    name: entry.name,
                })
                .collect(),
        }))
    }

    async fn validate_source(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::SourceStateResponse>, Status> {
        let state = self
            .0
            .api
            .validate_source(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::SourceStateResponse {
            state: convert::source_state_to_proto(state) as i32,
        }))
    }

    async fn can_open_browser(
        &self,
        _request: Request<proto::CanOpenBrowserRequest>,
    ) -> Result<Response<proto::BrowserAccess>, Status> {
        let available = self.0.api.can_open_browser().await.map_err(failed)?;
        Ok(Response::new(proto::BrowserAccess { available }))
    }

    async fn get_integrations(
        &self,
        _: Request<proto::GetIntegrationsRequest>,
    ) -> Result<Response<proto::IntegrationList>, Status> {
        let integrations = self.0.api.integrations().await.map_err(failed)?;
        Ok(Response::new(proto::IntegrationList {
            integrations: integrations
                .iter()
                .map(convert::integration_info_to_proto)
                .collect(),
        }))
    }

    async fn set_integration_settings(
        &self,
        request: Request<proto::SetIntegrationSettingsRequest>,
    ) -> Result<Response<proto::IntegrationInfo>, Status> {
        let request = request.into_inner();
        let values = request
            .values
            .iter()
            .map(convert::field_value_from_proto)
            .collect();
        let info = self
            .0
            .api
            .set_integration_settings(request.id, values)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::integration_info_to_proto(&info)))
    }

    async fn clear_integration(
        &self,
        request: Request<proto::IntegrationId>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .clear_integration(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn authenticate_integration(
        &self,
        request: Request<proto::IntegrationId>,
    ) -> Result<Response<proto::IntegrationInfo>, Status> {
        let info = self
            .0
            .api
            .authenticate_integration(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::integration_info_to_proto(&info)))
    }
}

/// The channel the daemon serves on: a Unix socket, or on Windows a named
/// pipe. Dropping it releases the address, so only the process that bound
/// a path ever removes it.
#[cfg(unix)]
#[derive(Debug)]
pub struct Listener {
    incoming: tokio_stream::wrappers::UnixListenerStream,
    path: std::path::PathBuf,
}

#[cfg(unix)]
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
impl Stream for Listener {
    type Item = std::io::Result<tokio::net::UnixStream>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.incoming).poll_next(cx)
    }
}

#[cfg(windows)]
pub type Listener = proto::pipe::Listener;

/// Bind the socket the frontend dials. A leftover socket from a crashed
/// daemon has no listener behind it, so a refused connect is the signal
/// that it is stale -- clear it and take the path. Anything at the path
/// that is not a socket is somebody else's file and stays. The mode is the
/// access control: 0600 means only this user can open the channel.
#[cfg(unix)]
pub fn bind_socket(path: &std::path::Path) -> std::io::Result<Listener> {
    use std::io::{Error, ErrorKind};
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};

    // sockaddr_un.sun_path is a fixed 108-byte field, and the kernel's error
    // for overrunning it names a constant nobody recognises.
    const SUN_PATH_MAX: usize = 100;
    if path.as_os_str().len() > SUN_PATH_MAX {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "socket path is {} bytes; a unix socket cannot exceed {SUN_PATH_MAX}: {}",
                path.as_os_str().len(),
                path.display()
            ),
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            match std::os::unix::net::UnixStream::connect(path) {
                Ok(_) => {
                    return Err(Error::new(
                        ErrorKind::AddrInUse,
                        format!("a kopuzd is already serving {}", path.display()),
                    ));
                }
                Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(path)?;
                }
                Err(error) => {
                    return Err(Error::new(
                        error.kind(),
                        format!("cannot tell whether {} is served: {error}", path.display()),
                    ));
                }
            }
        }
        Ok(_) => {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    let guard = Listener {
        incoming: tokio_stream::wrappers::UnixListenerStream::new(listener),
        path: path.to_path_buf(),
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(guard)
}

/// Bind the pipe the frontend dials; see `proto::pipe` for the access
/// control it carries.
#[cfg(windows)]
pub fn bind_socket(path: &std::path::Path) -> std::io::Result<Listener> {
    Listener::bind(path)
}

/// The bearer token a TCP client has to present. The socket and the pipe
/// let the OS decide who may connect; a port has no owner, so on that
/// transport the token is the boundary.
#[derive(Clone)]
pub struct Token(Arc<str>);

impl Token {
    pub fn new(secret: impl Into<Arc<str>>) -> Self {
        Self(secret.into())
    }

    pub fn secret(&self) -> &str {
        &self.0
    }

    /// 256 bits from the OS, hex-encoded so it survives a shell.
    pub fn generate() -> Self {
        let bytes: [u8; 32] = rand::random();
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Self(hex.into())
    }

    /// The token at `path`, minted there if the file does not exist yet.
    /// The file is created private to this user, since it is the secret.
    pub fn load_or_create(path: &std::path::Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.trim().is_empty() => return Ok(Self::new(text.trim())),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let token = Self::generate();
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options.open(path)?.write_all(token.secret().as_bytes())?;
        Ok(token)
    }

    /// `result_large_err` is tonic's own Status type; nothing to shrink here.
    #[allow(clippy::result_large_err)]
    fn check(&self, request: Request<()>) -> Result<Request<()>, Status> {
        let presented = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        match presented {
            Some(presented) if same_secret(presented, &self.0) => Ok(request),
            _ => Err(Status::unauthenticated(
                "this transport requires the daemon token as `authorization: Bearer <token>`",
            )),
        }
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

/// Compare without a length-dependent early exit, so timing does not leak
/// how much of a guess was right.
fn same_secret(presented: &str, expected: &str) -> bool {
    let (a, b) = (presented.as_bytes(), expected.as_bytes());
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter().chain(std::iter::repeat(&0))) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

/// Reflection (v1 and v1alpha) is registered so `grpcurl` works out of
/// the box, on every transport.
fn routes<L: Clone>(
    mut builder: tonic::transport::Server<L>,
    state: Arc<GrpcState>,
) -> std::io::Result<tonic::transport::server::Router<L>> {
    let reflection_v1 = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .map_err(std::io::Error::other)?;
    let reflection_v1alpha = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1alpha()
        .map_err(std::io::Error::other)?;
    Ok(builder
        .add_service(reflection_v1)
        .add_service(reflection_v1alpha)
        .add_service(KopuzServer::new(KopuzGrpc(state))))
}

/// Serve the daemon on `listener` until the future is dropped.
pub async fn serve(listener: Listener, state: Arc<GrpcState>) -> std::io::Result<()> {
    #[cfg(unix)]
    let incoming = listener;
    #[cfg(windows)]
    let incoming = listener.incoming();
    routes(tonic::transport::Server::builder(), state)?
        .serve_with_incoming(incoming)
        .await
        .map_err(std::io::Error::other)
}

/// Serve the daemon over plain HTTP/2 on `listener`, every call gated on
/// `token`, until the future is dropped.
#[allow(clippy::result_large_err)]
pub async fn serve_tcp(
    listener: tokio::net::TcpListener,
    token: Token,
    state: Arc<GrpcState>,
) -> std::io::Result<()> {
    let gate = tonic::service::interceptor(move |request| token.check(request));
    routes(tonic::transport::Server::builder().layer(gate), state)?
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
        .await
        .map_err(std::io::Error::other)
}

#[cfg(test)]
mod token_tests {
    use super::Token;

    #[test]
    fn a_token_is_minted_once_and_read_back_thereafter() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("kopuzd.token");
        let first = Token::load_or_create(&path).expect("mint");
        assert_eq!(first.secret().len(), 64, "256 bits of hex");
        let again = Token::load_or_create(&path).expect("read");
        assert_eq!(first.secret(), again.secret());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the token file is the secret");
        }
    }

    #[test]
    #[allow(clippy::result_large_err)]
    fn only_the_exact_bearer_token_passes() {
        let token = Token::new("s3cret");
        let with = |value: Option<&str>| {
            let mut request = tonic::Request::new(());
            if let Some(value) = value {
                request
                    .metadata_mut()
                    .insert("authorization", value.parse().expect("ascii"));
            }
            token.check(request).map(|_| ())
        };
        assert!(with(Some("Bearer s3cret")).is_ok());
        for wrong in [
            None,
            Some("Bearer s3cre"),
            Some("Bearer s3cret1"),
            Some("s3cret"),
        ] {
            let status = with(wrong).expect_err("refused");
            assert_eq!(status.code(), tonic::Code::Unauthenticated);
        }
    }
}

#[cfg(all(test, unix))]
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
    async fn an_overlong_socket_path_is_named_not_just_refused() {
        let path = std::path::PathBuf::from(format!("/tmp/{}/kopuzd.sock", "x".repeat(120)));
        let error = bind_socket(&path).expect_err("refused");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            error.to_string().contains("unix socket cannot exceed"),
            "the message has to say what is wrong: {error}"
        );
    }

    #[tokio::test]
    async fn a_stale_socket_is_reclaimed_but_a_live_one_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("kopuzd.sock");

        // A crashed daemon leaves the socket file with nobody behind it.
        drop(std::os::unix::net::UnixListener::bind(&path).expect("leftover"));
        let live = bind_socket(&path).expect("stale socket reclaimed");

        // Now one is really serving, so a second daemon must refuse.
        let error = bind_socket(&path).expect_err("live socket refused");
        assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
        drop(live);
    }

    #[tokio::test]
    async fn releasing_the_listener_unlinks_the_socket_it_bound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("kopuzd.sock");
        let listener = bind_socket(&path).expect("bind");
        assert!(path.exists());
        drop(listener);
        assert!(
            !path.exists(),
            "the socket is unlinked by the process that bound it"
        );
    }

    #[tokio::test]
    async fn an_ordinary_file_at_the_path_is_not_deleted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("kopuzd.sock");
        std::fs::write(&path, b"not a socket").expect("file");
        let error = bind_socket(&path).expect_err("refused");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            b"not a socket",
            "a failed connect is not licence to unlink a file"
        );
    }
}
