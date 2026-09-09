//! Wire types and the client-facing trait for the Kopuz daemon API.
//!
//! Everything here is transport-neutral: `LocalApi` (daemon crate) implements
//! [`KopuzApi`] with direct in-process calls; `GrpcApi` (client crate)
//! implements it over the gRPC wire. The protobuf schema in `kopuz-proto` is
//! the versioned wire contract.
//!
//! The trait starts with the playback core and grows one resource group at a
//! time as the daemon services land.

mod artwork;
mod catalog;
mod error;
mod events;
mod jobs;
mod library;
mod mutations;
mod player;
mod playlists;
mod queue;
mod radio;
pub mod schema;
mod sources;

pub use artwork::{ArtworkData, ArtworkRef, ArtworkRequest, ArtworkTarget};
pub use catalog::{
    CatalogDetail, CatalogDetailRequest, CatalogItem, CatalogItemKind, CatalogPage, CatalogShelf,
};
pub use error::{ApiError, ErrorBody, ErrorCode};
pub use events::{ApiEvent, JobKind, JobProgress, NoticeLevel, SourceState, Table};
pub use jobs::{DownloadHistoryEntry, DownloadItemState, DownloadItemStatus, DownloadState};
pub use library::{
    AlbumInfo, AlbumPage, ArtistInfo, ArtistPage, DEFAULT_PAGE_LIMIT, LyricChunkView,
    LyricLineView, LyricsView, Page, SearchResults, StatsView, TrackFilter, TrackInfo, TrackPage,
    TrackSort,
};
pub use mutations::{ArtworkChange, ArtworkUpload, TrackMetadataPatch};
pub use player::{
    BufferedRange, ExternalDevice, ExternalPlayback, FadingState, Intent, LoopMode, NowPlaying,
    Phase, PlayerCommand, PlayerState, PositionAnchor, QueueSummary, TrackKind,
};
pub use playlists::{PlaylistCatalog, PlaylistFolderInfo, PlaylistInfo, PlaylistReorder};
pub use queue::{
    QueueContext, QueueEdit, QueueItem, QueueMode, QueueSnapshot, QueueWindow, SetQueueRequest,
};
pub use radio::{RadioStationInfo, RadioStreamInfo};
pub use schema::{
    ChoiceOption, FieldKind, FieldSpec, FieldValue, Icon, Problem, Text, spec_value, toggle_of,
    value_of,
};
pub use sources::{
    AlbumPresentation, ArtistPresentation, ConnectKind, CredentialProvision, DraftCheck,
    FavoritesSyncMode, IntegrationInfo, LocalSourceDraft, PlaylistCapability, ServerDraft,
    ServiceInfo, ServiceRef, SignInKind, SourceCapabilities, SourceFolderEntry, SourceInfo,
    SourceKind, SourceLoginRequest,
};

/// The config view: the layered config with credential keys
/// stripped, plus the keys a managed settings file pins (rendered locked in
/// settings UIs).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigView {
    pub config: config::AppConfig,
    pub locked_keys: Vec<String>,
}

/// Returned by every command; `rev` names the state revision that includes
/// the command's effect, so a client can wait for the event stream to catch
/// up before trusting its local mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandAck {
    pub rev: u64,
}

