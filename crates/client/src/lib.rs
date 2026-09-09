//! `GrpcApi`: the wire twin of the daemon's in-process `LocalApi`.
//!
//! Implements [`api::KopuzApi`] over the daemon's gRPC surface, so a Rust
//! frontend can swap between embedding the daemon and attaching to one over
//! the socket without touching its data layer. The contract tests in the
//! daemon crate run the same assertions through both implementations.
//!
//! The transport is a Unix domain socket in the user's runtime dir. There
//! are no credentials: the socket's file mode is the access control, so the
//! kernel decides who may connect. The path is stable across daemon
//! restarts, so `events()` reattaches to it and reports the gap as
//! [`api::ApiEvent::Resync`].
//!
//! Playback mutations use typed unary RPCs. `events()` owns a reattaching
//! server-streaming subscription.

use std::path::{Path, PathBuf};

use api::{
    ApiError, CommandAck, ConfigView, FavoritesView, JobKind, JobRef, JobStatus, Page,
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
impl api::PlayerApi for GrpcApi {
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

    async fn queue_snapshot(&self) -> Result<api::QueueSnapshot, ApiError> {
        let snapshot = self
            .client()
            .get_queue_snapshot(Request::new(proto::GetQueueSnapshotRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::queue_snapshot_from_proto(snapshot.get_ref()))
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
}

#[async_trait::async_trait]
impl api::LibraryApi for GrpcApi {
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

    async fn tracks_by_keys(&self, keys: Vec<String>) -> Result<Vec<api::TrackInfo>, ApiError> {
        let tracks = self
            .client()
            .get_tracks_by_keys(Request::new(proto::TracksByKeysRequest { keys }))
            .await
            .map_err(wire_error)?;
        Ok(tracks
            .get_ref()
            .items
            .iter()
            .map(convert::track_info_from_proto)
            .collect())
    }

    async fn albums(&self, page: Page) -> Result<api::AlbumPage, ApiError> {
        let albums = self
            .client()
            .get_albums(Request::new(convert::page_to_proto(page)))
            .await
            .map_err(wire_error)?;
        Ok(convert::album_page_from_proto(albums.get_ref()))
    }

    async fn album(&self, id: String) -> Result<Option<api::AlbumInfo>, ApiError> {
        let album = self
            .client()
            .get_album(Request::new(proto::AlbumRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(album
            .get_ref()
            .album
            .as_ref()
            .map(convert::album_info_from_proto))
    }

    async fn album_tracks(&self, id: String, page: Page) -> Result<api::TrackPage, ApiError> {
        let tracks = self
            .client()
            .get_album_tracks(Request::new(proto::AlbumTracksRequest {
                id,
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn artists(&self, page: Page) -> Result<api::ArtistPage, ApiError> {
        let artists = self
            .client()
            .get_artists(Request::new(convert::page_to_proto(page)))
            .await
            .map_err(wire_error)?;
        Ok(convert::artist_page_from_proto(artists.get_ref()))
    }

    async fn artist_tracks(&self, artist: String, page: Page) -> Result<api::TrackPage, ApiError> {
        let tracks = self
            .client()
            .get_artist_tracks(Request::new(proto::ArtistTracksRequest {
                artist,
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn artist_sample_tracks(&self, page: Page) -> Result<api::TrackPage, ApiError> {
        let tracks = self
            .client()
            .get_artist_sample_tracks(Request::new(convert::page_to_proto(page)))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn genres(&self) -> Result<Vec<String>, ApiError> {
        let genres = self
            .client()
            .get_genres(Request::new(proto::GetGenresRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(genres.into_inner().genres)
    }

    async fn top_genre(&self) -> Result<Option<String>, ApiError> {
        let genre = self
            .client()
            .get_top_genre(Request::new(proto::GetTopGenreRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(genre.into_inner().genre)
    }

    async fn genre_tracks(&self, genre: String, page: Page) -> Result<api::TrackPage, ApiError> {
        let tracks = self
            .client()
            .get_genre_tracks(Request::new(proto::GenreTracksRequest {
                genre,
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn recent_tracks(&self, page: Page) -> Result<api::TrackPage, ApiError> {
        let tracks = self
            .client()
            .get_recent_tracks(Request::new(convert::page_to_proto(page)))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn search(&self, query: String) -> Result<api::SearchResults, ApiError> {
        let results = self
            .client()
            .search(Request::new(proto::SearchRequest { query }))
            .await
            .map_err(wire_error)?;
        Ok(convert::search_results_from_proto(results.get_ref()))
    }

    async fn catalog(&self, continuation: Option<String>) -> Result<api::CatalogPage, ApiError> {
        let page = self
            .client()
            .get_catalog(Request::new(proto::CatalogRequest { continuation }))
            .await
            .map_err(wire_error)?;
        Ok(convert::catalog_page_from_proto(page.get_ref()))
    }

    async fn catalog_detail(
        &self,
        request: api::CatalogDetailRequest,
    ) -> Result<api::CatalogDetail, ApiError> {
        let detail = self
            .client()
            .get_catalog_detail(Request::new(convert::catalog_detail_request_to_proto(
                &request,
            )))
            .await
            .map_err(wire_error)?;
        Ok(convert::catalog_detail_from_proto(detail.get_ref()))
    }

    async fn radio_stations(&self) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        let list = self
            .client()
            .get_radio_stations(Request::new(proto::GetRadioStationsRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(list
            .get_ref()
            .stations
            .iter()
            .map(convert::radio_station_from_proto)
            .collect())
    }

    async fn search_radio(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        let list = self
            .client()
            .search_radio(Request::new(proto::SearchRadioRequest { query, limit }))
            .await
            .map_err(wire_error)?;
        Ok(list
            .get_ref()
            .stations
            .iter()
            .map(convert::radio_station_from_proto)
            .collect())
    }

    async fn pin_radio_station(&self, id: String, pinned: bool) -> Result<(), ApiError> {
        self.client()
            .pin_radio_station(Request::new(proto::PinRadioStationRequest { id, pinned }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn update_track_metadata(
        &self,
        patch: api::TrackMetadataPatch,
    ) -> Result<api::TrackInfo, ApiError> {
        let track = self
            .client()
            .update_track_metadata(Request::new(convert::track_patch_to_proto(&patch)))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_info_from_proto(track.get_ref()))
    }

    async fn delete_tracks(&self, keys: Vec<String>, from_disk: bool) -> Result<(), ApiError> {
        self.client()
            .delete_tracks(Request::new(proto::DeleteTracksRequest { keys, from_disk }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn delete_album(&self, id: String, from_disk: bool) -> Result<(), ApiError> {
        self.client()
            .delete_album(Request::new(proto::DeleteAlbumRequest { id, from_disk }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn upload_artwork(&self, upload: api::ArtworkUpload) -> Result<(), ApiError> {
        self.client()
            .upload_artwork(Request::new(convert::artwork_upload_to_proto(&upload)))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn remove_artwork(&self, target: api::ArtworkTarget) -> Result<(), ApiError> {
        self.client()
            .remove_artwork(Request::new(convert::artwork_target_to_proto(&target)))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn refresh_artist_artwork(&self, names: Vec<String>) -> Result<(), ApiError> {
        self.client()
            .refresh_artist_artwork(Request::new(proto::RefreshArtistArtworkRequest { names }))
            .await
            .map_err(wire_error)?;
        Ok(())
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
}

#[async_trait::async_trait]
impl api::ArtworkApi for GrpcApi {
    async fn artwork(&self, request: api::ArtworkRequest) -> Result<api::ArtworkData, ApiError> {
        let mut stream = self
            .client()
            .get_artwork(Request::new(convert::artwork_request_to_proto(&request)))
            .await
            .map_err(wire_error)?
            .into_inner();
        let mut data = api::ArtworkData::default();
        // The content type rides the first chunk only; the rest is body.
        while let Some(chunk) = stream.message().await.map_err(wire_error)? {
            if !chunk.content_type.is_empty() {
                data.content_type = chunk.content_type;
            }
            data.bytes.extend_from_slice(&chunk.data);
        }
        Ok(data)
    }
}

#[async_trait::async_trait]
impl api::ConfigApi for GrpcApi {
    async fn config(&self) -> Result<ConfigView, ApiError> {
        let view = self
            .client()
            .get_config(Request::new(proto::GetConfigRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::config_view_from_proto(view.get_ref()))
    }

    async fn set_config(&self, config: config::AppConfig) -> Result<ConfigView, ApiError> {
        let view = self
            .client()
            .set_config(Request::new(proto::SetConfigRequest {
                config: Some(convert::config_to_proto(&config)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::config_view_from_proto(view.get_ref()))
    }
}

#[async_trait::async_trait]
impl api::JobApi for GrpcApi {
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
}

impl api::EventApi for GrpcApi {
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
    let mut attached = false;
    loop {
        // A reattach means the daemon restarted, so the mirror is stale in
        // ways no cursor could reconcile. Resync tells the consumer to
        // refetch, which is the same thing it does for a lagged channel.
        if attached && tx.send(api::ApiEvent::Resync).is_err() {
            return;
        }
        match stream_once(client.clone(), &tx).await {
            Ok(()) => return,
            // Nothing is listening on the socket, so there is no daemon to
            // reattach to: end the stream and let the consumer decide.
            Err(error) if error.code == api::ErrorCode::DaemonGone => {
                tracing::info!(%error, "the daemon is gone; ending the event stream");
                return;
            }
            Err(error) => tracing::debug!(%error, "daemon event stream ended; reattaching"),
        }
        attached = true;
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

async fn stream_once(
    mut client: Client,
    tx: &tokio::sync::mpsc::UnboundedSender<api::ApiEvent>,
) -> Result<(), ApiError> {
    let mut inbound = client
        .subscribe(Request::new(proto::SubscribeRequest {}))
        .await
        .map_err(wire_error)?
        .into_inner();
    loop {
        // Watch for the consumer going away as well as for events: parked in
        // message() alone, this task would never notice its receiver was
        // dropped, and the daemon would keep counting a frontend that is no
        // longer listening.
        let message = tokio::select! {
            message = inbound.message() => message,
            () = tx.closed() => return Ok(()),
        };
        match message {
            Ok(Some(proto::EventEnvelope { event })) => {
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

#[async_trait::async_trait]
impl api::PlaylistApi for GrpcApi {
    async fn playlists(&self) -> Result<api::PlaylistCatalog, ApiError> {
        let catalog = self
            .client()
            .get_playlists(Request::new(proto::GetPlaylistsRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::playlist_catalog_from_proto(catalog.get_ref()))
    }

    async fn create_playlist(&self, name: String, keys: Vec<String>) -> Result<String, ApiError> {
        let id = self
            .client()
            .create_playlist(Request::new(proto::CreatePlaylistRequest { name, keys }))
            .await
            .map_err(wire_error)?;
        Ok(id.into_inner().id)
    }

    async fn rename_playlist(&self, id: String, name: String) -> Result<(), ApiError> {
        self.client()
            .rename_playlist(Request::new(proto::RenamePlaylistRequest { id, name }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn delete_playlist(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .delete_playlist(Request::new(proto::PlaylistId { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn add_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        self.client()
            .add_playlist_tracks(Request::new(proto::AddPlaylistTracksRequest { id, keys }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn remove_playlist_track(&self, id: String, index: u32) -> Result<(), ApiError> {
        self.client()
            .remove_playlist_track(Request::new(proto::RemovePlaylistTrackRequest {
                id,
                index,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn reorder_playlist(
        &self,
        id: String,
        reorder: api::PlaylistReorder,
    ) -> Result<(), ApiError> {
        self.client()
            .reorder_playlist(Request::new(proto::ReorderPlaylistRequest {
                id,
                from: reorder.from,
                to: reorder.to,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn refresh_playlist(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .refresh_playlist(Request::new(proto::PlaylistId { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn create_playlist_folder(&self, name: String) -> Result<String, ApiError> {
        let id = self
            .client()
            .create_playlist_folder(Request::new(proto::CreatePlaylistFolderRequest { name }))
            .await
            .map_err(wire_error)?;
        Ok(id.into_inner().id)
    }

    async fn rename_playlist_folder(&self, id: String, name: String) -> Result<(), ApiError> {
        self.client()
            .rename_playlist_folder(Request::new(proto::RenamePlaylistFolderRequest {
                id,
                name,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn delete_playlist_folder(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .delete_playlist_folder(Request::new(proto::PlaylistFolderId { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn move_playlist(
        &self,
        playlist_id: String,
        folder_id: Option<String>,
    ) -> Result<(), ApiError> {
        self.client()
            .move_playlist(Request::new(proto::MovePlaylistRequest {
                playlist_id,
                folder_id,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl api::SourceApi for GrpcApi {
    async fn sources(&self) -> Result<Vec<api::SourceInfo>, ApiError> {
        let list = self
            .client()
            .get_sources(Request::new(proto::GetSourcesRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(list
            .get_ref()
            .sources
            .iter()
            .map(convert::source_info_from_proto)
            .collect())
    }

    async fn switch_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        let info = self
            .client()
            .select_source(Request::new(proto::SourceId { id }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(info.get_ref()))
    }

    async fn upsert_local_source(
        &self,
        draft: api::LocalSourceDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        let info = self
            .client()
            .upsert_local_source(Request::new(convert::local_draft_to_proto(&draft)))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(info.get_ref()))
    }

    async fn delete_local_source(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .delete_local_source(Request::new(proto::SourceId { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn set_source_directories(
        &self,
        id: String,
        directories: Vec<String>,
    ) -> Result<api::SourceInfo, ApiError> {
        let info = self
            .client()
            .set_source_directories(Request::new(proto::SetSourceDirectoriesRequest {
                id,
                directories,
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(info.get_ref()))
    }

    async fn upsert_server(&self, draft: api::ServerDraft) -> Result<api::SourceInfo, ApiError> {
        let info = self
            .client()
            .upsert_server(Request::new(convert::server_draft_to_proto(&draft)))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(info.get_ref()))
    }

    async fn delete_server(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .delete_server(Request::new(proto::SourceId { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn provision_credentials(
        &self,
        provision: api::CredentialProvision,
    ) -> Result<api::SourceInfo, ApiError> {
        let info = self
            .client()
            .provision_credentials(Request::new(convert::credential_provision_to_proto(
                &provision,
            )))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(info.get_ref()))
    }

    async fn login_source(
        &self,
        request: api::SourceLoginRequest,
    ) -> Result<api::SourceInfo, ApiError> {
        let info = self
            .client()
            .login_source(Request::new(proto::SourceLoginRequest {
                server_id: request.server_id,
                username: request.username,
                password: request.password,
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(info.get_ref()))
    }

    async fn clear_credentials(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .clear_credentials(Request::new(proto::SourceId { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn authenticate_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        let info = self
            .client()
            .authenticate_source(Request::new(proto::SourceId { id }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(info.get_ref()))
    }

    async fn browse_source(
        &self,
        id: String,
        path: String,
    ) -> Result<Vec<api::SourceFolderEntry>, ApiError> {
        let list = self
            .client()
            .browse_source(Request::new(proto::BrowseSourceRequest { id, path }))
            .await
            .map_err(wire_error)?;
        Ok(list
            .get_ref()
            .entries
            .iter()
            .map(|entry| api::SourceFolderEntry {
                path: entry.path.clone(),
                name: entry.name.clone(),
            })
            .collect())
    }

    async fn validate_source(&self, id: String) -> Result<api::SourceState, ApiError> {
        let state = self
            .client()
            .validate_source(Request::new(proto::SourceId { id }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_state_from_proto(state.get_ref().state))
    }

    async fn integrations(&self) -> Result<Vec<api::IntegrationStatus>, ApiError> {
        let list = self
            .client()
            .get_integrations(Request::new(proto::GetIntegrationsRequest {}))
            .await
            .map_err(wire_error)?;
        Ok(list
            .get_ref()
            .integrations
            .iter()
            .map(convert::integration_status_from_proto)
            .collect())
    }

    async fn provision_integration(
        &self,
        provision: api::IntegrationProvision,
    ) -> Result<api::IntegrationStatus, ApiError> {
        let status = self
            .client()
            .provision_integration(Request::new(convert::integration_provision_to_proto(
                &provision,
            )))
            .await
            .map_err(wire_error)?;
        Ok(convert::integration_status_from_proto(status.get_ref()))
    }

    async fn clear_integration(&self, kind: api::IntegrationKind) -> Result<(), ApiError> {
        self.client()
            .clear_integration(Request::new(proto::IntegrationRef {
                kind: convert::integration_kind_to_proto(kind) as i32,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn authenticate_integration(
        &self,
        kind: api::IntegrationKind,
    ) -> Result<api::IntegrationStatus, ApiError> {
        let status = self
            .client()
            .authenticate_integration(Request::new(proto::IntegrationRef {
                kind: convert::integration_kind_to_proto(kind) as i32,
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::integration_status_from_proto(status.get_ref()))
    }
}
