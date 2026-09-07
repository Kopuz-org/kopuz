#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Playing,
    Paused,
    Ended,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Intent {
    #[default]
    Stopped,
    Loading {
        token: u64,
        from_token: Option<u64>,
    },
    Committed {
        token: u64,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrackKind {
    #[default]
    Normal,
    Radio,
}

/// Now-playing summary. `key` and `uid` mean exactly what they do on
/// [`crate::TrackInfo`]: `key` is the library ref, and the entity id
/// `GetArtwork` takes; `uid` is the source-qualified identity.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowPlaying {
    pub key: String,
    pub uid: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
    pub khz: u32,
    pub bitrate: u16,
    pub kind: TrackKind,
    pub seekable: bool,
}

/// Position as an anchor, not a ticker: `ms` was correct at daemon-monotonic
/// time `at_ms`. Clients compute a clock offset from `PlayerState::now_ms`
/// once and interpolate locally while `playing` is true.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PositionAnchor {
    pub ms: u64,
    pub at_ms: u64,
    pub playing: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BufferedRange {
    pub start: u64,
    pub end: u64,
    pub total: Option<u64>,
}

/// The outgoing session during a crossfade. While present, frontends keep
/// displaying this track and drive the seek bar from `position_ms`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FadingState {
    pub from_token: u64,
    pub track: NowPlaying,
    pub position_ms: u64,
}

/// Playback happening outside the engine (Spotify in a browser).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExternalPlayback {
    pub kind: String,
    pub device: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueSummary {
    pub rev: u64,
    pub length: u32,
    pub index: Option<u32>,
    pub shuffle: bool,
    pub loop_mode: LoopMode,
}

/// The player snapshot (`GetPlayerState`) and the `player_state` event payload.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerState {
    pub rev: u64,
    pub now_ms: u64,
    pub phase: Phase,
    pub intent: Intent,
    pub track: Option<NowPlaying>,
    pub position: Option<PositionAnchor>,
    pub queue: QueueSummary,
    pub volume: f32,
    pub buffered: Vec<BufferedRange>,
    pub fading: Option<FadingState>,
    pub external: Option<ExternalPlayback>,
    pub error: Option<crate::error::ErrorBody>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
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
        shuffle: Option<bool>,
        loop_mode: Option<LoopMode>,
    },
}
