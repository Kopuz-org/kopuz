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
mod error;
mod events;
mod library;
mod player;
mod playlists;
mod queue;

pub use artwork::{ArtworkData, ArtworkRequest, ArtworkTarget};
pub use error::{ApiError, ErrorBody, ErrorCode};
pub use events::{ApiEvent, JobKind, JobProgress, NoticeLevel, SourceState, Table};
pub use library::{
    AlbumInfo, AlbumPage, ArtistInfo, ArtistPage, DEFAULT_PAGE_LIMIT, LyricChunkView,
    LyricLineView, LyricsView, Page, SearchResults, StatsView, TrackFilter, TrackInfo, TrackPage,
    TrackSort,
};
pub use player::{
    BufferedRange, ExternalPlayback, FadingState, Intent, LoopMode, NowPlaying, Phase,
    PlayerCommand, PlayerState, PositionAnchor, QueueSummary, TrackKind,
};
pub use playlists::{PlaylistCatalog, PlaylistFolderInfo, PlaylistInfo, PlaylistReorder};
pub use queue::{QueueContext, QueueEdit, QueueItem, QueueMode, QueueWindow, SetQueueRequest};

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

    async fn set_queue(&self, req: SetQueueRequest) -> Result<CommandAck, ApiError>;

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError>;
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

    async fn lyrics(&self, key: String) -> Result<LyricsView, ApiError>;

    async fn stats(&self) -> Result<StatsView, ApiError>;

    async fn favorites(&self) -> Result<FavoritesView, ApiError>;

    /// Optimistic set: recorded locally and reflected immediately, pushed to
    /// the remote in the background of the call; a rejected push reverts the
    /// local state and surfaces the error.
    async fn set_favorite(&self, key: String, favorite: bool) -> Result<(), ApiError>;
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

    /// Make `source` the active one. Separate from [`Self::set_config`]
    /// because switching to a server means loading that server's stored
    /// credentials into the active snapshot, and those are the fields
    /// `set_config` deliberately refuses to take from a caller.
    ///
    /// Answers whether the new source is usable: a server with no stored
    /// credentials still becomes active, but the caller has to prompt a
    /// sign-in rather than showing an empty library.
    async fn switch_source(&self, source: config::Source) -> Result<bool, ApiError>;
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
    PlayerApi + LibraryApi + PlaylistApi + ArtworkApi + JobApi + ConfigApi + EventApi + Send + Sync
{
}

impl<T> KopuzApi for T where
    T: PlayerApi
        + LibraryApi
        + PlaylistApi
        + ArtworkApi
        + JobApi
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
    };
}
