//! Spotify host lifecycle and external transport control.

use std::time::Duration;

use config::MusicService;
use dioxus::prelude::*;
use reader::Track;

use crate::use_player_controller::PlayerController;

impl PlayerController {
    /// The current Spotify OAuth access token, if Spotify is the active server.
    pub(crate) fn spotify_access(&self) -> Option<String> {
        let cfg = self.config.peek();
        let server = cfg.server.as_ref()?;
        if server.service != MusicService::Spotify {
            return None;
        }
        let packed = server.access_token.clone()?;
        let access = ::server::spotify::auth::unpack_token(&packed).0;
        (!access.is_empty()).then_some(access)
    }

    /// Start the browser playback host if it isn't running yet (first Spotify play).
    fn ensure_spotify_host(&mut self) {
        if self.spotify_host.peek().is_some() || *self.spotify_host_starting.peek() {
            return;
        }
        let Some(access) = self.spotify_access() else {
            return;
        };
        let browser = self.config.peek().spotify_browser.clone();
        self.spotify_host_starting.set(true);
        let mut host_sig = self.spotify_host;
        let mut starting = self.spotify_host_starting;
        let mut error = self.playback_error;
        let mut external = self.external_active;
        let mut playing = self.is_playing;
        let mut pending = self.spotify_pending_uri;
        spawn(async move {
            match ::server::spotify::host::SpotifyHost::start(access, browser).await {
                Ok(host) => host_sig.set(Some(host)),
                Err(e) => {
                    tracing::warn!(error = %e, "spotify host failed to start");
                    error.set(Some(e));
                    if *external.peek() {
                        external.set(false);
                        playing.set(false);
                        pending.set(None);
                    }
                }
            }
            starting.set(false);
        });
    }

    /// Forget a closed Spotify browser host. The next explicit play reads the
    /// current browser setting and starts a fresh host; closing a background
    /// tab must not immediately reopen a browser the user no longer wants.
    pub(crate) fn spotify_browser_disconnected(&mut self) {
        self.spotify_host.set(None);
        self.spotify_device.set(None);
        self.spotify_activated.set(false);
        if self.spotify_device_override.peek().is_some() {
            return;
        }
        self.spotify_pending_uri.set(None);
        self.spotify_commanded.set(None);
        if let Some(task) = self.spotify_start_task.take() {
            task.cancel();
        }
        self.spotify_progress_anchor.set(None);
        self.external_active.set(false);
        self.is_playing.set(false);
    }

    /// The Spotify access token for UI data fetches (Connect device list).
    pub fn spotify_access_token(&self) -> Option<String> {
        self.spotify_access()
    }

    /// Route Spotify playback to a Connect device (`None` = the in-app SDK
    /// device), transferring the live session over when one is active.
    pub fn spotify_select_device(&mut self, device_id: Option<String>) {
        self.spotify_device_chosen.set(true);
        self.spotify_device_override.set(device_id.clone());
        if !*self.external_active.peek() {
            return;
        }
        let Some(access) = self.spotify_access() else {
            return;
        };
        let Some(target) = device_id.or_else(|| self.spotify_device.peek().clone()) else {
            return;
        };
        let play = *self.is_playing.peek();
        spawn(async move {
            if let Err(e) = ::server::spotify::api::transfer_playback(&access, &target, play).await
            {
                tracing::warn!(error = %e, "spotify device transfer failed");
            }
        });
    }

    /// Adopt a Connect device that is already playing on its own — e.g. the user
    /// started it from the Spotify app, possibly before this app launched.
    /// Routes transport to it and flips `external_active` so the player-state
    /// poller begins syncing progress and play/pause state, without transferring
    /// or restarting the live session. No-op when the feature is disabled, a
    /// target is already active, or the user has made an explicit choice.
    pub fn spotify_adopt_external(&mut self, device_id: String) {
        if *self.spotify_device_chosen.peek()
            || self.spotify_device_override.peek().is_some()
            || !self.config.peek().spotify_prefer_active_device
        {
            return;
        }
        self.spotify_device_override.set(Some(device_id));
        self.external_active.set(true);
    }

