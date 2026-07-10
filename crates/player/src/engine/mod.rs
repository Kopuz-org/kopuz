//! Single-owner playback engine.
//!
//! A dedicated actor thread owns the output stream, the decode workers and all
//! session state. Callers talk to it exclusively through serialized [`Command`]s
//! tagged with a caller-supplied `token`; the engine answers with token-tagged
//! [`Event`]s and a lock-free [`EngineStatus`] snapshot. The real-time audio
//! callback owns its own state and never takes a lock (see `rt.rs`).

mod actor;
mod rt;
mod sink;
#[cfg(test)]
mod tests;
mod worker;

pub(crate) use actor::ActorMsg;
pub use actor::EngineHandle;
pub use sink::{AudioSink, CpalSink, DataCallback, DataCallbackFactory, SinkConfig, SinkEvent};

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use config::{ChannelMode, EqualizerSettings};
use symphonia::core::formats::probe::Hint;

/// Builds the media source on the decode worker thread, so slow constructions
/// (HTTP buffering, HLS assembly) never block the actor or an async executor.
pub type SourceFactory = Box<
    dyn FnOnce() -> Result<(Box<dyn symphonia::core::io::MediaSource>, Hint), String>
        + Send
        + 'static,
>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Immediate,
    Crossfade(Duration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Playing,
    Paused,
    Ended,
}

/// What actually happened when a load started playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadOutcome {
    /// False when a requested crossfade fell back to an immediate switch
    /// (config mismatch, idle/drained outgoing session, or paused).
    pub crossfaded: bool,
}

/// Resolved once the source is playing (`Ok`) or failed to load (`Err`).
/// Dropped without a send when the load is cancelled (superseded, stopped,
/// shutdown) — cancellation is not an error.
pub type LoadReply = tokio::sync::oneshot::Sender<Result<LoadOutcome, String>>;

pub struct LoadRequest {
    pub token: u64,
    pub factory: SourceFactory,
    pub duration: Duration,
    pub transition: Transition,
    pub start_at: Option<Duration>,
    pub reply: Option<LoadReply>,
}

pub enum Command {
    Load(LoadRequest),
    /// Drop a load that is still probing without touching the live session.
    CancelPending,
    /// Seek the visible session. `token`, when set, is the session the caller
    /// believes is visible; the engine drops the seek if a crossfade has since
    /// promoted a different session, so a scrub can't land on the wrong track.
    Seek {
        position: Duration,
        token: Option<u64>,
    },
    Pause,
    Resume,
    Stop {
        pause_device: bool,
    },
    SetVolume(f32),
    SetChannelMode(ChannelMode),
    SetEqualizer(EqualizerSettings),
    SetDeviceChangeBehavior(config::DeviceChangeBehavior),
    SetDuration(Duration),
    Subscribe(tokio::sync::mpsc::UnboundedSender<Event>),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Loaded {
        token: u64,
    },
    PhaseChanged {
        token: u64,
        phase: Phase,
    },
    Position {
        token: u64,
        position: Duration,
    },
    Ended {
        token: u64,
    },
    /// A crossfade finished and the outgoing session was torn down. `token` is
    /// the now-sole (incoming) session; `from_token` the retired outgoing one.
    /// This is the authoritative signal to commit a deferred crossfade UI.
    TrackSwitched {
        token: u64,
        from_token: u64,
    },
    Error {
        token: u64,
        message: String,
    },
}

/// The outgoing session during a crossfade. Exposed so the UI can render the
/// track it is still showing (the outgoing one) truthfully while the incoming
/// session's own position drives `EngineStatus::position`.
pub struct FadingStatus {
    pub token: u64,
    pub duration: Duration,
    pub(crate) base_micros: u64,
    pub(crate) played_samples: Arc<AtomicU64>,
}

/// Lock-free snapshot published by the actor. Position is exact on demand:
/// `base` is set at load/seek time and `played` is advanced by the audio
/// callback, so readers don't see tick-rate quantization.
pub struct EngineStatus {
    pub token: u64,
    pub phase: Phase,
    pub paused: bool,
    pub duration: Duration,
    /// A load is resolving (probe/network); its token. The current session
    /// keeps playing until it lands.
    pub pending_token: Option<u64>,
    /// A crossfade is mixing out the previous session. Cleared on
    /// `TrackSwitched` (fade completion) or when a seek cancels the fade.
    pub fading: Option<FadingStatus>,
    pub(crate) base_micros: u64,
    pub(crate) played_samples: Arc<AtomicU64>,
    pub(crate) channels: u32,
    pub(crate) sample_rate: u32,
}

/// Position from a base offset plus the samples the RT callback has played,
/// clamped to the track duration. Shared by the current and fading sessions.
fn position_from(
    base_micros: u64,
    played_samples: &AtomicU64,
    channels: u32,
    sample_rate: u32,
    duration: Duration,
) -> Duration {
    let played = played_samples.load(Ordering::Relaxed);
    let micros = if channels > 0 && sample_rate > 0 {
        base_micros + (played * 1_000_000) / (channels as u64 * sample_rate as u64)
    } else {
        base_micros
    };
    let raw = Duration::from_micros(micros);
    if duration > Duration::ZERO && raw > duration {
        duration
    } else {
        raw
    }
}

impl EngineStatus {
    pub(crate) fn idle() -> Self {
        Self {
            token: 0,
            phase: Phase::Idle,
            paused: false,
            duration: Duration::ZERO,
            pending_token: None,
            fading: None,
            base_micros: 0,
            played_samples: Arc::new(AtomicU64::new(0)),
            channels: 0,
            sample_rate: 0,
        }
    }

    /// True while a load is resolving or a crossfade is mixing out — the single
    /// source of truth the UI gates its transition handling on.
    pub fn transition_in_flight(&self) -> bool {
        self.pending_token.is_some() || self.fading.is_some()
    }

    pub fn position(&self) -> Duration {
        if self.phase == Phase::Idle {
            return Duration::ZERO;
        }
        position_from(
            self.base_micros,
            &self.played_samples,
            self.channels,
            self.sample_rate,
            self.duration,
        )
    }

    /// The outgoing session's live position during a crossfade. Uses the
    /// current stream config — a crossfade requires an identical one by
    /// construction (the fade only starts when the configs match).
    pub fn fading_position(&self) -> Option<Duration> {
        let fading = self.fading.as_ref()?;
        Some(position_from(
            fading.base_micros,
            &fading.played_samples,
            self.channels,
            self.sample_rate,
            fading.duration,
        ))
    }
}
