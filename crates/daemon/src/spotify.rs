//! Spotify as a playback sink.
//!
//! Spotify does not hand out decodable audio, so it plays itself: either in a
//! browser tab running the Web Playback SDK, or on a Connect device the
//! account already owns. Both are driven from here, because both need the
//! account's token and one of them spawns a browser.
//!
//! It is a sink, not a player. It holds one track, and kopuz's queue decides
//! what follows: an end-of-track report advances the session, which loads the
//! next track into whichever of the engine and this can play it.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::{ApiError, ExternalDevice};
use reader::Track;
use server::spotify::host::{HostEvent, SpotifyHost};

use crate::config_service::ConfigService;
use crate::external::{ExternalPlayer, ExternalReport};
use crate::session::SessionHandle;

/// How often a selected Connect device is asked what it is doing. It is a
/// remote poll, so it is slow enough not to be rude and fast enough that a
/// scrub on the phone shows up here.
const CONNECT_POLL: Duration = Duration::from_millis(1500);

/// A refused token is refreshed at most this often, so a burst of auth errors
/// does not become a burst of token requests.
const REFRESH_BACKOFF: Duration = Duration::from_secs(60);

/// The SDK keeps reporting the outgoing track for a beat after a skip; a
/// report older than this is no longer held back.
const COMMAND_GRACE: Duration = Duration::from_secs(3);

pub struct SpotifySink {
    me: std::sync::OnceLock<std::sync::Weak<SpotifySink>>,
    session: SessionHandle,
    config: Arc<ConfigService>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    host: Option<SpotifyHost>,
    starting: bool,
    /// The tab has had its play gesture; before that the SDK refuses to start.
    activated: bool,
    /// The SDK's own Connect device, once it registers.
    device: Option<String>,
    /// A Connect device the user picked instead of this app's.
    selected: Option<String>,
    /// Whether the user has picked at all, which suppresses adoption.
    chosen: bool,
    /// A track asked for before the tab could take it.
    pending_uri: Option<String>,
    /// What was last asked for, so a stale report can be ignored.
    commanded: Option<(String, Instant)>,
    /// What the sink believes is playing, to make a repeated load a no-op.
    playing_key: Option<String>,
    last_refresh: Option<Instant>,
    poll: Option<tokio::task::JoinHandle<()>>,
}

impl SpotifySink {
    pub fn new(session: SessionHandle, config: Arc<ConfigService>) -> Arc<Self> {
        let sink = Arc::new(Self {
            me: std::sync::OnceLock::new(),
            session,
            config,
            state: Mutex::new(State::default()),
        });
        let _ = sink.me.set(Arc::downgrade(&sink));
        sink
    }

    /// The shared handle to this sink, for the paths that spawn work outliving
    /// the call: the trait's methods only take `&self`.
    fn arc(&self) -> Option<Arc<Self>> {
        self.me.get()?.upgrade()
    }

    fn with_state<T>(&self, edit: impl FnOnce(&mut State) -> T) -> T {
        match self.state.lock() {
            Ok(mut state) => edit(&mut state),
            Err(poisoned) => edit(&mut poisoned.into_inner()),
        }
    }

    /// The account's access token, when Spotify is the active server.
    fn access(&self) -> Option<String> {
        let config = self.session.config_watch().borrow().clone();
        let server = config.server.as_ref()?;
        if server.service != config::MusicService::Spotify {
            return None;
        }
        let packed = server.access_token.clone()?;
        let access = server::spotify::auth::unpack_token(&packed).0;
        (!access.is_empty()).then_some(access)
    }

    fn require_access(&self) -> Result<String, ApiError> {
        self.access().ok_or_else(|| {
            ApiError::new(
                api::ErrorCode::SourceAuthExpired,
                "Spotify is not signed in",
            )
        })
    }