    pub(crate) fn spotify_transport_pause(&mut self) {
        if self.spotify_device_override.peek().is_some() {
            if let Some(access) = self.spotify_access() {
                spawn(async move {
                    let _ = ::server::spotify::api::player_pause(&access).await;
                });
            }
        } else if let Some(host) = self.spotify_host.peek().clone() {
            host.pause();
        }
    }

    pub(crate) fn spotify_transport_resume(&mut self) {
        if self.spotify_device_override.peek().is_some() {
            if let Some(access) = self.spotify_access() {
                spawn(async move {
                    let _ = ::server::spotify::api::player_resume(&access).await;
                });
            }
        } else if let Some(host) = self.spotify_host.peek().clone() {
            host.resume();
        }
    }

    /// Begin playing a Spotify track id. A chosen Connect device gets the URI
    /// directly; otherwise ensure the host and start it on the SDK device — or
    /// stash it as pending until the device is ready AND the tab has confirmed
    /// playback is allowed. The pump fires the pending URI on those events.
    pub(crate) fn start_spotify_uri(&mut self, access: String, device: String, uri: String) {
        if let Some(task) = self.spotify_start_task.take() {
            task.cancel();
        }
        let mut error = self.playback_error;
        let task = spawn(async move {
            tokio::time::sleep(Duration::from_millis(125)).await;
            if let Err(e) = ::server::spotify::api::start_playback(&access, &device, &[uri]).await {
                tracing::warn!(error = %e, "spotify start_playback failed");
                error.set(Some(e));
            }
        });
        self.spotify_start_task.set(Some(task));
    }

    /// True while Spotify is still reporting a track other than the one kopuz
    /// last commanded. The SDK keeps emitting state for the outgoing track for
    /// a beat after a skip, and applying it flashes the previous song back over
    /// the one kopuz already showed. Self-clearing: the first report that
    /// matches, or the window lapsing, drops the guard so a track genuinely
    /// started from another client is never held back for long.
    pub(crate) fn spotify_report_is_stale(&mut self, reported: Option<&str>) -> bool {
        let Some((commanded, at)) = self.spotify_commanded.peek().clone() else {
            return false;
        };
        if at.elapsed() > Duration::from_secs(3) || reported == Some(commanded.as_str()) {
            self.spotify_commanded.set(None);
            return false;
        }
        true
    }

    pub(super) fn spotify_play(&mut self, item_id: &str, track: &Track) {
        let uri = format!("spotify:track:{item_id}");
        self.spotify_commanded
            .set(Some((item_id.to_string(), std::time::Instant::now())));
        let override_device = self.spotify_device_override.peek().clone();
        if let Some(device) = override_device {
            if let Some(access) = self.spotify_access() {
                self.spotify_pending_uri.set(None);
                self.start_spotify_uri(access, device, uri);
            }
            return;
        }
        self.ensure_spotify_host();
        if let Some(host) = self.spotify_host.peek().clone() {
            let artwork = self.current_song_cover_url.peek().clone();
            host.set_now_playing(track, &artwork);
        }
        if !*self.spotify_activated.peek() {
            self.spotify_pending_uri.set(Some(uri));
            return;
        }
        let sdk_device = self.spotify_device.peek().clone();
        match (self.spotify_access(), sdk_device) {
            (Some(access), Some(device)) => {
                self.spotify_pending_uri.set(None);
                self.start_spotify_uri(access, device, uri);
            }
            _ => self.spotify_pending_uri.set(Some(uri)),
        }
    }

    /// Stop external (Spotify) playback and hand control back to the engine.
    /// Also drops any deferred play so a later `Ready`/`Activated` event can't
    /// start an obsolete track over whatever plays next.
    pub(crate) fn stop_external_playback(&mut self) {
        if !*self.external_active.peek() {
            return;
        }
        self.spotify_pending_uri.set(None);
        self.spotify_commanded.set(None);
        if let Some(task) = self.spotify_start_task.take() {
            task.cancel();
        }
        self.spotify_transport_pause();
        self.external_active.set(false);
    }
}
