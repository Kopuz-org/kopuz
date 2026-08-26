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
const MAX_MESSAGE_BYTES: usize = 33 * 1024 * 1024;

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
            client: KopuzClient::with_interceptor(channel, AuthInterceptor { header })
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
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

    async fn queue_snapshot(&self) -> Result<api::QueuePersistenceSnapshot, ApiError> {
        let snapshot = self
            .client()
            .get_queue_snapshot(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::queue_persistence_snapshot_from_proto(
            snapshot.get_ref(),
        ))
    }

    async fn save_queue_snapshot(
        &self,
        snapshot: api::QueuePersistenceSnapshot,
    ) -> Result<(), ApiError> {
        self.client()
            .save_queue_snapshot(Request::new(convert::queue_persistence_snapshot_to_proto(
                &snapshot,
            )))
            .await
            .map_err(wire_error)?;
        Ok(())
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

    fn lyrics_stream(&self, key: String) -> api::LyricsStream {
        use futures_util::StreamExt as _;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut client = self.client();
        tokio::spawn(async move {
            let response = client
                .stream_lyrics(Request::new(proto::TrackRef { key }))
                .await;
            let mut stream = match response {
                Ok(response) => response.into_inner(),
                Err(status) => {
                    let _ = tx.send(Err(wire_error(status)));
                    return;
                }
            };
            loop {
                match stream.message().await {
                    Ok(Some(lyrics)) => {
                        if tx.send(Ok(convert::lyrics_from_proto(&lyrics))).is_err() {
                            return;
                        }
                    }
                    Ok(None) => return,
                    Err(status) => {
                        let _ = tx.send(Err(wire_error(status)));
                        return;
                    }
                }
            }
        });
        futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        })
        .boxed()
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

    async fn download_statuses(&self) -> Result<Vec<api::DownloadItemStatus>, ApiError> {
        let list = self
            .client()
            .get_download_statuses(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(list
            .get_ref()
            .statuses
            .iter()
            .map(convert::download_status_from_proto)
            .collect())
    }

    async fn cancel_download_item(&self, key: String) -> Result<(), ApiError> {
        self.client()
            .cancel_download_item(Request::new(proto::TrackRef { key }))
            .await
            .map_err(wire_error)?;
        Ok(())
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

    async fn albums(
        &self,
        filter: api::AlbumFilter,
        page: Page,
    ) -> Result<api::AlbumPage, ApiError> {
        let response = self
            .client()
            .get_albums(Request::new(proto::AlbumsRequest {
                filter: Some(convert::album_filter_to_proto(&filter)),
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::album_page_from_proto(response.get_ref()))
    }

    async fn album(&self, id: String) -> Result<api::AlbumInfo, ApiError> {
        let response = self
            .client()
            .get_album(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(convert::album_info_from_proto(response.get_ref()))
    }

    async fn artists(&self, page: Page) -> Result<api::ArtistPage, ApiError> {
        let response = self
            .client()
            .get_artists(Request::new(convert::page_to_proto(page)))
            .await
            .map_err(wire_error)?;
        Ok(convert::artist_page_from_proto(response.get_ref()))
    }

    async fn refresh_artist_artwork(&self, names: Vec<String>) -> Result<Vec<String>, ApiError> {
        Ok(self
            .client()
            .refresh_artist_artwork(Request::new(proto::StringList { values: names }))
            .await
            .map_err(wire_error)?
            .into_inner()
            .values)
    }

    async fn genres(&self) -> Result<Vec<String>, ApiError> {
        let response = self
            .client()
            .get_genres(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(response.get_ref().values.clone())
    }

    async fn recent_tracks(&self, page: Page) -> Result<TrackPage, ApiError> {
        let response = self
            .client()
            .get_recent_tracks(Request::new(convert::page_to_proto(page)))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn album_tracks(&self, id: String, page: Page) -> Result<TrackPage, ApiError> {
        let response = self
            .client()
            .get_album_tracks(Request::new(proto::EntityPage {
                value: id,
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn artist_tracks(&self, name: String, page: Page) -> Result<TrackPage, ApiError> {
        let response = self
            .client()
            .get_artist_tracks(Request::new(proto::EntityPage {
                value: name,
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn genre_tracks(&self, name: String, page: Page) -> Result<TrackPage, ApiError> {
        let response = self
            .client()
            .get_genre_tracks(Request::new(proto::EntityPage {
                value: name,
                page: Some(convert::page_to_proto(page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn artist_sample_tracks(&self, page: Page) -> Result<TrackPage, ApiError> {
        let response = self
            .client()
            .get_artist_sample_tracks(Request::new(convert::page_to_proto(page)))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn tracks_by_keys(&self, keys: Vec<String>) -> Result<Vec<api::TrackInfo>, ApiError> {
        let response = self
            .client()
            .get_tracks_by_keys(Request::new(proto::TrackKeysRequest { keys }))
            .await
            .map_err(wire_error)?;
        Ok(response
            .get_ref()
            .tracks
            .iter()
            .map(convert::track_info_from_proto)
            .collect())
    }

    async fn track_web_url(&self, key: String) -> Result<Option<String>, ApiError> {
        let response = self
            .client()
            .get_track_web_url(Request::new(proto::TrackRef { key }))
            .await
            .map_err(wire_error)?;
        Ok(response.get_ref().value.clone())
    }

    async fn album_web_url(&self, id: String) -> Result<Option<String>, ApiError> {
        let response = self
            .client()
            .get_album_web_url(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(response.get_ref().value.clone())
    }

    async fn top_genre(&self) -> Result<Option<String>, ApiError> {
        let response = self
            .client()
            .get_top_genre(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(response.get_ref().value.clone())
    }

    async fn search(&self, query: String) -> Result<api::SearchResults, ApiError> {
        let response = self
            .client()
            .search(Request::new(proto::SearchRequest { query }))
            .await
            .map_err(wire_error)?;
        Ok(convert::search_results_from_proto(response.get_ref()))
    }

    async fn playlists(&self) -> Result<api::PlaylistCatalog, ApiError> {
        let response = self
            .client()
            .get_playlists(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(convert::playlist_catalog_from_proto(response.get_ref()))
    }

    async fn playlist_tracks(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<TrackPage, ApiError> {
        let response = self
            .client()
            .get_playlist_tracks(Request::new(proto::PlaylistTracksRequest {
                id: request.id,
                page: Some(convert::page_to_proto(request.page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn refresh_playlist(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<api::TrackPage, ApiError> {
        let response = self
            .client()
            .refresh_playlist(Request::new(proto::PlaylistTracksRequest {
                id: request.id,
                page: Some(convert::page_to_proto(request.page)),
            }))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn create_playlist(&self, name: String, keys: Vec<String>) -> Result<String, ApiError> {
        let response = self
            .client()
            .create_playlist(Request::new(proto::CreatePlaylistRequest { name, keys }))
            .await
            .map_err(wire_error)?;
        Ok(response.get_ref().id.clone())
    }

    async fn rename_playlist(&self, id: String, name: String) -> Result<(), ApiError> {
        self.client()
            .rename_playlist(Request::new(proto::NamedEntity { id, name }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn delete_playlist(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .delete_playlist(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn add_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        self.client()
            .add_playlist_tracks(Request::new(proto::PlaylistKeysRequest { id, keys }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn remove_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        self.client()
            .remove_playlist_tracks(Request::new(proto::PlaylistKeysRequest { id, keys }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn reorder_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        self.client()
            .reorder_playlist_tracks(Request::new(proto::PlaylistKeysRequest { id, keys }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn create_playlist_folder(&self, name: String) -> Result<String, ApiError> {
        let response = self
            .client()
            .create_playlist_folder(Request::new(proto::Name { name }))
            .await
            .map_err(wire_error)?;
        Ok(response.get_ref().id.clone())
    }

    async fn rename_playlist_folder(&self, id: String, name: String) -> Result<(), ApiError> {
        self.client()
            .rename_playlist_folder(Request::new(proto::NamedEntity { id, name }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn delete_playlist_folder(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .delete_playlist_folder(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn move_playlist(&self, id: String, folder_id: Option<String>) -> Result<(), ApiError> {
        self.client()
            .move_playlist(Request::new(proto::MovePlaylistRequest { id, folder_id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn sources(&self) -> Result<Vec<api::SourceInfo>, ApiError> {
        let response = self
            .client()
            .get_sources(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(response
            .get_ref()
            .sources
            .iter()
            .map(convert::source_info_from_proto)
            .collect())
    }

    async fn switch_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        let response = self
            .client()
            .switch_source(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn upsert_local_source(
        &self,
        source: api::LocalSourceDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        let response = self
            .client()
            .upsert_local_source(Request::new(convert::local_source_draft_to_proto(&source)))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn delete_local_source(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .delete_local_source(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn set_source_directories(
        &self,
        id: String,
        directories: Vec<String>,
    ) -> Result<api::SourceInfo, ApiError> {
        let response = self
            .client()
            .set_source_directories(Request::new(proto::SourceDirectories { id, directories }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn upsert_server(&self, server: api::ServerDraft) -> Result<api::SourceInfo, ApiError> {
        let response = self
            .client()
            .upsert_server(Request::new(convert::server_draft_to_proto(&server)))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn delete_server(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .delete_server(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn provision_credentials(
        &self,
        provision: api::CredentialProvision,
    ) -> Result<api::SourceInfo, ApiError> {
        let response = self
            .client()
            .provision_credentials(Request::new(convert::credential_to_proto(&provision)))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn login_source(
        &self,
        request: api::SourceLoginRequest,
    ) -> Result<api::SourceInfo, ApiError> {
        let response = self
            .client()
            .login_source(Request::new(convert::source_login_to_proto(&request)))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn clear_credentials(&self, id: String) -> Result<(), ApiError> {
        self.client()
            .clear_credentials(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn authenticate_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        let response = self
            .client()
            .authenticate_source(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn browse_source(
        &self,
        id: String,
        path: String,
    ) -> Result<Vec<api::SourceFolderEntry>, ApiError> {
        let response = self
            .client()
            .browse_source(Request::new(proto::SourceFolderRequest {
                server_id: id,
                path,
            }))
            .await
            .map_err(wire_error)?;
        Ok(response
            .get_ref()
            .entries
            .iter()
            .map(convert::source_folder_from_proto)
            .collect())
    }

    async fn integration_credentials(
        &self,
    ) -> Result<Vec<api::IntegrationCredentialStatus>, ApiError> {
        let response = self
            .client()
            .get_integration_credentials(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(response
            .get_ref()
            .statuses
            .iter()
            .map(convert::integration_status_from_proto)
            .collect())
    }

    async fn provision_integration_credentials(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        let response = self
            .client()
            .provision_integration_credentials(Request::new(
                convert::integration_provision_to_proto(&provision),
            ))
            .await
            .map_err(wire_error)?;
        Ok(convert::integration_status_from_proto(response.get_ref()))
    }

    async fn clear_integration_credentials(
        &self,
        kind: api::IntegrationKind,
    ) -> Result<(), ApiError> {
        self.client()
            .clear_integration_credentials(Request::new(proto::IntegrationRef {
                kind: convert::integration_kind_to_proto(kind) as i32,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn authenticate_integration(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        let response = self
            .client()
            .authenticate_integration(Request::new(convert::integration_provision_to_proto(
                &provision,
            )))
            .await
            .map_err(wire_error)?;
        Ok(convert::integration_status_from_proto(response.get_ref()))
    }

    async fn validate_source(&self, id: String) -> Result<api::SourceState, ApiError> {
        let response = self
            .client()
            .validate_source(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?;
        Ok(convert::source_state_from_proto(response.get_ref().state))
    }

    async fn external_access(&self, kind: String) -> Result<api::ExternalAccess, ApiError> {
        let response = self
            .client()
            .get_external_access(Request::new(proto::ExternalAccessRequest { kind }))
            .await
            .map_err(wire_error)?;
        Ok(convert::external_access_from_proto(response.get_ref()))
    }

    async fn set_external_playback(
        &self,
        external: Option<api::ExternalPlayback>,
    ) -> Result<(), ApiError> {
        self.client()
            .set_external_playback(Request::new(proto::SetExternalPlaybackRequest {
                external: external.map(|external| proto::ExternalPlayback {
                    kind: external.kind,
                    device: external.device,
                }),
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn claim_external_playback(
        &self,
        external: api::ExternalPlayback,
    ) -> Result<api::ExternalPlaybackLease, ApiError> {
        let response = self
            .client()
            .claim_external_playback(Request::new(convert::external_playback_to_proto(&external)))
            .await
            .map_err(wire_error)?;
        Ok(convert::external_lease_from_proto(response.get_ref()))
    }

    async fn report_external_playback(
        &self,
        report: api::ExternalPlaybackReport,
    ) -> Result<(), ApiError> {
        self.client()
            .report_external_playback(Request::new(convert::external_report_to_proto(&report)))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn release_external_playback(&self, lease_id: String) -> Result<(), ApiError> {
        self.client()
            .release_external_playback(Request::new(proto::ExternalPlaybackLease {
                lease_id,
                expires_in_ms: 0,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn start_ytdlp(&self, request: api::YtdlpRequest) -> Result<JobRef, ApiError> {
        let response = self
            .client()
            .start_ytdlp(Request::new(convert::ytdlp_request_to_proto(&request)))
            .await
            .map_err(wire_error)?;
        Ok(JobRef {
            job_id: response.get_ref().job_id.clone(),
        })
    }

    async fn catalog(&self, continuation: Option<String>) -> Result<api::CatalogPage, ApiError> {
        let response = self
            .client()
            .get_catalog(Request::new(proto::CatalogRequest { continuation }))
            .await
            .map_err(wire_error)?;
        Ok(convert::catalog_page_from_proto(response.get_ref()))
    }

    async fn catalog_detail(
        &self,
        request: api::CatalogDetailRequest,
    ) -> Result<api::CatalogDetail, ApiError> {
        let response = self
            .client()
            .get_catalog_detail(Request::new(convert::catalog_detail_request_to_proto(
                &request,
            )))
            .await
            .map_err(wire_error)?;
        Ok(convert::catalog_detail_from_proto(response.get_ref()))
    }

    async fn radio_stations(&self) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        let response = self
            .client()
            .get_radio_stations(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(response
            .get_ref()
            .stations
            .iter()
            .map(convert::radio_station_from_proto)
            .collect())
    }

    async fn track_radio(&self, key: String) -> Result<Vec<api::TrackInfo>, ApiError> {
        let response = self
            .client()
            .start_track_radio(Request::new(proto::TrackRef { key }))
            .await
            .map_err(wire_error)?
            .into_inner();
        Ok(response
            .tracks
            .iter()
            .map(convert::track_info_from_proto)
            .collect())
    }

    async fn playlist_radio(&self, id: String) -> Result<Vec<api::TrackInfo>, ApiError> {
        let response = self
            .client()
            .start_playlist_radio(Request::new(proto::EntityRef { id }))
            .await
            .map_err(wire_error)?
            .into_inner();
        Ok(response
            .tracks
            .iter()
            .map(convert::track_info_from_proto)
            .collect())
    }

    async fn search_radio(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        let response = self
            .client()
            .search_radio(Request::new(proto::RadioSearchRequest { query, limit }))
            .await
            .map_err(wire_error)?;
        Ok(response
            .get_ref()
            .stations
            .iter()
            .map(convert::radio_station_from_proto)
            .collect())
    }

    async fn radio_registries(&self) -> Result<Vec<api::RadioRegistryInfo>, ApiError> {
        let response = self
            .client()
            .get_radio_registries(Request::new(proto::Empty {}))
            .await
            .map_err(wire_error)?;
        Ok(response
            .get_ref()
            .registries
            .iter()
            .map(convert::radio_registry_from_proto)
            .collect())
    }

    async fn add_radio_registry(&self, url: String) -> Result<(), ApiError> {
        self.client()
            .add_radio_registry(Request::new(proto::RegistryRequest { url }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn remove_radio_registry(&self, url: String) -> Result<(), ApiError> {
        self.client()
            .remove_radio_registry(Request::new(proto::RegistryRequest { url }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn set_radio_registry_enabled(&self, url: String, enabled: bool) -> Result<(), ApiError> {
        self.client()
            .set_radio_registry_enabled(Request::new(proto::SetRegistryEnabledRequest {
                url,
                enabled,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn pin_radio_station(
        &self,
        station: api::RadioStationInfo,
        pinned: bool,
    ) -> Result<(), ApiError> {
        self.client()
            .pin_radio_station(Request::new(proto::PinRadioStationRequest {
                station: Some(convert::radio_station_to_proto(&station)),
                pinned,
            }))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn update_track_metadata(
        &self,
        patch: api::TrackMetadataPatch,
    ) -> Result<api::TrackInfo, ApiError> {
        let response = self
            .client()
            .update_track_metadata(Request::new(convert::metadata_patch_to_proto(&patch)))
            .await
            .map_err(wire_error)?;
        Ok(convert::track_info_from_proto(response.get_ref()))
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
            .remove_artwork(Request::new(convert::remove_artwork_to_proto(&target)))
            .await
            .map_err(wire_error)?;
        Ok(())
    }

    async fn artwork(&self, request: api::ArtworkRequest) -> Result<api::ArtworkData, ApiError> {
        const MAX_ARTWORK_BYTES: usize = 32 * 1024 * 1024;
        if request.entity.is_none() {
            return Err(ApiError::invalid_input("artwork entity is required"));
        }
        let mut stream = self
            .client()
            .get_artwork(Request::new(convert::artwork_request_to_proto(&request)))
            .await
            .map_err(wire_error)?
            .into_inner();
        let mut content_type = String::new();
        let mut data = Vec::new();
        while let Some(chunk) = stream.message().await.map_err(wire_error)? {
            if content_type.is_empty() && !chunk.content_type.is_empty() {
                content_type = chunk.content_type;
            }
            if data.len().saturating_add(chunk.data.len()) > MAX_ARTWORK_BYTES {
                return Err(ApiError::invalid_input("artwork response is too large"));
            }
            data.extend_from_slice(&chunk.data);
        }
        if content_type.is_empty() || data.is_empty() {
            return Err(ApiError::internal("the daemon returned empty artwork"));
        }
        Ok(api::ArtworkData { content_type, data })
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
