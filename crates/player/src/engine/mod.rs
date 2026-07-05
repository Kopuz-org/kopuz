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
pub use sink::{AudioSink, CpalSink, DataCallback, DataCallbackFactory, SinkConfig};

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

pub struct LoadRequest {
    pub token: u64,
    pub factory: SourceFactory,
    pub duration: Duration,
    pub transition: Transition,
    pub start_at: Option<Duration>,
    /// Present while the façade's blocking `play()` bridge is in use; resolved
    /// once the source is playing (or failed to load).
    pub reply: Option<std::sync::mpsc::Sender<Result<(), String>>>,
}

pub enum Command {
    Load(LoadRequest),
    Seek(Duration),
    Pause,
    Resume,
    Stop { pause_device: bool },
    SetVolume(f32),
    SetChannelMode(ChannelMode),
    SetEqualizer(EqualizerSettings),
    SetDuration(Duration),
    SetFinishCallback(Arc<dyn Fn() + Send + Sync + 'static>),
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
    /// A crossfade finished and the outgoing session was torn down.
    TrackSwitched {
        token: u64,
    },
    Error {
        token: u64,
        message: String,
    },
}

/// Lock-free snapshot published by the actor. Position is exact on demand:
/// `base` is set at load/seek time and `played` is advanced by the audio
/// callback, so readers don't see tick-rate quantization.
pub struct EngineStatus {
    pub token: u64,
    pub phase: Phase,
    pub paused: bool,
    pub duration: Duration,
    base_micros: u64,
    played_samples: Arc<AtomicU64>,
    channels: u32,
    sample_rate: u32,
}

impl EngineStatus {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        token: u64,
        phase: Phase,
        paused: bool,
        duration: Duration,
        base_micros: u64,
        played_samples: Arc<AtomicU64>,
        channels: u32,
        sample_rate: u32,
    ) -> Self {
        Self {
            token,
            phase,
            paused,
            duration,
            base_micros,
            played_samples,
            channels,
            sample_rate,
        }
    }

    pub(crate) fn idle() -> Self {
        Self {
            token: 0,
            phase: Phase::Idle,
            paused: false,
            duration: Duration::ZERO,
            base_micros: 0,
            played_samples: Arc::new(AtomicU64::new(0)),
            channels: 0,
            sample_rate: 0,
        }
    }

    pub fn position(&self) -> Duration {
        if self.phase == Phase::Idle {
            return Duration::ZERO;
        }
        let played = self.played_samples.load(Ordering::Relaxed);
        let micros = if self.channels > 0 && self.sample_rate > 0 {
            self.base_micros
                + (played * 1_000_000) / (self.channels as u64 * self.sample_rate as u64)
        } else {
            self.base_micros
        };
        let raw = Duration::from_micros(micros);
        if self.duration > Duration::ZERO && raw > self.duration {
            self.duration
        } else {
            raw
        }
    }
}
