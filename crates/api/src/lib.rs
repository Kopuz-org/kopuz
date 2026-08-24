//! Wire types and the client-facing trait for the Kopuz daemon API.
//!
//! Everything here is transport-neutral: `LocalApi` (daemon crate) implements
//! [`KopuzApi`] with direct in-process calls; `HttpApi` (client crate)
//! implements it over HTTP/JSON + SSE. The JSON shapes of these types are the
//! protocol; a breaking change to a shipped shape requires a new API version.
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
pub use library::{DEFAULT_PAGE_LIMIT, Page, TrackFilter, TrackPage};
pub use player::{
    BufferedRange, ExternalPlayback, FadingState, Intent, LoopMode, NowPlaying, Phase,
    PlayerCommand, PlayerState, PositionAnchor, QueueSummary, TrackKind,
};
pub use queue::{QueueContext, QueueEdit, QueueItem, QueueMode, QueueWindow, SetQueueRequest};

pub const API_VERSION: u32 = 1;

/// The `GET /v1/config` payload: the layered config with credential keys
/// stripped, plus the keys a managed settings file pins (rendered locked in
/// settings UIs).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConfigView {
    pub config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked_keys: Vec<String>,
}

/// Returned by every command; `rev` names the state revision that includes
/// the command's effect, so a client can wait for the event stream to catch
/// up before trusting its local mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommandAck {
    pub rev: u64,
}

pub type EventStream = futures_util::stream::BoxStream<'static, ApiEvent>;

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

    async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError>;

    /// Subscribe to the state stream. Every subscriber gets every event from
    /// the moment of subscription; a snapshot fetch plus this stream is the
    /// complete synchronization story.
    fn events(&self) -> EventStream;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_map_to_statuses() {
        assert_eq!(ErrorCode::InvalidInput.http_status(), 400);
        assert_eq!(ErrorCode::SourceAuthExpired.http_status(), 401);
        assert_eq!(ErrorCode::Unsupported.http_status(), 501);
        assert_eq!(ErrorCode::SourceUnreachable.http_status(), 502);
    }

    #[test]
    fn api_event_serializes_with_dotted_names() {
        let event = ApiEvent::LibraryInvalidated {
            table: Table::Favorites,
            generation: 42,
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["event"], "library.invalidated");
        assert_eq!(json["data"]["table"], "favorites");
        assert_eq!(json["data"]["generation"], 42);
        let back: ApiEvent = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, event);
    }

    #[test]
    fn resync_serializes_without_a_data_key() {
        let json = serde_json::to_value(&ApiEvent::Resync).expect("serialize");
        assert_eq!(json["event"], "resync");
        assert!(json.get("data").is_none());
        let back: ApiEvent = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, ApiEvent::Resync);
    }

    #[test]
    fn set_mode_round_trips_with_the_loop_field() {
        let command = PlayerCommand::SetMode {
            shuffle: Some(true),
            loop_mode: Some(LoopMode::Track),
        };
        let json = serde_json::to_value(command).expect("serialize");
        assert_eq!(json["type"], "set_mode");
        assert_eq!(json["loop"], "track");
        let back: PlayerCommand = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, command);
    }

    #[test]
    fn unknown_enum_values_fall_back_instead_of_failing() {
        let code: ErrorCode = serde_json::from_value("brand_new_code".into()).expect("code");
        assert_eq!(code, ErrorCode::Internal);
        let table: Table = serde_json::from_value("brand_new_table".into()).expect("table");
        assert_eq!(table, Table::Unknown);
    }

    #[test]
    fn player_state_round_trips() {
        let state = PlayerState {
            rev: 7,
            now_ms: 1234,
            phase: Phase::Playing,
            intent: Intent::Committed { token: 3 },
            volume: 0.8,
            queue: QueueSummary {
                rev: 2,
                length: 10,
                index: Some(4),
                shuffle: true,
                loop_mode: LoopMode::Queue,
            },
            ..Default::default()
        };
        let json = serde_json::to_value(&state).expect("serialize");
        assert_eq!(json["intent"]["kind"], "committed");
        assert_eq!(json["queue"]["loop"], "queue");
        let back: PlayerState = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, state);
    }
}
