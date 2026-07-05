//! Public playback handle.
//!
//! `Player` is a thin façade over the engine actor (`crate::engine`): methods
//! send serialized commands; reads come from the actor's lock-free
//! `EngineStatus` snapshot. No audio state lives on the caller's thread.

use std::sync::Arc;
use std::time::Duration;

use config::{ChannelMode, EqualizerSettings};
use symphonia::core::formats::probe::Hint;

use crate::engine::{
    ActorMsg, AudioSink, Command, CpalSink, EngineHandle, EngineStatus, Event, LoadRequest, Phase,
    SourceFactory, Transition,
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

/// How long a blocking `play()` waits for the engine to probe and start the
/// source. Generous: remote probes can do real network I/O.
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Player {
    engine: EngineHandle,
    now_playing: Option<NowPlayingMeta>,
    next_token: u64,
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
            next_token: 0,
        })
    }

    pub fn new() -> Self {
        Self::try_new().expect("failed to initialize audio player")
    }

    /// Register a callback that fires whenever a track finishes playing naturally
    /// (e.g. EOF or decode error) but NOT when playback is explicitly stopped.
    /// Use this to trigger auto-skip from a background thread without depending
    /// on the Dioxus event loop being active.
    pub fn set_finish_callback(&mut self, f: impl Fn() + Send + Sync + 'static) {
        self.engine.send(Command::SetFinishCallback(Arc::new(f)));
    }

    /// Subscribe to the engine's event stream. Intended for a single pump task;
    /// a later subscription replaces the previous one.
    pub fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<Event> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.engine.send(Command::Subscribe(tx));
        rx
    }

    #[tracing::instrument(name = "player.play", skip_all, fields(title = %meta.title))]
    pub fn play(
        &mut self,
        source: Box<dyn symphonia::core::io::MediaSource>,
        meta: NowPlayingMeta,
        hint: Hint,
    ) -> Result<(), String> {
        self.load(source, meta, hint, Transition::Immediate)
    }

    #[tracing::instrument(name = "player.crossfade", skip_all, fields(title = %meta.title))]
    pub fn crossfade_to(
        &mut self,
        source: Box<dyn symphonia::core::io::MediaSource>,
        meta: NowPlayingMeta,
        hint: Hint,
        duration: Duration,
    ) -> Result<(), String> {
        self.load(source, meta, hint, Transition::Crossfade(duration))
    }

    /// Blocking bridge kept for the poll-loop era: waits until the engine has
    /// probed and started the source so callers see errors synchronously, the
    /// same contract the old in-place engine had.
    fn load(
        &mut self,
        source: Box<dyn symphonia::core::io::MediaSource>,
        meta: NowPlayingMeta,
        hint: Hint,
        transition: Transition,
    ) -> Result<(), String> {
        self.next_token += 1;
        let token = self.next_token;
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let factory: SourceFactory = Box::new(move || Ok((source, hint)));

        self.engine.send(Command::Load(LoadRequest {
            token,
            factory,
            duration: meta.duration,
            transition,
            start_at: None,
            reply: Some(reply_tx),
        }));

        match reply_rx.recv_timeout(LOAD_TIMEOUT) {
            Ok(Ok(())) => {
                self.now_playing = Some(meta);
                self.update_now_playing_system();
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("player load timed out".to_string()),
        }
    }

    pub fn pause(&mut self) {
        self.engine.send(Command::Pause);
        self.push_now_playing(self.get_position(), false);
    }

    pub fn play_resume(&mut self) {
        self.engine.send(Command::Resume);
        self.push_now_playing(self.get_position(), true);
    }

    pub fn seek(&mut self, time: Duration) {
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

    pub fn is_empty(&self) -> bool {
        matches!(self.status().phase, Phase::Idle | Phase::Ended)
    }

    /// True once the track has ended (drained past its last sample). Unlike the
    /// old engine, a seek on an ended track revives it in place.
    pub fn track_ended(&self) -> bool {
        matches!(self.status().phase, Phase::Idle | Phase::Ended)
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

    pub fn stop_for_transition(&mut self) {
        self.engine.send(Command::Stop {
            pause_device: false,
        });
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.engine.send(Command::SetVolume(volume));
    }

    pub fn set_channel_mode(&mut self, mode: ChannelMode) {
        self.engine.send(Command::SetChannelMode(mode));
    }

    pub fn set_equalizer(&mut self, settings: EqualizerSettings) {
        self.engine.send(Command::SetEqualizer(settings));
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
