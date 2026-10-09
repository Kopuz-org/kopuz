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
    pub track: crate::TrackInfo,
    pub position_ms: u64,
}

/// Playback happening outside the engine (Spotify in a browser).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExternalPlayback {
    pub kind: String,
    pub device: Option<String>,
}

/// One place an integration can play: a Connect speaker, a phone, another
/// desktop. `active` marks the one it is playing on now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalDevice {
    pub id: String,
    pub name: String,
    /// What the service calls it: "Smartphone", "Speaker", "Computer".
    pub kind: String,
    /// A glyph for that kind, chosen by the daemon so a client need not know
    /// one service's device vocabulary.
    pub icon: crate::schema::Icon,
    pub active: bool,
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
    /// The whole row, the same one the queue holds, so nothing has to be looked up there.
    pub track: Option<crate::TrackInfo>,
    pub position: Option<PositionAnchor>,
    pub queue: QueueSummary,
    pub volume: f32,
    pub buffered: Vec<BufferedRange>,
    pub fading: Option<FadingState>,
    pub external: Option<ExternalPlayback>,
    pub error: Option<crate::error::ErrorBody>,
    /// Output pipeline latency, for lyric/visual sync offsets.
    pub output_latency_ms: Option<u64>,
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
    /// Swap the playing track for its other cut, at the same place in the
    /// song. Only the swapped queue item changes; a track with no
    /// [`crate::TrackCounterpart`] of that version answers `invalid_input`.
    SetVersion {
        version: crate::TrackVersion,
    },
}

/// A byte range of a music video's picture, for [`crate::PlayerApi::video`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoRequest {
    /// A queued track whose version is [`crate::TrackVersion::Video`].
    pub key: String,
    pub start: u64,
    /// At most this many bytes; the daemon may send fewer.
    pub length: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoChunk {
    pub content_type: String,
    pub start: u64,
    /// The whole stream's length, when the source says.
    pub total: Option<u64>,
    pub bytes: Vec<u8>,
}