    /// The devices the account can play on, for a picker.
    pub async fn devices(&self) -> Result<Vec<ExternalDevice>, ApiError> {
        let access = self.require_access()?;
        let own = self.with_state(|state| state.device.clone());
        let selected = self.with_state(|state| state.selected.clone());
        let devices = server::spotify::api::devices(&access)
            .await
            .map_err(ApiError::internal)?;
        Ok(devices
            .into_iter()
            // This app's own SDK device is offered as "this app", not as one
            // of the account's other devices.
            .filter(|device| Some(&device.id) != own.as_ref())
            .map(|device| ExternalDevice {
                active: selected.as_deref() == Some(device.id.as_str()) || device.is_active,
                id: device.id,
                name: device.name,
                icon: device_icon(&device.kind),
                kind: device.kind,
            })
            .collect())
    }

    /// Route playback to a Connect device, or back to this app's own.
    pub async fn select_device(
        self: &Arc<Self>,
        device_id: Option<String>,
    ) -> Result<(), ApiError> {
        let target = self.with_state(|state| {
            state.chosen = true;
            state.selected = device_id.clone();
            device_id.clone().or_else(|| state.device.clone())
        });
        self.restart_poll(device_id.is_some());
        let Some(target) = target else {
            return Ok(());
        };
        let access = self.require_access()?;
        let playing = self.session.state().phase == api::Phase::Playing;
        server::spotify::api::transfer_playback(&access, &target, playing)
            .await
            .map_err(ApiError::internal)
    }

    /// Adopt a device that is already playing on its own, which is how a
    /// session started from the phone becomes the one this app shows. The
    /// setting that turns this off is read here rather than at the poll, so
    /// changing it takes effect on the next device seen.
    fn adopt(self: &Arc<Self>, device_id: String) {
        let wanted = self
            .session
            .config_watch()
            .borrow()
            .spotify_prefer_active_device;
        let adopt = self.with_state(|state| {
            let prefer = wanted && !state.chosen && state.selected.is_none();
            if prefer {
                state.selected = Some(device_id.clone());
            }
            prefer
        });
        if adopt {
            self.session.attach_external(self.clone());
            self.restart_poll(true);
        }
    }

    /// Start, or stop, the loop that follows a Connect device.
    fn restart_poll(self: &Arc<Self>, run: bool) {
        let previous = self.with_state(|state| state.poll.take());
        if let Some(task) = previous {
            task.abort();
        }
        if !run {
            return;
        }
        let sink = self.clone();
        let handle = tokio::spawn(async move { sink.poll_connect().await });
        self.with_state(|state| state.poll = Some(handle));
    }

    /// Follow the selected Connect device: its position, its play state, and
    /// the track it moved on to.
    async fn poll_connect(self: Arc<Self>) {
        let mut was_playing = false;
        let mut last_position = 0u64;
        let mut last_duration = 0u64;
        loop {
            tokio::time::sleep(CONNECT_POLL).await;
            if self.with_state(|state| state.selected.is_none()) {
                return;
            }
            let Some(access) = self.access() else {
                continue;
            };
            let ended_before =
                was_playing && last_duration > 0 && last_position + 5000 >= last_duration;
            let state = match server::spotify::api::player_state(&access).await {
                Ok(Some(state)) => state,
                Ok(None) => continue,
                Err(error) => {
                    tracing::debug!(%error, "spotify player state poll failed");
                    continue;
                }
            };
            let at_end = state.progress_ms == 0
                || (state.duration_ms > 0 && state.progress_ms + 1500 >= state.duration_ms);
            let completed = !state.is_playing && ended_before && at_end;
            self.report(ExternalReport {
                track: state.track.clone(),
                position_ms: state.progress_ms,
                playing: state.is_playing,
                completed,
                device: state.device_id.clone(),
                cover_url: state.track.as_ref().and_then(|track| track.cover.clone()),
            });
            if completed {
                was_playing = false;
                last_position = 0;
                continue;
            }
            was_playing = state.is_playing;
            last_position = state.progress_ms;
            last_duration = state.duration_ms;
        }
    }

    fn report(&self, report: ExternalReport) {
        if let Some(track) = report.track.as_ref() {
            let key = track.id.key().into_owned();
            self.with_state(|state| state.playing_key = Some(key));
        }
        self.session.report_external(report);
    }

