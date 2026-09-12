//! Playback the audio engine cannot drive: a Spotify Connect device, a browser
//! tab holding a Web Playback SDK session.
//!
//! These are media sources like any other, so the daemon owns them: it holds
//! the credentials, it makes the API calls, and it drives the OS media widget
//! from whatever they report. A frontend sees only `PlayerState.external`,
//! telling it what is playing and where, and sends the same transport commands
//! it always does -- the session routes them here instead of to the engine.
//!
//! An integration is a sink, not a player: it holds one track. The queue, the
//! shuffle order and what plays next stay the session's, which is why there is
//! no `next` here -- an end-of-track report advances kopuz's queue, and the
//! next track is loaded into whichever of the two can play it.

use std::sync::Arc;

use api::ApiError;
use reader::Track;

/// One external integration. Registered with the session while it owns
/// playback; every method is the daemon calling out to the service.
#[async_trait::async_trait]
pub trait ExternalPlayer: Send + Sync {
    /// Integration id, surfaced as `ExternalPlayback.kind`.
    fn kind(&self) -> &'static str;

    /// Which source the reported tracks belong to. They need not be in the
    /// library, so their ids cannot be looked up.
    fn service(&self) -> config::MusicService;

    /// Start one track. `artwork` is a URL the integration may show on its own
    /// surface, which for a browser tab is its Media Session card.
    async fn load(&self, track: &Track, artwork: Option<String>) -> Result<(), ApiError>;

    async fn resume(&self) -> Result<(), ApiError>;
    async fn pause(&self) -> Result<(), ApiError>;
    async fn seek(&self, position_ms: u64) -> Result<(), ApiError>;
    async fn set_volume(&self, volume: f32) -> Result<(), ApiError>;

    /// Give up playback: the queue moved to something the engine plays.
    async fn stop(&self) -> Result<(), ApiError>;
}

/// What the integration is playing, pushed by the integration as it learns it.
#[derive(Debug, Clone, Default)]
pub struct ExternalReport {
    pub track: Option<Track>,
    pub position_ms: u64,
    pub playing: bool,
    /// The track reached its end; counts a listen exactly once, and advances
    /// the queue the way the engine's own end-of-track does.
    pub completed: bool,
    pub device: Option<String>,
    /// Cover for the reported track, which the library may not hold.
    pub cover_url: Option<String>,
}

pub type SharedExternalPlayer = Arc<dyn ExternalPlayer>;
