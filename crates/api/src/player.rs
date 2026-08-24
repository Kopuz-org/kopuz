use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Playing,
    Paused,
    Ended,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    #[default]
    None,
    Queue,
    Track,
}

impl LoopMode {
    /// The loop-toggle cycle, matching the current UI behavior.
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::Queue,
            Self::Queue => Self::Track,
            Self::Track => Self::None,
        }
    }
}

/// What the daemon is trying to do, as distinct from [`Phase`] (engine truth).
/// Frontends render optimistic UI from the intent, exactly like the current
/// app's `is_playing` blend, but the blend is defined daemon-side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Intent {
    #[default]
    Stopped,
    Loading {
        token: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_token: Option<u64>,
    },
    Committed {
        token: u64,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    #[default]
    Normal,
    Radio,
}

/// Now-playing summary. `key` is the track's stable library ref; `artwork` is
/// a daemon-relative URL, never a filesystem path or a credentialed remote
/// URL.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NowPlaying {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
    pub khz: u32,
    pub bitrate: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork: Option<String>,
    pub kind: TrackKind,
    pub seekable: bool,
}

/// Position as an anchor, not a ticker: `ms` was correct at daemon-monotonic
/// time `at_ms`. Clients compute a clock offset from `PlayerState::now_ms`
/// once and interpolate locally while `playing` is true.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionAnchor {
    pub ms: u64,
    pub at_ms: u64,
    pub playing: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferedRange {
    pub start: u64,
    pub end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// The outgoing session during a crossfade. While present, frontends keep
/// displaying this track and drive the seek bar from `position_ms`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FadingState {
    pub from_token: u64,
    pub track: NowPlaying,
    pub position_ms: u64,
}

/// Playback happening outside the engine (Spotify in a browser).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExternalPlayback {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueSummary {
    pub rev: u64,
    pub length: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    pub shuffle: bool,
    #[serde(rename = "loop")]
    pub loop_mode: LoopMode,
}

/// The `GET /v1/player` snapshot and the `player.state` SSE payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub rev: u64,
    pub now_ms: u64,
    pub phase: Phase,
    pub intent: Intent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<NowPlaying>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<PositionAnchor>,
    pub queue: QueueSummary,
    pub volume: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buffered: Vec<BufferedRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fading: Option<FadingState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalPlayback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlayerCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    Seek {
        position_ms: u64,
    },
    SetVolume {
        volume: f32,
    },
    SetMode {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shuffle: Option<bool>,
        #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
        loop_mode: Option<LoopMode>,
    },
}