    /// Start the browser tab that hosts the SDK, once.
    fn ensure_host(self: &Arc<Self>) {
        let start = self.with_state(|state| {
            let idle = state.host.is_none() && !state.starting;
            if idle {
                state.starting = true;
            }
            idle
        });
        if !start {
            return;
        }
        let Some(access) = self.access() else {
            self.with_state(|state| state.starting = false);
            return;
        };
        let browser = self.session.config_watch().borrow().spotify_browser.clone();
        let sink = self.clone();
        tokio::spawn(async move {
            match SpotifyHost::start(access, browser).await {
                Ok(host) => {
                    sink.with_state(|state| {
                        state.host = Some(host.clone());
                        state.starting = false;
                    });
                    sink.clone().pump(host).await;
                }
                Err(error) => {
                    tracing::warn!(%error, "spotify playback host would not start");
                    sink.with_state(|state| state.starting = false);
                    sink.notice(error);
                    sink.session.detach_external();
                }
            }
        });
    }

    /// The tab's event stream: what the SDK is doing, and what the person did
    /// to the tab's own media controls.
    async fn pump(self: Arc<Self>, host: SpotifyHost) {
        use tokio::sync::broadcast::error::RecvError;
        let mut events = host.subscribe();
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            match event {
                HostEvent::Ready { device_id } => {
                    self.with_state(|state| state.device = Some(device_id.clone()));
                    self.start_pending().await;
                }
                HostEvent::NotReady => self.with_state(|state| state.device = None),
                HostEvent::Activated => {
                    self.with_state(|state| state.activated = true);
                    self.start_pending().await;
                }
                HostEvent::BrowserDisconnected => {
                    tracing::info!("spotify playback tab closed");
                    self.with_state(|state| {
                        state.host = None;
                        state.device = None;
                        state.activated = false;
                        state.pending_uri = None;
                        state.commanded = None;
                        state.playing_key = None;
                    });
                    self.session.detach_external();
                    break;
                }
                HostEvent::Media {
                    action,
                    position_ms,
                } => self.media_key(&action, position_ms).await,
                HostEvent::State {
                    paused,
                    position_ms,
                    duration_ms,
                    track_id,
                    track,
                    ended,
                    ..
                } => {
                    if self.with_state(|state| state.selected.is_some()) {
                        // A Connect device owns playback; the tab's own state
                        // is about a session nobody is listening to.
                        continue;
                    }
                    if self.stale(track_id.as_deref()) {
                        continue;
                    }
                    let _ = duration_ms;
                    self.report(ExternalReport {
                        track: track.map(|track| *track),
                        position_ms,
                        playing: !paused,
                        completed: ended,
                        device: self.with_state(|state| state.device.clone()),
                        cover_url: None,
                    });
                }
                HostEvent::Error { kind, message } => self.host_error(&kind, message).await,
            }
        }
    }

    /// A media key pressed on the tab: the queue is kopuz's, so it is answered
    /// as an ordinary player command rather than by the SDK.
    async fn media_key(&self, action: &str, position_ms: Option<u64>) {
        let command = match action {
            "play" => api::PlayerCommand::Play,
            "pause" => api::PlayerCommand::Pause,
            "next" => api::PlayerCommand::Next,
            "prev" => api::PlayerCommand::Previous,
            "seek" => match position_ms {
                Some(position_ms) => api::PlayerCommand::Seek { position_ms },
                None => return,
            },
            _ => return,
        };
        if let Err(error) = self.session.player_command(command).await {
            tracing::warn!(%error, action, "a media key from the Spotify tab failed");
        }
    }

    /// Whether a report is about the track we just skipped away from.
    fn stale(&self, reported: Option<&str>) -> bool {
        self.with_state(|state| {
            let Some((commanded, at)) = state.commanded.clone() else {
                return false;
            };
            if at.elapsed() > COMMAND_GRACE || reported == Some(commanded.as_str()) {
                state.commanded = None;
                return false;
            }
            true
        })
    }

    /// Play whatever was asked for before the tab was ready.
    async fn start_pending(&self) {
        let ready = self.with_state(|state| {
            if !state.activated {
                return None;
            }
            let device = state.device.clone()?;
            let uri = state.pending_uri.take()?;
            Some((device, uri))
        });
        let Some((device, uri)) = ready else {
            return;
        };
        self.start_uri(device, uri).await;
    }

    async fn start_uri(&self, device: String, uri: String) {
        let Some(access) = self.access() else {
            return;
        };
        if let Err(error) = server::spotify::api::start_playback(&access, &device, &[uri]).await {
            tracing::warn!(%error, "spotify would not start the track");
            self.notice(error);
        }
    }

    /// An error from the SDK. An expired token is refreshed once and the play
    /// retried by the next load; everything else is said out loud.
    async fn host_error(&self, kind: &str, message: String) {
        if kind == "auth" && self.refresh_due() {
            match self.refresh().await {
                Ok(()) => return,
                Err(error) => tracing::warn!(%error, "spotify token refresh failed"),
            }
        }
        let message = match kind {
            "account" => "Spotify playback needs a Premium account.".to_string(),
            "auth" => "Spotify session expired, sign in again from settings.".to_string(),
            "widevine" => "This browser cannot play Spotify: it has no Widevine DRM.".to_string(),
            "license" => message,
            _ => format!("Spotify player error: {message}"),
        };
        self.notice(message);
    }

    fn refresh_due(&self) -> bool {
        self.with_state(|state| {
            let due = state
                .last_refresh
                .is_none_or(|at| at.elapsed() > REFRESH_BACKOFF);
            if due {
                state.last_refresh = Some(Instant::now());
            }
            due
        })
    }

    /// Swap the stored token for a fresh one. The credential never leaves the
    /// daemon, so this is a config write only it can make.
    async fn refresh(&self) -> Result<(), ApiError> {
        let (packed, client_id) = {
            let config = self.session.config_watch().borrow().clone();
            let server = config
                .server
                .as_ref()
                .filter(|server| server.service == config::MusicService::Spotify)
                .ok_or_else(|| {
                    ApiError::new(
                        api::ErrorCode::SourceAuthExpired,
                        "Spotify is not the active source",
                    )
                })?;
            (
                server.access_token.clone().ok_or_else(|| {
                    ApiError::new(
                        api::ErrorCode::SourceAuthExpired,
                        "Spotify is not signed in",
                    )
                })?,
                server.url.clone(),
            )
        };
        let refreshed = server::spotify::auth::refresh_packed(&packed, client_id.clone())
            .await
            .map_err(ApiError::internal)?;
        self.config
            .mutate_state(move |config| {
                if let Some(server) = config.server.as_mut()
                    && server.service == config::MusicService::Spotify
                    && server.url == client_id
                {
                    server.access_token = Some(refreshed.clone());
                }
            })
            .await?;
        if let Some(host) = self.with_state(|state| state.host.clone())
            && let Some(access) = self.access()
        {
            host.set_token(access).await;
        }
        Ok(())
    }

    fn notice(&self, message: String) {
        self.session.emit_event(api::ApiEvent::Notice {
            level: api::NoticeLevel::Error,
            code: "spotify".to_string(),
            message: Some(message),
        });
    }

    /// Watch for a Connect device that starts playing on its own, so a session
    /// begun on the phone becomes the one this app shows. Stops looking once
    /// the user has picked a device themselves.
    pub fn spawn_discovery(self: &Arc<Self>) {
        let sink = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CONNECT_POLL * 4).await;
                let looking = sink.with_state(|state| !state.chosen && state.selected.is_none());
                if !looking {
                    continue;
                }
                let Some(access) = sink.access() else {
                    continue;
                };
                let own = sink.with_state(|state| state.device.clone());
                if let Ok(Some(state)) = server::spotify::api::player_state(&access).await
                    && state.is_playing
                    && let Some(device) = state.device_id
                    && Some(&device) != own.as_ref()
                {
                    sink.adopt(device);
                }
            }
        });
    }
}

