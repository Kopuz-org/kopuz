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

macro_rules! rpc {
    ($api:expr, $method:ident, $request:expr) => {
        $api.client()
            .$method(Request::new($request))
            .await
            .map_err(wire_error)?
    };
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
        let state = rpc!(self, get_player_state, proto::Empty {});
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
        let window = rpc!(self, get_queue, convert::page_to_proto(page));
        Ok(convert::queue_window_from_proto(window.get_ref()))
    }

    async fn set_queue(&self, request: SetQueueRequest) -> Result<CommandAck, ApiError> {
        let ack = rpc!(self, set_queue, convert::set_queue_to_proto(&request));
        Ok(CommandAck {
            rev: ack.get_ref().rev,
        })
    }

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError> {
        let ack = rpc!(self, edit_queue, convert::queue_edit_to_proto(&edit));
        Ok(CommandAck {
            rev: ack.get_ref().rev,
        })
    }

    async fn queue_snapshot(&self) -> Result<api::QueuePersistenceSnapshot, ApiError> {
        let snapshot = rpc!(self, get_queue_snapshot, proto::Empty {});
        Ok(convert::queue_persistence_snapshot_from_proto(
            snapshot.get_ref(),
        ))
    }

    async fn live_queue(&self) -> Result<api::QueuePersistenceSnapshot, ApiError> {
        let snapshot = rpc!(self, get_live_queue, proto::Empty {});
        Ok(convert::queue_persistence_snapshot_from_proto(
            snapshot.get_ref(),
        ))
    }

    async fn save_queue_snapshot(
        &self,
        snapshot: api::QueuePersistenceSnapshot,
    ) -> Result<(), ApiError> {
        rpc!(
            self,
            save_queue_snapshot,
            convert::queue_persistence_snapshot_to_proto(&snapshot)
        );
        Ok(())
    }

    async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError> {
        let tracks = rpc!(
            self,
            get_tracks,
            proto::TracksRequest {
                filter: Some(convert::track_filter_to_proto(&filter)),
                page: Some(convert::page_to_proto(page)),
            }
        );
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn config(&self) -> Result<ConfigView, ApiError> {
        let view = rpc!(self, get_config, proto::Empty {});
        Ok(convert::config_view_from_proto(view.get_ref()))
    }

    async fn patch_config(&self, patch: serde_json::Value) -> Result<ConfigView, ApiError> {
        let view = rpc!(
            self,
            patch_config,
            proto::ConfigPatch {
                merge_patch_json: patch.to_string(),
            }
        );
        Ok(convert::config_view_from_proto(view.get_ref()))
    }

    async fn preview_equalizer(&self, equalizer: serde_json::Value) -> Result<(), ApiError> {
        rpc!(
            self,
            preview_equalizer,
            proto::ConfigPatch {
                merge_patch_json: equalizer.to_string(),
            }
        );
        Ok(())
    }

    async fn favorites(&self) -> Result<FavoritesView, ApiError> {
        let favorites = rpc!(self, get_favorites, proto::Empty {});
        Ok(convert::favorites_from_proto(favorites.get_ref()))
    }

    async fn set_favorite(&self, key: String, favorite: bool) -> Result<(), ApiError> {
        rpc!(self, set_favorite, proto::FavoriteRequest { key, favorite });
        Ok(())
    }

    async fn start_job(&self, kind: JobKind) -> Result<JobRef, ApiError> {
        let job = rpc!(
            self,
            start_job,
            proto::StartJobRequest {
                kind: convert::job_kind_to_proto(kind) as i32,
            }
        );
        Ok(JobRef {
            job_id: job.get_ref().job_id.clone(),
        })
    }

    async fn folder_tracks(&self, prefix: String, page: Page) -> Result<api::TrackPage, ApiError> {
        let tracks = rpc!(
            self,
            get_folder_tracks,
            proto::FolderRequest {
                prefix,
                page: Some(convert::page_to_proto(page)),
            }
        );
        Ok(convert::track_page_from_proto(tracks.get_ref()))
    }

    async fn lyrics(&self, key: String) -> Result<api::LyricsView, ApiError> {
        let lyrics = rpc!(self, get_lyrics, proto::TrackRef { key });
        Ok(convert::lyrics_from_proto(lyrics.get_ref()))
    }

    fn lyrics_stream(&self, key: String) -> api::LyricsStream {
        use futures_util::StreamExt as _;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut client = self.client();
        tokio::spawn(async move {
            let cancel = tx.clone();
            let response = tokio::select! {
                biased;
                () = cancel.closed() => return,
                response = client.stream_lyrics(Request::new(proto::TrackRef { key })) => response,
            };
            let mut stream = match response {
                Ok(response) => response.into_inner(),
                Err(status) => {
                    let _ = tx.send(Err(wire_error(status)));
                    return;
                }
            };
            loop {
                let message = tokio::select! {
                    biased;
                    () = cancel.closed() => return,
                    message = stream.message() => message,
                };
                match message {
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
        let stats = rpc!(self, get_stats, proto::Empty {});
        Ok(convert::stats_from_proto(stats.get_ref()))
    }

    async fn download(&self, keys: Vec<String>) -> Result<JobRef, ApiError> {
        let job = rpc!(self, start_downloads, proto::DownloadRequest { keys });
        Ok(JobRef {
            job_id: job.get_ref().job_id.clone(),
        })
    }

    async fn downloads(&self) -> Result<Vec<String>, ApiError> {
        let list = rpc!(self, get_downloads, proto::Empty {});
        Ok(list.get_ref().keys.clone())
    }

    async fn download_statuses(&self) -> Result<Vec<api::DownloadItemStatus>, ApiError> {
        let list = rpc!(self, get_download_statuses, proto::Empty {});
        Ok(list
            .get_ref()
            .statuses
            .iter()
            .map(convert::download_status_from_proto)
            .collect())
    }

    async fn cancel_download_item(&self, key: String) -> Result<(), ApiError> {
        rpc!(self, cancel_download_item, proto::TrackRef { key });
        Ok(())
    }

    async fn remove_download(&self, key: String) -> Result<(), ApiError> {
        rpc!(self, remove_download, proto::TrackRef { key });
        Ok(())
    }

    async fn jobs(&self) -> Result<Vec<JobStatus>, ApiError> {
        let jobs = rpc!(self, get_jobs, proto::Empty {});
        Ok(jobs
            .get_ref()
            .jobs
            .iter()
            .map(convert::job_status_from_proto)
            .collect())
    }

    async fn cancel_job(&self, id: String) -> Result<(), ApiError> {
        rpc!(self, cancel_job, proto::JobId { id });
        Ok(())
    }

    async fn albums(
        &self,
        filter: api::AlbumFilter,
        page: Page,
    ) -> Result<api::AlbumPage, ApiError> {
        let response = rpc!(
            self,
            get_albums,
            proto::AlbumsRequest {
                filter: Some(convert::album_filter_to_proto(&filter)),
                page: Some(convert::page_to_proto(page)),
            }
        );
        Ok(convert::album_page_from_proto(response.get_ref()))
    }

    async fn album(&self, id: String) -> Result<api::AlbumInfo, ApiError> {
        let response = rpc!(self, get_album, proto::EntityRef { id });
        Ok(convert::album_info_from_proto(response.get_ref()))
    }

    async fn artists(&self, page: Page) -> Result<api::ArtistPage, ApiError> {
        let response = rpc!(self, get_artists, convert::page_to_proto(page));
        Ok(convert::artist_page_from_proto(response.get_ref()))
    }

    async fn refresh_artist_artwork(&self, names: Vec<String>) -> Result<Vec<String>, ApiError> {
        Ok(rpc!(
            self,
            refresh_artist_artwork,
            proto::StringList { values: names }
        )
        .into_inner()
        .values)
    }

    async fn genres(&self) -> Result<Vec<String>, ApiError> {
        let response = rpc!(self, get_genres, proto::Empty {});
        Ok(response.get_ref().values.clone())
    }

    async fn recent_tracks(&self, page: Page) -> Result<TrackPage, ApiError> {
        let response = rpc!(self, get_recent_tracks, convert::page_to_proto(page));
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn album_tracks(&self, id: String, page: Page) -> Result<TrackPage, ApiError> {
        let response = rpc!(
            self,
            get_album_tracks,
            proto::EntityPage {
                value: id,
                page: Some(convert::page_to_proto(page)),
            }
        );
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn artist_tracks(&self, name: String, page: Page) -> Result<TrackPage, ApiError> {
        let response = rpc!(
            self,
            get_artist_tracks,
            proto::EntityPage {
                value: name,
                page: Some(convert::page_to_proto(page)),
            }
        );
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn genre_tracks(&self, name: String, page: Page) -> Result<TrackPage, ApiError> {
        let response = rpc!(
            self,
            get_genre_tracks,
            proto::EntityPage {
                value: name,
                page: Some(convert::page_to_proto(page)),
            }
        );
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn artist_sample_tracks(&self, page: Page) -> Result<TrackPage, ApiError> {
        let response = rpc!(self, get_artist_sample_tracks, convert::page_to_proto(page));
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn tracks_by_keys(&self, keys: Vec<String>) -> Result<Vec<api::TrackInfo>, ApiError> {
        let response = rpc!(self, get_tracks_by_keys, proto::TrackKeysRequest { keys });
        Ok(response
            .get_ref()
            .tracks
            .iter()
            .map(convert::track_info_from_proto)
            .collect())
    }

    async fn track_web_url(&self, key: String) -> Result<Option<String>, ApiError> {
        let response = rpc!(self, get_track_web_url, proto::TrackRef { key });
        Ok(response.get_ref().value.clone())
    }

    async fn album_web_url(&self, id: String) -> Result<Option<String>, ApiError> {
        let response = rpc!(self, get_album_web_url, proto::EntityRef { id });
        Ok(response.get_ref().value.clone())
    }

    async fn top_genre(&self) -> Result<Option<String>, ApiError> {
        let response = rpc!(self, get_top_genre, proto::Empty {});
        Ok(response.get_ref().value.clone())
    }

    async fn search(&self, query: String) -> Result<api::SearchResults, ApiError> {
        let response = rpc!(self, search, proto::SearchRequest { query });
        Ok(convert::search_results_from_proto(response.get_ref()))
    }

    async fn playlists(&self) -> Result<api::PlaylistCatalog, ApiError> {
        let response = rpc!(self, get_playlists, proto::Empty {});
        Ok(convert::playlist_catalog_from_proto(response.get_ref()))
    }

    async fn playlist_tracks(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<TrackPage, ApiError> {
        let response = rpc!(
            self,
            get_playlist_tracks,
            proto::PlaylistTracksRequest {
                id: request.id,
                page: Some(convert::page_to_proto(request.page)),
            }
        );
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn refresh_playlist(
        &self,
        request: api::PlaylistTracksRequest,
    ) -> Result<api::TrackPage, ApiError> {
        let response = rpc!(
            self,
            refresh_playlist,
            proto::PlaylistTracksRequest {
                id: request.id,
                page: Some(convert::page_to_proto(request.page)),
            }
        );
        Ok(convert::track_page_from_proto(response.get_ref()))
    }

    async fn create_playlist(&self, name: String, keys: Vec<String>) -> Result<String, ApiError> {
        let response = rpc!(
            self,
            create_playlist,
            proto::CreatePlaylistRequest { name, keys }
        );
        Ok(response.get_ref().id.clone())
    }

    async fn rename_playlist(&self, id: String, name: String) -> Result<(), ApiError> {
        rpc!(self, rename_playlist, proto::NamedEntity { id, name });
        Ok(())
    }

    async fn delete_playlist(&self, id: String) -> Result<(), ApiError> {
        rpc!(self, delete_playlist, proto::EntityRef { id });
        Ok(())
    }

    async fn add_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        rpc!(
            self,
            add_playlist_tracks,
            proto::PlaylistKeysRequest { id, keys }
        );
        Ok(())
    }

    async fn remove_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        rpc!(
            self,
            remove_playlist_tracks,
            proto::PlaylistKeysRequest { id, keys }
        );
        Ok(())
    }

    async fn reorder_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError> {
        rpc!(
            self,
            reorder_playlist_tracks,
            proto::PlaylistKeysRequest { id, keys }
        );
        Ok(())
    }

    async fn create_playlist_folder(&self, name: String) -> Result<String, ApiError> {
        let response = rpc!(self, create_playlist_folder, proto::Name { name });
        Ok(response.get_ref().id.clone())
    }

    async fn rename_playlist_folder(&self, id: String, name: String) -> Result<(), ApiError> {
        rpc!(
            self,
            rename_playlist_folder,
            proto::NamedEntity { id, name }
        );
        Ok(())
    }

    async fn delete_playlist_folder(&self, id: String) -> Result<(), ApiError> {
        rpc!(self, delete_playlist_folder, proto::EntityRef { id });
        Ok(())
    }

    async fn move_playlist(&self, id: String, folder_id: Option<String>) -> Result<(), ApiError> {
        rpc!(
            self,
            move_playlist,
            proto::MovePlaylistRequest { id, folder_id }
        );
        Ok(())
    }

    async fn sources(&self) -> Result<Vec<api::SourceInfo>, ApiError> {
        let response = rpc!(self, get_sources, proto::Empty {});
        Ok(response
            .get_ref()
            .sources
            .iter()
            .map(convert::source_info_from_proto)
            .collect())
    }

    async fn switch_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        let response = rpc!(self, switch_source, proto::EntityRef { id });
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn upsert_local_source(
        &self,
        source: api::LocalSourceDraft,
    ) -> Result<api::SourceInfo, ApiError> {
        let response = rpc!(
            self,
            upsert_local_source,
            convert::local_source_draft_to_proto(&source)
        );
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn delete_local_source(&self, id: String) -> Result<(), ApiError> {
        rpc!(self, delete_local_source, proto::EntityRef { id });
        Ok(())
    }

    async fn set_source_directories(
        &self,
        id: String,
        directories: Vec<String>,
    ) -> Result<api::SourceInfo, ApiError> {
        let response = rpc!(
            self,
            set_source_directories,
            proto::SourceDirectories { id, directories }
        );
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn upsert_server(&self, server: api::ServerDraft) -> Result<api::SourceInfo, ApiError> {
        let response = rpc!(self, upsert_server, convert::server_draft_to_proto(&server));
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn delete_server(&self, id: String) -> Result<(), ApiError> {
        rpc!(self, delete_server, proto::EntityRef { id });
        Ok(())
    }

    async fn provision_credentials(
        &self,
        provision: api::CredentialProvision,
    ) -> Result<api::SourceInfo, ApiError> {
        let response = rpc!(
            self,
            provision_credentials,
            convert::credential_to_proto(&provision)
        );
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn login_source(
        &self,
        request: api::SourceLoginRequest,
    ) -> Result<api::SourceInfo, ApiError> {
        let response = rpc!(self, login_source, convert::source_login_to_proto(&request));
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn clear_credentials(&self, id: String) -> Result<(), ApiError> {
        rpc!(self, clear_credentials, proto::EntityRef { id });
        Ok(())
    }

    async fn authenticate_source(&self, id: String) -> Result<api::SourceInfo, ApiError> {
        let response = rpc!(self, authenticate_source, proto::EntityRef { id });
        Ok(convert::source_info_from_proto(response.get_ref()))
    }

    async fn browse_source(
        &self,
        id: String,
        path: String,
    ) -> Result<Vec<api::SourceFolderEntry>, ApiError> {
        let response = rpc!(
            self,
            browse_source,
            proto::SourceFolderRequest {
                server_id: id,
                path,
            }
        );
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
        let response = rpc!(self, get_integration_credentials, proto::Empty {});
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
        let response = rpc!(
            self,
            provision_integration_credentials,
            convert::integration_provision_to_proto(&provision)
        );
        Ok(convert::integration_status_from_proto(response.get_ref()))
    }

    async fn clear_integration_credentials(
        &self,
        kind: api::IntegrationKind,
    ) -> Result<(), ApiError> {
        rpc!(
            self,
            clear_integration_credentials,
            proto::IntegrationRef {
                kind: convert::integration_kind_to_proto(kind) as i32,
            }
        );
        Ok(())
    }

    async fn authenticate_integration(
        &self,
        provision: api::IntegrationCredentialProvision,
    ) -> Result<api::IntegrationCredentialStatus, ApiError> {
        let response = rpc!(
            self,
            authenticate_integration,
            convert::integration_provision_to_proto(&provision)
        );
        Ok(convert::integration_status_from_proto(response.get_ref()))
    }

    async fn validate_source(&self, id: String) -> Result<api::SourceState, ApiError> {
        let response = rpc!(self, validate_source, proto::EntityRef { id });
        Ok(convert::source_state_from_proto(response.get_ref().state))
    }

    async fn external_access(&self, kind: String) -> Result<api::ExternalAccess, ApiError> {
        let response = rpc!(
            self,
            get_external_access,
            proto::ExternalAccessRequest { kind }
        );
        Ok(convert::external_access_from_proto(response.get_ref()))
    }

    async fn set_external_playback(
        &self,
        external: Option<api::ExternalPlayback>,
    ) -> Result<(), ApiError> {
        rpc!(
            self,
            set_external_playback,
            proto::SetExternalPlaybackRequest {
                external: external.map(|external| proto::ExternalPlayback {
                    kind: external.kind,
                    device: external.device,
                }),
            }
        );
        Ok(())
    }

    async fn claim_external_playback(
        &self,
        external: api::ExternalPlayback,
    ) -> Result<api::ExternalPlaybackLease, ApiError> {
        let response = rpc!(
            self,
            claim_external_playback,
            convert::external_playback_to_proto(&external)
        );
        Ok(convert::external_lease_from_proto(response.get_ref()))
    }

    async fn report_external_playback(
        &self,
        report: api::ExternalPlaybackReport,
    ) -> Result<(), ApiError> {
        rpc!(
            self,
            report_external_playback,
            convert::external_report_to_proto(&report)
        );
        Ok(())
    }

    async fn release_external_playback(&self, lease_id: String) -> Result<(), ApiError> {
        rpc!(
            self,
            release_external_playback,
            proto::ExternalPlaybackLease {
                lease_id,
                expires_in_ms: 0,
            }
        );
        Ok(())
    }

    async fn start_ytdlp(&self, request: api::YtdlpRequest) -> Result<JobRef, ApiError> {
        let response = rpc!(self, start_ytdlp, convert::ytdlp_request_to_proto(&request));
        Ok(JobRef {
            job_id: response.get_ref().job_id.clone(),
        })
    }

    async fn catalog(&self, continuation: Option<String>) -> Result<api::CatalogPage, ApiError> {
        let response = rpc!(self, get_catalog, proto::CatalogRequest { continuation });
        Ok(convert::catalog_page_from_proto(response.get_ref()))
    }

    async fn catalog_detail(
        &self,
        request: api::CatalogDetailRequest,
    ) -> Result<api::CatalogDetail, ApiError> {
        let response = rpc!(
            self,
            get_catalog_detail,
            convert::catalog_detail_request_to_proto(&request)
        );
        Ok(convert::catalog_detail_from_proto(response.get_ref()))
    }

    async fn radio_stations(&self) -> Result<Vec<api::RadioStationInfo>, ApiError> {
        let response = rpc!(self, get_radio_stations, proto::Empty {});
        Ok(response
            .get_ref()
            .stations
            .iter()
            .map(convert::radio_station_from_proto)
            .collect())
    }

    async fn track_radio(&self, key: String) -> Result<Vec<api::TrackInfo>, ApiError> {
        let response = rpc!(self, start_track_radio, proto::TrackRef { key }).into_inner();
        Ok(response
            .tracks
            .iter()
            .map(convert::track_info_from_proto)
            .collect())
    }

    async fn playlist_radio(&self, id: String) -> Result<Vec<api::TrackInfo>, ApiError> {
        let response = rpc!(self, start_playlist_radio, proto::EntityRef { id }).into_inner();
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
        let response = rpc!(
            self,
            search_radio,
            proto::RadioSearchRequest { query, limit }
        );
        Ok(response
            .get_ref()
            .stations
            .iter()
            .map(convert::radio_station_from_proto)
            .collect())
    }

    async fn radio_registries(&self) -> Result<Vec<api::RadioRegistryInfo>, ApiError> {
        let response = rpc!(self, get_radio_registries, proto::Empty {});
        Ok(response
            .get_ref()
            .registries
            .iter()
            .map(convert::radio_registry_from_proto)
            .collect())
    }

    async fn add_radio_registry(&self, url: String) -> Result<(), ApiError> {
        rpc!(self, add_radio_registry, proto::RegistryRequest { url });
        Ok(())
    }

    async fn remove_radio_registry(&self, url: String) -> Result<(), ApiError> {
        rpc!(self, remove_radio_registry, proto::RegistryRequest { url });
        Ok(())
    }

    async fn set_radio_registry_enabled(&self, url: String, enabled: bool) -> Result<(), ApiError> {
        rpc!(
            self,
            set_radio_registry_enabled,
            proto::SetRegistryEnabledRequest { url, enabled }
        );
        Ok(())
    }

    async fn pin_radio_station(
        &self,
        station: api::RadioStationInfo,
        pinned: bool,
    ) -> Result<(), ApiError> {
        rpc!(
            self,
            pin_radio_station,
            proto::PinRadioStationRequest {
                station: Some(convert::radio_station_to_proto(&station)),
                pinned,
            }
        );
        Ok(())
    }

    async fn update_track_metadata(
        &self,
        patch: api::TrackMetadataPatch,
    ) -> Result<api::TrackInfo, ApiError> {
        let response = rpc!(
            self,
            update_track_metadata,
            convert::metadata_patch_to_proto(&patch)
        );
        Ok(convert::track_info_from_proto(response.get_ref()))
    }

    async fn delete_tracks(&self, keys: Vec<String>, from_disk: bool) -> Result<(), ApiError> {
        rpc!(
            self,
            delete_tracks,
            proto::DeleteTracksRequest { keys, from_disk }
        );
        Ok(())
    }

    async fn delete_album(&self, id: String, from_disk: bool) -> Result<(), ApiError> {
        rpc!(
            self,
            delete_album,
            proto::DeleteAlbumRequest { id, from_disk }
        );
        Ok(())
    }

    async fn upload_artwork(&self, upload: api::ArtworkUpload) -> Result<(), ApiError> {
        rpc!(
            self,
            upload_artwork,
            convert::artwork_upload_to_proto(&upload)
        );
        Ok(())
    }

    async fn remove_artwork(&self, target: api::ArtworkTarget) -> Result<(), ApiError> {
        rpc!(
            self,
            remove_artwork,
            convert::remove_artwork_to_proto(&target)
        );
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
