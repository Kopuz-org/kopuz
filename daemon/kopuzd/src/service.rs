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

/// How many apps hold an event subscription right now; `kopuzctl` calls never subscribe, so they never count.
#[derive(Clone, Default)]
pub struct Clients(Arc<std::sync::atomic::AtomicUsize>);

impl Clients {
    pub fn count(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn join(&self) -> ClientGuard {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ClientGuard(self.clone())
    }
}

/// One subscriber, counted for as long as its stream is alive.
struct ClientGuard(Clients);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

pub struct GrpcState {
    pub api: Arc<dyn KopuzApi>,
    /// Entity-addressed artwork; `None` makes GetArtwork answer unsupported.
    pub artwork: Option<Arc<daemon::ArtworkService>>,
    pub session: SessionHandle,
    pub started: Instant,
    pub clients: Clients,
    /// `None` makes the update RPCs answer unsupported.
    pub updater: Option<Arc<crate::updates::Updater>>,
}

pub struct KopuzGrpc(Arc<GrpcState>);

fn no_updater() -> Status {
    failed(ApiError::unsupported("updates"))
}

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
    fn updater(&self) -> Option<&Arc<crate::updates::Updater>> {
        self.0.updater.as_ref()
    }

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
    _client: ClientGuard,
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
            _client: self.0.clients.join(),
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

    async fn set_muted(
        &self,
        request: Request<proto::SetMuted>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        self.player_mutation(api::PlayerCommand::SetMuted {
            muted: request.get_ref().muted,
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

    async fn set_version(
        &self,
        request: Request<proto::SetVersion>,
    ) -> Result<Response<proto::MutationResult>, Status> {
        let version = convert::track_version_from_proto(request.get_ref().version)
            .ok_or_else(|| Status::invalid_argument("pass the song or the video version"))?;
        self.player_mutation(api::PlayerCommand::SetVersion { version })
            .await
    }

    async fn get_status(
        &self,
        _request: Request<proto::GetStatusRequest>,
    ) -> Result<Response<proto::DaemonStatus>, Status> {
        Ok(Response::new(proto::DaemonStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: self.0.started.elapsed().as_secs(),
            proto_revision: proto::WIRE_REVISION,
        }))
    }

    async fn get_update_status(
        &self,
        _request: Request<proto::GetUpdateStatusRequest>,
    ) -> Result<Response<proto::UpdateStatus>, Status> {
        let status = self.updater().ok_or_else(no_updater)?.status();
        Ok(Response::new(convert::update_status_to_proto(&status)))
    }

    async fn check_for_update(
        &self,
        _request: Request<proto::CheckForUpdateRequest>,
    ) -> Result<Response<proto::UpdateStatus>, Status> {
        let status = self
            .updater()
            .ok_or_else(no_updater)?
            .check()
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::update_status_to_proto(&status)))
    }

    async fn install_update(
        &self,
        request: Request<proto::InstallUpdateRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.updater()
            .ok_or_else(no_updater)?
            .install(request.get_ref().app_pid)
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
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
        request: Request<proto::AlbumsRequest>,
    ) -> Result<Response<proto::AlbumPage>, Status> {
        let request = request.get_ref();
        let query = request
            .query
            .as_ref()
            .map(convert::album_query_from_proto)
            .unwrap_or_default();
        let page = convert::page_from_proto(request.page.as_ref());
        let albums = self.0.api.albums(query, page).await.map_err(failed)?;
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
        request: Request<proto::ArtistsRequest>,
    ) -> Result<Response<proto::ArtistPage>, Status> {
        let request = request.get_ref();
        let query = request
            .query
            .as_ref()
            .map(convert::artist_query_from_proto)
            .unwrap_or_default();
        let page = convert::page_from_proto(request.page.as_ref());
        let artists = self.0.api.artists(query, page).await.map_err(failed)?;
        Ok(Response::new(convert::artist_page_to_proto(&artists)))
    }