#[async_trait::async_trait]
impl ExternalPlayer for SpotifySink {
    fn kind(&self) -> &'static str {
        "spotify"
    }

    fn service(&self) -> config::MusicService {
        config::MusicService::Spotify
    }

    async fn load(&self, track: &Track, artwork: Option<String>) -> Result<(), ApiError> {
        let key = track.id.key().into_owned();
        // Adopting a session already playing this track must not restart it.
        if self.with_state(|state| state.playing_key.as_deref() == Some(key.as_str())) {
            return Ok(());
        }
        let uri = format!("spotify:track:{key}");
        self.with_state(|state| state.commanded = Some((key, Instant::now())));

        if let Some(device) = self.with_state(|state| state.selected.clone()) {
            self.with_state(|state| state.pending_uri = None);
            self.start_uri(device, uri).await;
            return Ok(());
        }

        // The tab shows what is playing on its own media card, so it is told
        // before it is asked to play.
        if let Some(host) = self.with_state(|state| state.host.clone()) {
            host.set_now_playing(track, artwork.as_deref().unwrap_or_default());
        }
        let ready = self.with_state(|state| {
            let ready = state.activated.then(|| state.device.clone()).flatten();
            if ready.is_none() {
                state.pending_uri = Some(uri.clone());
            }
            ready
        });
        match ready {
            Some(device) => self.start_uri(device, uri).await,
            // No tab yet, or one that has not had its play gesture: start it
            // and let the pending URI go when it is ready.
            None => {
                if let Some(sink) = self.arc() {
                    sink.ensure_host();
                }
            }
        }
        Ok(())
    }

    async fn resume(&self) -> Result<(), ApiError> {
        if let Some(_device) = self.with_state(|state| state.selected.clone()) {
            let access = self.require_access()?;
            return server::spotify::api::player_resume(&access)
                .await
                .map_err(ApiError::internal);
        }
        if let Some(host) = self.with_state(|state| state.host.clone()) {
            host.resume();
        }
        Ok(())
    }

    async fn pause(&self) -> Result<(), ApiError> {
        if let Some(_device) = self.with_state(|state| state.selected.clone()) {
            let access = self.require_access()?;
            return server::spotify::api::player_pause(&access)
                .await
                .map_err(ApiError::internal);
        }
        if let Some(host) = self.with_state(|state| state.host.clone()) {
            host.pause();
        }
        Ok(())
    }

    async fn seek(&self, position_ms: u64) -> Result<(), ApiError> {
        if let Some(_device) = self.with_state(|state| state.selected.clone()) {
            let access = self.require_access()?;
            return server::spotify::api::player_seek(&access, position_ms)
                .await
                .map_err(ApiError::internal);
        }
        if let Some(host) = self.with_state(|state| state.host.clone()) {
            host.seek(position_ms);
        }
        Ok(())
    }

    async fn set_volume(&self, volume: f32) -> Result<(), ApiError> {
        if let Some(_device) = self.with_state(|state| state.selected.clone()) {
            let access = self.require_access()?;
            let percent = (volume.clamp(0.0, 1.0) * 100.0).round() as u8;
            return server::spotify::api::player_volume(&access, percent)
                .await
                .map_err(ApiError::internal);
        }
        if let Some(host) = self.with_state(|state| state.host.clone()) {
            host.set_volume(volume);
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), ApiError> {
        self.with_state(|state| {
            state.pending_uri = None;
            state.commanded = None;
            state.playing_key = None;
        });
        self.pause().await
    }
}

/// A glyph for one of Spotify's device kinds, so a client renders the picker
/// without knowing that vocabulary.
fn device_icon(kind: &str) -> api::schema::Icon {
    api::schema::Icon::Class(
        match kind {
            "Smartphone" | "Tablet" => "fa-solid fa-mobile-screen",
            "Speaker" | "AVR" | "STB" | "AudioDongle" => "fa-solid fa-volume-high",
            "TV" | "CastVideo" => "fa-solid fa-tv",
            _ => "fa-solid fa-computer",
        }
        .to_string(),
    )
}
