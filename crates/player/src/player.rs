//! Public playback handle.
//!
//! `Player` is a thin façade over the engine actor (`crate::engine`): methods
//! send serialized commands; reads come from the actor's lock-free
//! `EngineStatus` snapshot. No audio state lives on the caller's thread.

use std::sync::Arc;
use std::time::Duration;

use config::{ChannelMode, EqualizerSettings};

use crate::engine::{
    ActorMsg, AudioSink, Command, CpalSink, EngineHandle, EngineStatus, Event, LoadReply,
    LoadRequest, Phase, SourceFactory, Transition,
};
#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "windows",
    target_os = "android"
))]
use crate::systemint;

pub struct NowPlayingMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    pub artwork: Option<String>,
}

#[derive(Debug)]
pub enum PlayerInitError {
    NoOutputDevice,
    DefaultOutputConfig(cpal::DefaultStreamConfigError),
    BuildOutputStream(cpal::BuildStreamError),
    StartOutputStream(cpal::PlayStreamError),
}

impl std::fmt::Display for PlayerInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoOutputDevice => f.write_str("no output device available"),
            Self::DefaultOutputConfig(e) => write!(f, "no default output config: {e}"),
            Self::BuildOutputStream(e) => write!(f, "failed to build output stream: {e}"),
            Self::StartOutputStream(e) => write!(f, "failed to start output stream: {e}"),
        }
    }
}

impl std::error::Error for PlayerInitError {}

/// Everything the engine needs to start playing a new source.
pub struct LoadArgs {
    /// Caller-chosen monotonic token; every engine event and the reply are
    /// correlated to it (the controller uses its play generation).
    pub token: u64,
    pub factory: SourceFactory,
    pub meta: NowPlayingMeta,
    pub transition: Transition,
    pub start_at: Option<Duration>,
    /// Resolves once the source is playing or failed; dropped on cancellation.
    pub reply: Option<LoadReply>,
}

pub struct Player {
    engine: EngineHandle,
    now_playing: Option<NowPlayingMeta>,
}

impl Player {
    pub fn try_new() -> Result<Self, PlayerInitError> {
        // Android initialises the JNI media session + classloader cache here; the desktop
        // platforms set up their system integration from the app entry point instead.
        #[cfg(target_os = "android")]
        systemint::init();

        let engine = EngineHandle::spawn(|tx| {
            let tx = tx.clone();
            CpalSink::try_new(move || {
                let _ = tx.send(ActorMsg::DeviceError);
            })
            .map(|sink| Box::new(sink) as Box<dyn AudioSink>)
        })?;

        Ok(Self {
            engine,
            now_playing: None,
        })
    }

    pub fn new() -> Self {
        Self::try_new().expect("failed to initialize audio player")
    }

    /// Subscribe to the engine's event stream. Intended for a single pump task;
    /// a later subscription replaces the previous one.
    pub fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<Event> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.engine.send(Command::Subscribe(tx));
        rx
    }

    /// Start loading a new source. Fire-and-forget: the source is built and
    /// probed on an engine worker thread; completion arrives through
    /// `args.reply` and the event stream. The OS now-playing display switches
    /// immediately, consistent with the UI hydrating before the load resolves.
    #[tracing::instrument(name = "player.load", skip_all, fields(title = %args.meta.title))]
    pub fn load(&mut self, args: LoadArgs) {
        let LoadArgs {
            token,
            factory,
            meta,
            transition,
            start_at,
            reply,
        } = args;
        self.engine.send(Command::Load(LoadRequest {
            token,
            factory,
            duration: meta.duration,
            transition,
            start_at,
            reply,
        }));
        self.now_playing = Some(meta);
        self.push_now_playing(start_at.unwrap_or(Duration::ZERO), true);
    }

    /// Drop a load that is still resolving without touching live playback.
    pub fn cancel_pending_load(&self) {
        self.engine.send(Command::CancelPending);
    }

    pub fn pause(&self) {
        self.engine.send(Command::Pause);
        self.push_now_playing(self.get_position(), false);
    }

    pub fn play_resume(&self) {
        self.engine.send(Command::Resume);
        self.push_now_playing(self.get_position(), true);
    }

    pub fn seek(&self, time: Duration) {
        // Mirror the engine's end-guard clamp so the system position display
        // matches what will actually play.
        const END_GUARD: Duration = Duration::from_millis(2000);
        let time = if let Some(meta) = &self.now_playing {
            if meta.duration > END_GUARD {
                time.min(meta.duration - END_GUARD)
            } else {
                Duration::ZERO
            }
        } else {
            time
        };

        self.engine.send(Command::Seek(time));
        self.push_now_playing(time, !self.is_paused());
    }

    pub fn is_playback_complete(&self) -> bool {
        matches!(self.status().phase, Phase::Idle | Phase::Ended)
    }

    pub fn is_paused(&self) -> bool {
        self.status().paused
    }

    pub fn can_resume(&self) -> bool {
        matches!(self.status().phase, Phase::Playing | Phase::Paused)
    }

    pub fn stop(&mut self) {
        self.engine.send(Command::Stop { pause_device: true });
        self.now_playing = None;
        // Tear down the Android foreground service + media notification so the OS can
        // reclaim the process; otherwise the dismissed-notification state lingers.
        #[cfg(target_os = "android")]
        systemint::stop_session();
    }

    pub fn stop_for_transition(&self) {
        self.engine.send(Command::Stop {
            pause_device: false,
        });
    }

    pub fn set_volume(&self, volume: f32) {
        self.engine.send(Command::SetVolume(volume));
    }

    pub fn set_channel_mode(&self, mode: ChannelMode) {
        self.engine.send(Command::SetChannelMode(mode));
    }

    pub fn set_equalizer(&self, settings: EqualizerSettings) {
        self.engine.send(Command::SetEqualizer(settings));
    }

    /// Loudness normalization from the source's ReplayGain/R128 tags. Applies
    /// to the current session immediately and to every session after it.
    pub fn set_replaygain(&self, mode: config::ReplayGainMode, preamp_db: f32) {
        self.engine.send(Command::SetReplayGain { mode, preamp_db });
    }

    pub fn update_metadata(&mut self, meta: NowPlayingMeta) {
        self.engine.send(Command::SetDuration(meta.duration));
        self.now_playing = Some(meta);
        self.update_now_playing_system();
    }

    pub fn get_position(&self) -> Duration {
        self.status().position()
    }

    fn status(&self) -> Arc<EngineStatus> {
        self.engine.status()
    }

    fn update_now_playing_system(&self) {
        self.push_now_playing(self.get_position(), !self.is_paused());
    }

    /// Position/playing are passed explicitly: right after a command the status
    /// snapshot may not reflect it yet, and the OS display should show intent.
    fn push_now_playing(&self, position: Duration, playing: bool) {
        #[cfg(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "windows",
            target_os = "android"
        ))]
        if let Some(meta) = &self.now_playing {
            systemint::update_now_playing(
                &meta.title,
                &meta.artist,
                &meta.album,
                meta.duration.as_secs_f64(),
                position.as_secs_f64(),
                playing,
                meta.artwork.as_deref(),
            );
        }
        #[cfg(not(any(
            target_os = "macos",
            target_os = "linux",
            target_os = "windows",
            target_os = "android"
        )))]
        let _ = (position, playing);
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}