pub type EventStream = futures_util::stream::BoxStream<'static, ApiEvent>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRef {
    pub job_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Finished,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobStatus {
    pub id: String,
    pub kind: JobKind,
    pub state: JobState,
    pub phase: String,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub message: Option<String>,
    pub error: Option<ErrorBody>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FavoritesView {
    pub refs: Vec<String>,
    pub generation: u64,
}

/// Transport and queue.
#[async_trait::async_trait]
pub trait PlayerApi: Send + Sync {
    async fn player_state(&self) -> Result<PlayerState, ApiError>;

    async fn player_command(&self, cmd: PlayerCommand) -> Result<CommandAck, ApiError>;

    async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError>;

    /// The whole queue, including the shuffle permutation. A frontend that
    /// mirrors the queue reads this once and then follows `queue.changed`.
    async fn queue_snapshot(&self) -> Result<QueueSnapshot, ApiError>;

    async fn set_queue(&self, req: SetQueueRequest) -> Result<CommandAck, ApiError>;

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError>;

    /// Where a source can play on its own: the Connect devices, the phones,
    /// the speakers. Empty unless its capabilities say it has them and it is
    /// signed in.
    async fn external_devices(&self, source_id: String) -> Result<Vec<ExternalDevice>, ApiError>;

    /// Move that source's playback to one of those devices, or back to this
    /// app's own with `None`.
    async fn select_external_device(
        &self,
        source_id: String,
        device_id: Option<String>,
    ) -> Result<(), ApiError>;
}

/// Reading the library, and the per-track state that belongs to it.
#[async_trait::async_trait]
pub trait LibraryApi: Send + Sync {
    async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError>;

    /// Local tracks under a directory prefix, path-ordered.
    async fn folder_tracks(&self, prefix: String, page: Page) -> Result<TrackPage, ApiError>;

    /// Rows for specific keys, in the order asked for. Keys the library does
    /// not hold are skipped rather than erroring.
    async fn tracks_by_keys(&self, keys: Vec<String>) -> Result<Vec<TrackInfo>, ApiError>;

    async fn albums(&self, page: Page) -> Result<AlbumPage, ApiError>;

    async fn album(&self, id: String) -> Result<Option<AlbumInfo>, ApiError>;

    async fn album_tracks(&self, id: String, page: Page) -> Result<TrackPage, ApiError>;

    async fn artists(&self, page: Page) -> Result<ArtistPage, ApiError>;

    async fn artist_tracks(&self, artist: String, page: Page) -> Result<TrackPage, ApiError>;

    /// One track per artist, for the artist grid's tiles.
    async fn artist_sample_tracks(&self, page: Page) -> Result<TrackPage, ApiError>;

    async fn genres(&self) -> Result<Vec<String>, ApiError>;

    /// The genre with the most tracks, for the home page's heading.
    async fn top_genre(&self) -> Result<Option<String>, ApiError>;

    async fn genre_tracks(&self, genre: String, page: Page) -> Result<TrackPage, ApiError>;

    /// Most recently played first.
    async fn recent_tracks(&self, page: Page) -> Result<TrackPage, ApiError>;

    /// Search the active source. Remote sources answer over the network, so
    /// this is a daemon call and not a filter the caller composes.
    async fn search(&self, query: String) -> Result<SearchResults, ApiError>;

    /// The source's public page for a row, for a share action. `None` when the
    /// source has no web pages, which is a client's cue to fall back to a
    /// metadata lookup rather than to build a URL from a service name.
    async fn track_web_url(&self, key: String) -> Result<Option<String>, ApiError>;

    /// The same for an album, by the id it is browsed under.
    async fn album_web_url(&self, id: String) -> Result<Option<String>, ApiError>;

    /// The source's browse feed. `continuation` pages it; `None` starts over.
    async fn catalog(&self, continuation: Option<String>) -> Result<CatalogPage, ApiError>;

    /// Open one catalog entity. Tracks it returns are registered, so a client
    /// can queue them by key like any others.
    async fn catalog_detail(
        &self,
        request: CatalogDetailRequest,
    ) -> Result<CatalogDetail, ApiError>;

    /// Every station the configured registries hold, pinned ones marked.
    async fn radio_stations(&self) -> Result<Vec<RadioStationInfo>, ApiError>;

    /// Search the public station directory. Hits join the live registry, so
    /// one can be played by id straight afterwards.
    async fn search_radio(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<RadioStationInfo>, ApiError>;

    /// Pin a station so it survives a registry refresh and appears first.
    async fn pin_radio_station(&self, id: String, pinned: bool) -> Result<(), ApiError>;

    /// Check that a URL really is a station registry, and say how many
    /// stations it holds. A client writes the registry list into its config;
    /// this is how it can refuse a bad URL without fetching one itself.
    async fn validate_radio_registry(&self, url: String) -> Result<u32, ApiError>;

    /// Look for photos for these artists, storing what it finds. Names already
    /// resolved, and names whose last search definitively found nothing, are
    /// skipped -- so calling it on every visit is cheap. Results arrive as a
    /// `Tracks` invalidation, not in the answer.
    async fn refresh_artist_artwork(&self, names: Vec<String>) -> Result<(), ApiError>;

    async fn lyrics(&self, key: String) -> Result<LyricsView, ApiError>;

    async fn stats(&self) -> Result<StatsView, ApiError>;

    async fn favorites(&self) -> Result<FavoritesView, ApiError>;

    /// Optimistic set: recorded locally and reflected immediately, pushed to
    /// the remote in the background of the call; a rejected push reverts the
    /// local state and surfaces the error.
    async fn set_favorite(&self, key: String, favorite: bool) -> Result<(), ApiError>;

    /// Rewrite one track's tags, and its embedded cover with them. Only local
    /// files have tags to edit; a server track answers `unsupported`.
    async fn update_track_metadata(&self, patch: TrackMetadataPatch)
    -> Result<TrackInfo, ApiError>;

    /// Forget these tracks. `from_disk` also unlinks the files, which is
    /// refused for anything outside the configured library roots.
    async fn delete_tracks(&self, keys: Vec<String>, from_disk: bool) -> Result<(), ApiError>;

    async fn delete_album(&self, id: String, from_disk: bool) -> Result<(), ApiError>;

    /// Set a picture for an album, artist, playlist or track. The daemon
    /// stores it and tells the source, so a server that hosts covers gets it.
    async fn upload_artwork(&self, upload: ArtworkUpload) -> Result<(), ApiError>;

    async fn remove_artwork(&self, target: ArtworkTarget) -> Result<(), ApiError>;
}

/// Playlists and the folders they sit in.
///
/// Every mutation goes through the active source, so a server playlist is
/// pushed to the server and a local one is not, without the caller knowing
/// which it holds. The daemon reports the change as a `Playlists` (or
/// `Folders`) invalidation.
#[async_trait::async_trait]
pub trait PlaylistApi: Send + Sync {
    async fn playlists(&self) -> Result<PlaylistCatalog, ApiError>;

    /// Create a playlist and return its id.
    async fn create_playlist(&self, name: String, keys: Vec<String>) -> Result<String, ApiError>;

    async fn rename_playlist(&self, id: String, name: String) -> Result<(), ApiError>;

    async fn delete_playlist(&self, id: String) -> Result<(), ApiError>;

    async fn add_playlist_tracks(&self, id: String, keys: Vec<String>) -> Result<(), ApiError>;

    /// Remove one entry by position. Position rather than key, because a
    /// playlist may hold the same track twice.
    async fn remove_playlist_track(&self, id: String, index: u32) -> Result<(), ApiError>;

    async fn reorder_playlist(&self, id: String, reorder: PlaylistReorder) -> Result<(), ApiError>;

    /// Pull a server playlist's contents again. A no-op for a local one.
    async fn refresh_playlist(&self, id: String) -> Result<(), ApiError>;

    async fn create_playlist_folder(&self, name: String) -> Result<String, ApiError>;

    async fn rename_playlist_folder(&self, id: String, name: String) -> Result<(), ApiError>;

    async fn delete_playlist_folder(&self, id: String) -> Result<(), ApiError>;

    /// Move a playlist into a folder, or out of every folder with `None`.
    async fn move_playlist(
        &self,
        playlist_id: String,
        folder_id: Option<String>,
    ) -> Result<(), ApiError>;
}

/// Cover bytes for a library entity. Clients ask by id rather than resolving
/// a URL themselves: the daemon holds the credentials that sign server cover
/// URLs, and they never reach the wire.
#[async_trait::async_trait]
pub trait ArtworkApi: Send + Sync {
    async fn artwork(&self, request: ArtworkRequest) -> Result<ArtworkData, ApiError>;
}

/// Long-running work and the offline cache.
#[async_trait::async_trait]
pub trait JobApi: Send + Sync {
    /// Start a long-running job (`scan`, `library_sync`, `favorites_sync`).
    /// Progress arrives as `job.progress` / `job.finished` events; a second
    /// start of an already-running kind returns `conflict`.
    async fn start_job(&self, kind: JobKind) -> Result<JobRef, ApiError>;

    async fn jobs(&self) -> Result<Vec<JobStatus>, ApiError>;

    async fn cancel_job(&self, id: String) -> Result<(), ApiError>;

    /// Cache server tracks for offline playback; returns the download job.
    async fn download(&self, keys: Vec<String>) -> Result<JobRef, ApiError>;

    /// Item ids with a registered offline copy.
    async fn downloads(&self) -> Result<Vec<String>, ApiError>;

    async fn remove_download(&self, key: String) -> Result<(), ApiError>;

    /// Per-item state for the downloads a batch is working through.
    async fn download_statuses(&self) -> Result<Vec<DownloadItemStatus>, ApiError>;

    /// Fetch a URL to a file. What tool does it, and every option it is given
    /// beyond the format, is the daemon's; progress arrives as job events and
    /// the outcome joins [`Self::downloader_history`].
    async fn download_url(&self, url: String, format: String) -> Result<JobRef, ApiError>;

    /// The formats a download can be asked for, picked per download rather
    /// than kept in the settings.
    async fn download_formats(&self) -> Result<Vec<ChoiceOption>, ApiError>;

    /// The downloader's own options, with the values it currently has.
    async fn downloader_settings(&self) -> Result<Vec<FieldSpec>, ApiError>;

    async fn set_downloader_settings(
        &self,
        values: Vec<FieldValue>,
    ) -> Result<Vec<FieldSpec>, ApiError>;

    async fn downloader_history(&self) -> Result<Vec<DownloadHistoryEntry>, ApiError>;

    async fn clear_downloader_history(&self) -> Result<(), ApiError>;
}

/// What is configured to play from, and what is signed into.
///
/// A frontend never builds a media source or holds a credential. It reads
/// these rows to decide what to offer, and calls these methods to change what
/// is configured; every secret stays behind the seam. Browser sign-in belongs
/// here for the same reason: it spawns a browser and ends holding a token.
#[async_trait::async_trait]
pub trait SourceApi: Send + Sync {
    async fn sources(&self) -> Result<Vec<SourceInfo>, ApiError>;

    /// Every service this daemon can be pointed at, each with the form that
    /// adds one. A client renders these rather than knowing any of them.
    async fn services(&self) -> Result<Vec<ServiceInfo>, ApiError>;

    /// What a draft would do if it were saved, and what is wrong with it.
    /// Cheap enough to call as a form is typed into.
    async fn check_server_draft(&self, draft: ServerDraft) -> Result<DraftCheck, ApiError>;

    /// Answer a source's own options. Absent keys are left alone, and an empty
    /// secret is not a request to clear one.
    async fn set_source_settings(
        &self,
        id: String,
        values: Vec<FieldValue>,
    ) -> Result<SourceInfo, ApiError>;

    /// Make one active. Returns the source as it now stands, including
    /// whether it is usable -- a server with no credentials still becomes
    /// active, so the caller prompts a sign-in rather than showing an empty
    /// library.
    async fn switch_source(&self, id: String) -> Result<SourceInfo, ApiError>;

    /// Create or update a local library. Absent `id` creates.
    async fn upsert_local_source(&self, draft: LocalSourceDraft) -> Result<SourceInfo, ApiError>;

    async fn delete_local_source(&self, id: String) -> Result<(), ApiError>;

    /// Replace a source's scan roots, or a server's selected folders.
    async fn set_source_directories(
        &self,
        id: String,
        directories: Vec<String>,
    ) -> Result<SourceInfo, ApiError>;

    async fn upsert_server(&self, draft: ServerDraft) -> Result<SourceInfo, ApiError>;

    async fn delete_server(&self, id: String) -> Result<(), ApiError>;

    /// Store a secret obtained elsewhere. Write-only.
    async fn provision_credentials(
        &self,
        provision: CredentialProvision,
    ) -> Result<SourceInfo, ApiError>;

    /// Sign in with a username and password, for servers that take one.
    async fn login_source(&self, request: SourceLoginRequest) -> Result<SourceInfo, ApiError>;

    async fn clear_credentials(&self, id: String) -> Result<(), ApiError>;

    /// Run this source's browser sign-in and keep the result. The caller
    /// learns that the source is authenticated, never with what.
    async fn authenticate_source(&self, id: String) -> Result<SourceInfo, ApiError>;

    /// List folders on a server that has them, for a folder picker.
    async fn browse_source(
        &self,
        id: String,
        path: String,
    ) -> Result<Vec<SourceFolderEntry>, ApiError>;

    /// Probe whether a source is reachable and still signed in.
    async fn validate_source(&self, id: String) -> Result<SourceState, ApiError>;

    /// Whether this daemon can run a browser sign-in at all. A sandboxed
    /// daemon cannot spawn one, and a client asks before offering a source
    /// whose only sign-in is a browser one.
    async fn can_open_browser(&self) -> Result<bool, ApiError>;

    /// Everything configured per account rather than per source, each with the
    /// fields that configure it.
    async fn integrations(&self) -> Result<Vec<IntegrationInfo>, ApiError>;

    /// Answer an integration's fields. A secret is write-only, and an empty
    /// one leaves what is stored alone.
    async fn set_integration_settings(
        &self,
        id: String,
        values: Vec<FieldValue>,
    ) -> Result<IntegrationInfo, ApiError>;

    async fn clear_integration(&self, id: String) -> Result<(), ApiError>;

    /// Run a service's web sign-in and keep the session key it returns.
    async fn authenticate_integration(&self, id: String) -> Result<IntegrationInfo, ApiError>;
}

/// The settings surface.
#[async_trait::async_trait]
pub trait ConfigApi: Send + Sync {
    async fn config(&self) -> Result<ConfigView, ApiError>;

    /// Replace the settings surface. Read [`Self::config`], change what you
    /// want, send it back. Credential fields are ignored (the daemon keeps
    /// its own), and a locked key whose value actually differs is refused
    /// with `invalid_input`.
    async fn set_config(&self, config: config::AppConfig) -> Result<ConfigView, ApiError>;

    /// Hear an equalizer setting without keeping it. The engine applies it
    /// live; nothing is written, so cancelling a preview is doing nothing.
    async fn preview_equalizer(&self, equalizer: config::EqualizerSettings)
    -> Result<(), ApiError>;

    // Switching sources lives on `SourceApi`, which is where sources are.
}

/// Subscribe to the state stream. Every subscriber gets every event from the
/// moment of subscription; a snapshot fetch plus this stream is the complete
/// synchronization story.
pub trait EventApi: Send + Sync {
    fn events(&self) -> EventStream;
}

/// Everything a frontend can ask of a daemon.
///
/// Split by domain so each implementor writes one `impl` block per group
/// instead of one that grows without bound; consumers still hold a single
/// `Arc<dyn KopuzApi>` and call any method on it.
pub trait KopuzApi:
    PlayerApi
    + LibraryApi
    + PlaylistApi
    + ArtworkApi
    + JobApi
    + SourceApi
    + ConfigApi
    + EventApi
    + Send
    + Sync
{
}

impl<T> KopuzApi for T where
    T: PlayerApi
        + LibraryApi
        + PlaylistApi
        + ArtworkApi
        + JobApi
        + SourceApi
        + ConfigApi
        + EventApi
        + Send
        + Sync
        + ?Sized
{
}

/// The sub-traits, for code holding a concrete implementor rather than a
/// `dyn KopuzApi` -- a method is only callable with its own trait in scope.
pub mod prelude {
    pub use super::{
        ArtworkApi, ConfigApi, EventApi, JobApi, KopuzApi, LibraryApi, PlayerApi, PlaylistApi,
        SourceApi,
    };
}
