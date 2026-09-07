//! Wire types and the client-facing trait for the Kopuz daemon API.
//!
//! Everything here is transport-neutral: `LocalApi` (daemon crate) implements
//! [`KopuzApi`] with direct in-process calls; `GrpcApi` (client crate)
//! implements it over the gRPC wire. The protobuf schema in `kopuz-proto` is
//! the versioned wire contract.
//!
//! The trait starts with the playback core and grows one resource group at a
//! time as the daemon services land.

mod error;
mod events;
mod library;
mod player;
mod queue;

pub use error::{ApiError, ErrorBody, ErrorCode};
pub use events::{ApiEvent, JobKind, JobProgress, NoticeLevel, SourceState, Table};
pub use library::{
    DEFAULT_PAGE_LIMIT, LyricChunkView, LyricLineView, LyricsView, Page, StatsView, TrackFilter,
    TrackInfo, TrackPage, TrackSort,
};
pub use player::{
    BufferedRange, ExternalPlayback, FadingState, Intent, LoopMode, NowPlaying, Phase,
    PlayerCommand, PlayerState, PositionAnchor, QueueSummary, TrackKind,
};
pub use queue::{QueueContext, QueueEdit, QueueItem, QueueMode, QueueWindow, SetQueueRequest};

/// The config view: the layered config with credential keys
/// stripped, plus the keys a managed settings file pins (rendered locked in
/// settings UIs).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigView {
    pub config: serde_json::Value,
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

#[async_trait::async_trait]
pub trait KopuzApi: Send + Sync {
    async fn player_state(&self) -> Result<PlayerState, ApiError>;

    async fn player_command(&self, cmd: PlayerCommand) -> Result<CommandAck, ApiError>;

    async fn queue_window(&self, page: Page) -> Result<QueueWindow, ApiError>;

    async fn set_queue(&self, req: SetQueueRequest) -> Result<CommandAck, ApiError>;

    async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError>;

    async fn config(&self) -> Result<ConfigView, ApiError>;

    /// RFC 7396 merge patch against the config surface. Credential and
    /// locked keys are refused with `invalid_input`.
    async fn patch_config(&self, patch: serde_json::Value) -> Result<ConfigView, ApiError>;

    async fn favorites(&self) -> Result<FavoritesView, ApiError>;

    /// Optimistic set: recorded locally and reflected immediately, pushed to
    /// the remote in the background of the call; a rejected push reverts the
    /// local state and surfaces the error.
    async fn set_favorite(&self, key: String, favorite: bool) -> Result<(), ApiError>;

    /// Start a long-running job (`scan`, `library_sync`, `favorites_sync`).
    /// Progress arrives as `job.progress` / `job.finished` events; a second
    /// start of an already-running kind returns `conflict`.
    async fn start_job(&self, kind: JobKind) -> Result<JobRef, ApiError>;

    /// Cache server tracks for offline playback; returns the download job.
    async fn download(&self, keys: Vec<String>) -> Result<JobRef, ApiError>;

    /// Item ids with a registered offline copy.
    async fn downloads(&self) -> Result<Vec<String>, ApiError>;

    async fn remove_download(&self, key: String) -> Result<(), ApiError>;

    async fn jobs(&self) -> Result<Vec<JobStatus>, ApiError>;

    async fn cancel_job(&self, id: String) -> Result<(), ApiError>;

    async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError>;

    /// Local tracks under a directory prefix, path-ordered.
    async fn folder_tracks(&self, prefix: String, page: Page) -> Result<TrackPage, ApiError>;

    async fn lyrics(&self, key: String) -> Result<LyricsView, ApiError>;

    async fn stats(&self) -> Result<StatsView, ApiError>;

    /// Subscribe to the state stream. Every subscriber gets every event from
    /// the moment of subscription; a snapshot fetch plus this stream is the
    /// complete synchronization story.
    fn events(&self) -> EventStream;
}