    async fn get_artist_tracks(
        &self,
        request: Request<proto::ArtistTracksRequest>,
    ) -> Result<Response<proto::TrackPage>, Status> {
        let request = request.get_ref();
        let page = convert::page_from_proto(request.page.as_ref());
        let artist = request.key.clone();
        let tracks = self
            .0
            .api
            .artist_tracks(artist, page)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::track_page_to_proto(&tracks)))
    }

    async fn get_artist(
        &self,
        request: Request<proto::ArtistRequest>,
    ) -> Result<Response<proto::ArtistDetail>, Status> {
        let artist = request.get_ref().key.clone();
        let detail = self.0.api.artist(artist).await.map_err(failed)?;
        Ok(Response::new(convert::artist_detail_to_proto(&detail)))
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
            .search(convert::search_request_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::search_results_to_proto(&results)))
    }

    async fn get_search_suggestions(
        &self,
        request: Request<proto::SearchSuggestionsRequest>,
    ) -> Result<Response<proto::SearchSuggestions>, Status> {
        let suggestions = self
            .0
            .api
            .search_suggestions(request.into_inner().query)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::SearchSuggestions {
            suggestions: suggestions
                .iter()
                .map(convert::search_suggestion_to_proto)
                .collect(),
        }))
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

    async fn get_for_you(
        &self,
        _request: Request<proto::GetForYouRequest>,
    ) -> Result<Response<proto::ForYou>, Status> {
        let mix = self.0.api.for_you().await.map_err(failed)?;
        Ok(Response::new(convert::for_you_to_proto(&mix)))
    }

    async fn refresh_for_you(
        &self,
        _request: Request<proto::RefreshForYouRequest>,
    ) -> Result<Response<proto::ForYou>, Status> {
        let mix = self.0.api.refresh_for_you().await.map_err(failed)?;
        Ok(Response::new(convert::for_you_to_proto(&mix)))
    }

    async fn get_discover_rows(
        &self,
        _request: Request<proto::GetDiscoverRowsRequest>,
    ) -> Result<Response<proto::DiscoverRows>, Status> {
        let rows = self.0.api.discover_rows().await.map_err(failed)?;
        Ok(Response::new(convert::discover_rows_to_proto(&rows)))
    }

    async fn refresh_discover_rows(
        &self,
        _request: Request<proto::RefreshDiscoverRowsRequest>,
    ) -> Result<Response<proto::DiscoverRows>, Status> {
        let rows = self.0.api.refresh_discover_rows().await.map_err(failed)?;
        Ok(Response::new(convert::discover_rows_to_proto(&rows)))
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
            .refresh_artist_artwork(convert::refresh_artists_from_proto(request.get_ref()))
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
        let privacy = match request.privacy {
            Some(value) => Some(
                convert::playlist_privacy_from_proto(value)
                    .ok_or_else(|| failed(ApiError::invalid_input("unknown playlist privacy")))?,
            ),
            None => None,
        };
        if request.description.is_none() && privacy.is_none() {
            self.0
                .api
                .rename_playlist(request.id, request.name)
                .await
                .map_err(failed)?;
            return Ok(Response::new(proto::Unit {}));
        }
        let edit = api::PlaylistEdit {
            name: (!request.name.is_empty()).then_some(request.name),
            description: request.description,
            privacy,
        };
        self.0
            .api
            .edit_playlist(request.id, edit)
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

    async fn dont_recommend(
        &self,
        request: Request<proto::DontRecommendRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .dont_recommend(request.get_ref().key.clone())
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn rate(
        &self,
        request: Request<proto::RateRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        let rating = convert::rating_from_proto(request.rating)
            .ok_or_else(|| failed(ApiError::invalid_input("a rating is required")))?;
        self.0
            .api
            .rate(request.item_ref, rating)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn follow(
        &self,
        request: Request<proto::FollowRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .follow(request.artist_ref, request.follow)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn save(
        &self,
        request: Request<proto::SaveRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        self.0
            .api
            .save(request.item_ref, request.saved)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn remove_from_history(
        &self,
        request: Request<proto::RemoveFromHistoryRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .remove_from_history(request.into_inner().token)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
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

    async fn rip_audio(
        &self,
        request: Request<proto::RipAudioRequest>,
    ) -> Result<Response<proto::JobRef>, Status> {
        let request = request.into_inner();
        let job = self
            .0
            .api
            .rip_audio(request.keys, request.output_dir)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::JobRef { job_id: job.job_id }))
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

    async fn search_downloads(
        &self,
        request: Request<proto::SearchDownloadsRequest>,
    ) -> Result<Response<proto::DownloadCandidates>, Status> {
        let candidates = self
            .0
            .api
            .search_downloads(request.into_inner().query)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::DownloadCandidates {
            candidates: candidates
                .iter()
                .map(convert::download_candidate_to_proto)
                .collect(),
        }))
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

    async fn get_offline_cache_status(
        &self,
        _: Request<proto::GetOfflineCacheStatusRequest>,
    ) -> Result<Response<proto::OfflineCacheStatus>, Status> {
        let status = self.0.api.offline_cache_status().await.map_err(failed)?;
        Ok(Response::new(convert::offline_cache_status_to_proto(
            &status,
        )))
    }

    async fn clear_offline_cache(
        &self,
        _: Request<proto::ClearOfflineCacheRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0.api.clear_offline_cache().await.map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    /// An edit this build cannot read is answered invalid in its place; the readable ones still apply.
    async fn edit_config(
        &self,
        request: Request<proto::EditConfigRequest>,
    ) -> Result<Response<proto::EditReport>, Status> {
        let decoded: Vec<Option<config::SettingsEdit>> = request
            .get_ref()
            .edits
            .iter()
            .map(convert::config_edit_from_proto)
            .collect();
        let report = self
            .0
            .api
            .edit_config(decoded.iter().flatten().cloned().collect())
            .await
            .map_err(failed)?;
        let mut applied = report.outcomes.into_iter();
        let outcomes = decoded
            .iter()
            .map(|edit| match edit {
                Some(_) => applied.next().unwrap_or(api::EditOutcome::Invalid {
                    reason: "the daemon answered fewer edits than it was sent".into(),
                }),
                None => api::EditOutcome::Invalid {
                    reason: "an edit this daemon cannot read".into(),
                },
            })
            .collect();
        Ok(Response::new(convert::edit_report_to_proto(
            &api::EditReport {
                outcomes,
                view: report.view,
            },
        )))
    }

    async fn get_frontend_prefs(
        &self,
        request: Request<proto::GetFrontendPrefsRequest>,
    ) -> Result<Response<proto::FrontendPrefsList>, Status> {
        let prefs = self
            .0
            .api
            .frontend_prefs(request.into_inner().frontend)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::FrontendPrefsList {
            prefs: prefs.iter().map(convert::frontend_pref_to_proto).collect(),
        }))
    }

    async fn set_frontend_prefs(
        &self,
        request: Request<proto::SetFrontendPrefsRequest>,
    ) -> Result<Response<proto::Unit>, Status> {
        let request = request.into_inner();
        let entries = request
            .entries
            .iter()
            .map(convert::pref_entry_from_proto)
            .collect();
        self.0
            .api
            .set_frontend_prefs(request.frontend, entries)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
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

    async fn preview_replay_gain(
        &self,
        request: Request<proto::ReplayGainSettings>,
    ) -> Result<Response<proto::Unit>, Status> {
        let replay_gain = convert::replay_gain_from_proto(Some(request.get_ref()));
        self.0
            .api
            .preview_replay_gain(replay_gain)
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

    async fn get_video(
        &self,
        request: Request<proto::VideoRequest>,
    ) -> Result<Response<proto::VideoChunk>, Status> {
        let request = request.into_inner();
        let chunk = self
            .0
            .api
            .video(api::VideoRequest {
                key: request.key,
                start: request.start,
                length: request.length,
            })
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::VideoChunk {
            content_type: chunk.content_type,
            start: chunk.start,
            total: chunk.total,
            data: chunk.bytes,
        }))
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

    async fn get_storage_quota(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::StorageQuota>, Status> {
        let quota = self
            .0
            .api
            .storage_quota(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::StorageQuota {
            used_bytes: quota.used_bytes,
            quota_bytes: quota.quota_bytes,
            remaining_bytes: quota.remaining_bytes,
        }))
    }

    async fn upload_track(
        &self,
        request: Request<tonic::Streaming<proto::TrackUploadChunk>>,
    ) -> Result<Response<proto::Unit>, Status> {
        let mut stream = request.into_inner();
        let header = tokio::time::timeout(std::time::Duration::from_secs(30), stream.message())
            .await
            .map_err(|_| Status::deadline_exceeded("upload timed out"))??
            .ok_or_else(|| Status::invalid_argument("missing upload metadata"))?;
        if header.total_bytes == 0
            || header.total_bytes > api::MAX_MUSIC_UPLOAD_BYTES as u64
            || header.source_id.is_empty()
            || header.filename.is_empty()
            || !header.content.is_empty()
        {
            return Err(Status::invalid_argument("invalid upload metadata"));
        }
        let source = self
            .0
            .api
            .sources()
            .await
            .map_err(failed)?
            .into_iter()
            .find(|source| source.id == header.source_id)
            .ok_or_else(|| Status::not_found("no such source"))?;
        if !source.capabilities.uploads {
            return Err(Status::unimplemented(
                "this source does not support music uploads",
            ));
        }
        let mut content = Vec::new();
        while let Some(chunk) =
            tokio::time::timeout(std::time::Duration::from_secs(30), stream.message())
                .await
                .map_err(|_| Status::deadline_exceeded("upload timed out"))??
        {
            if !chunk.source_id.is_empty()
                || !chunk.filename.is_empty()
                || chunk.total_bytes != 0
                || chunk.content.is_empty()
                || chunk.content.len() > 256 * 1024
                || content.len() as u64 + chunk.content.len() as u64 > header.total_bytes
            {
                return Err(Status::invalid_argument("invalid upload chunk"));
            }
            content.extend(chunk.content);
        }
        if content.len() as u64 != header.total_bytes {
            return Err(Status::invalid_argument("incomplete upload"));
        }
        self.0
            .api
            .upload_track(api::TrackUpload {
                source_id: header.source_id,
                filename: header.filename,
                content,
            })
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::Unit {}))
    }

    async fn check_source_draft(
        &self,
        request: Request<proto::SourceDraft>,
    ) -> Result<Response<proto::DraftCheck>, Status> {
        let check = self
            .0
            .api
            .check_source_draft(convert::source_draft_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::draft_check_to_proto(&check)))
    }

    async fn probe_source_draft(
        &self,
        request: Request<proto::SourceDraft>,
    ) -> Result<Response<proto::DraftProbe>, Status> {
        let probe = self
            .0
            .api
            .probe_source_draft(convert::source_draft_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::draft_probe_to_proto(&probe)))
    }

    async fn upsert_source(
        &self,
        request: Request<proto::SourceDraft>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let info = self
            .0
            .api
            .upsert_source(convert::source_draft_from_proto(request.get_ref()))
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
    }

    async fn delete_source(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::Unit>, Status> {
        self.0
            .api
            .delete_source(request.into_inner().id)
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

    async fn get_browser_sessions(
        &self,
        request: Request<proto::GetBrowserSessionsRequest>,
    ) -> Result<Response<proto::BrowserSessionList>, Status> {
        let sessions = self
            .0
            .api
            .browser_sessions(request.into_inner().service)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::BrowserSessionList {
            sessions: sessions
                .into_iter()
                .map(|session| proto::BrowserSession {
                    id: session.id,
                    browser: session.browser,
                    profile: session.profile,
                    account: session.account,
                })
                .collect(),
        }))
    }

    async fn import_browser_session(
        &self,
        request: Request<proto::ImportBrowserSessionRequest>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let request = request.into_inner();
        let info = self
            .0
            .api
            .import_browser_session(request.id, request.session)
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

    async fn get_network_status(
        &self,
        _request: Request<proto::GetNetworkStatusRequest>,
    ) -> Result<Response<proto::NetworkStatus>, Status> {
        let status = self.0.api.network_status().await.map_err(failed)?;
        Ok(Response::new(convert::network_status_to_proto(&status)))
    }

    async fn set_offline(
        &self,
        request: Request<proto::SetOfflineRequest>,
    ) -> Result<Response<proto::NetworkStatus>, Status> {
        let status = self
            .0
            .api
            .set_offline(request.get_ref().offline)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::network_status_to_proto(&status)))
    }

    async fn get_source_accounts(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::SourceAccountList>, Status> {
        let accounts = self
            .0
            .api
            .accounts(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::SourceAccountList {
            accounts: accounts
                .iter()
                .map(convert::source_account_to_proto)
                .collect(),
        }))
    }

    async fn test_source_connection(
        &self,
        request: Request<proto::SourceId>,
    ) -> Result<Response<proto::ConnectionReport>, Status> {
        let checks = self
            .0
            .api
            .test_source_connection(request.into_inner().id)
            .await
            .map_err(failed)?;
        Ok(Response::new(proto::ConnectionReport {
            checks: checks
                .iter()
                .map(convert::connection_check_to_proto)
                .collect(),
        }))
    }

    async fn switch_source_account(
        &self,
        request: Request<proto::SwitchSourceAccountRequest>,
    ) -> Result<Response<proto::SourceInfo>, Status> {
        let request = request.into_inner();
        let info = self
            .0
            .api
            .switch_account(request.id, request.account)
            .await
            .map_err(failed)?;
        Ok(Response::new(convert::source_info_to_proto(&info)))
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

    async fn get_browser_profiles(
        &self,
        _: Request<proto::GetBrowserProfilesRequest>,
    ) -> Result<Response<proto::BrowserProfileList>, Status> {
        let profiles = self.0.api.browser_profiles().await.map_err(failed)?;
        Ok(Response::new(proto::BrowserProfileList {
            profiles: profiles
                .iter()
                .map(convert::browser_profile_to_proto)
                .collect(),
        }))
    }

    async fn connect_integration_from_browser(
        &self,
        request: Request<proto::ConnectIntegrationFromBrowserRequest>,
    ) -> Result<Response<proto::IntegrationInfo>, Status> {
        let request = request.into_inner();
        let info = self
            .0
            .api
            .connect_integration_from_browser(request.id, request.profile)
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

/// Reflection (v1 and v1alpha) is registered so `grpcurl` works out of
/// the box, on every transport.
fn routes(state: Arc<GrpcState>) -> std::io::Result<tonic::transport::server::Router> {
    let reflection_v1 = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .map_err(std::io::Error::other)?;
    let reflection_v1alpha = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1alpha()
        .map_err(std::io::Error::other)?;
    Ok(tonic::transport::Server::builder()
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
    routes(state)?
        .serve_with_incoming(incoming)
        .await
        .map_err(std::io::Error::other)
}

/// Serve the daemon over plain HTTP/2 on `listener` until the future is
/// dropped.
pub async fn serve_tcp(
    listener: tokio::net::TcpListener,
    state: Arc<GrpcState>,
) -> std::io::Result<()> {
    routes(state)?
        .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
        .await
        .map_err(std::io::Error::other)
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

        drop(std::os::unix::net::UnixListener::bind(&path).expect("leftover"));
        let live = bind_socket(&path).expect("stale socket reclaimed");

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
