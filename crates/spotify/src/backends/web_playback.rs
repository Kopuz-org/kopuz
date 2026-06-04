//! Spotify Web Playback SDK backend.
//!
//! This wraps the Connect-control backend for Web API calls and adds the
//! handshake with the embedded WebView. Audio is emitted by Spotify's official
//! Web Playback SDK, not by this app's audio engine.

use crate::auth::AuthCore;
use crate::backends::connect::SpotifyConnectBackend;
use crate::error::{Result, SpotifyError};
use crate::provider::{
    PlaybackDevice, PlaybackState, PlaylistSummary, RepeatMode, SearchResult, StreamingProvider,
    TrackSummary,
};
use crate::token_store::TokenStore;
use crate::types::SpotifyConfig;
use crate::web_playback::bridge::{BridgeEvent, WebPlaybackBridge};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct SpotifyWebPlaybackBackend<S: TokenStore + 'static> {
    inner: SpotifyConnectBackend<S>,
    pub bridge: Arc<WebPlaybackBridge<S>>,
    sdk_device_id: Arc<Mutex<Option<String>>>,
    /// The URL the user must open in a browser to host the SDK. Populated by
    /// `start_bridge_server`. Same-origin to the bridge's /token and /event.
    player_url: Arc<Mutex<Option<String>>>,
}

impl<S: TokenStore + 'static> SpotifyWebPlaybackBackend<S> {
    pub fn new(config: SpotifyConfig, auth: Arc<AuthCore<S>>) -> Self {
        let mut inner = SpotifyConnectBackend::new(config.clone(), auth.clone());
        inner.needs_streaming_scope = true;
        let bridge = Arc::new(WebPlaybackBridge::new(auth, config.device_name.clone()));
        Self {
            inner,
            bridge,
            sdk_device_id: Arc::new(Mutex::new(None)),
            player_url: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn sdk_device_id(&self) -> Option<String> {
        self.sdk_device_id.lock().await.clone()
    }

    /// URL the user should open to host the Web Playback SDK. Available after
    /// `start_bridge_server` has been called.
    pub async fn player_url(&self) -> Option<String> {
        self.player_url.lock().await.clone()
    }

    /// Bind the bridge HTTP server on an OS-assigned port on `127.0.0.1` and
    /// remember the resulting URL. Idempotent: calling twice does nothing.
    pub async fn start_bridge_server(&self) -> Result<String> {
        let mut url_guard = self.player_url.lock().await;
        if let Some(u) = url_guard.as_ref() {
            return Ok(u.clone());
        }
        let addr = self.bridge.clone().start(0).await?;
        let url = format!("http://{}/", addr);
        *url_guard = Some(url.clone());
        Ok(url)
    }

    /// Drive the bridge event loop. The WebView host is responsible for
    /// loading the HTML page from `web_playback::SPOTIFY_PLAYER_HTML` and
    /// forwarding inbound JS events to `bridge.dispatch`.
    ///
    /// On `ready` the backend transfers playback to the SDK-created device id.
    /// On `account_error` it surfaces `PremiumRequired`.
    /// On `authentication_error` it forces a token refresh once and waits for
    /// the next event (the JS will retry).
    pub async fn run_event_loop(self: Arc<Self>) -> Result<()> {
        let mut auth_retry_used = false;
        loop {
            let ev = match self.bridge.next_event().await {
                Ok(e) => e,
                Err(SpotifyError::WebPlaybackUnavailable) => {
                    return Err(SpotifyError::WebPlaybackUnavailable);
                }
                Err(e) => return Err(e),
            };
            match ev {
                BridgeEvent::Ready { device_id } => {
                    *self.sdk_device_id.lock().await = Some(device_id.clone());
                    if let Err(e) = self.inner.api.transfer_playback(&device_id, false).await {
                        tracing::warn!(target: "spotify::web_playback", "transfer_playback failed: {e}");
                    }
                }
                BridgeEvent::NotReady { device_id } => {
                    tracing::warn!(target: "spotify::web_playback", "device {device_id} not ready");
                    let mut g = self.sdk_device_id.lock().await;
                    if g.as_deref() == Some(device_id.as_str()) {
                        *g = None;
                    }
                }
                BridgeEvent::AccountError(_) => {
                    return Err(SpotifyError::PremiumRequired);
                }
                BridgeEvent::AuthenticationError(_) => {
                    if auth_retry_used {
                        return Err(SpotifyError::WebPlaybackAuthenticationError);
                    }
                    auth_retry_used = true;
                    let _ = self.inner.auth.force_refresh().await?;
                }
                BridgeEvent::InitializationError(m) => {
                    return Err(SpotifyError::WebPlaybackInitializationError(m));
                }
                BridgeEvent::PlaybackError(m) => {
                    return Err(SpotifyError::WebPlaybackPlaybackError(m));
                }
                BridgeEvent::AutoplayFailed => {
                    tracing::info!(target: "spotify::web_playback", "autoplay blocked, user gesture required");
                }
                BridgeEvent::PlayerStateChanged(_) | BridgeEvent::Activated { .. } => {}
            }
        }
    }

    /// Wait up to `timeout` for the SDK to report a device id.
    pub async fn wait_for_device(&self, timeout: Duration) -> Result<String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(id) = self.sdk_device_id().await {
                return Ok(id);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(SpotifyError::WebPlaybackUnavailable);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

#[async_trait]
impl<S: TokenStore + 'static> StreamingProvider for SpotifyWebPlaybackBackend<S> {
    async fn login(&self) -> Result<()> {
        self.inner.run_login_flow().await
    }
    async fn logout(&self) -> Result<()> {
        self.inner.logout().await
    }
    async fn is_logged_in(&self) -> Result<bool> {
        self.inner.is_logged_in().await
    }
    async fn search(&self, q: &str) -> Result<Vec<SearchResult>> {
        self.inner.search(q).await
    }
    async fn user_playlists(&self) -> Result<Vec<PlaylistSummary>> {
        self.inner.user_playlists().await
    }
    async fn playlist_tracks(&self, id: &str) -> Result<Vec<TrackSummary>> {
        self.inner.playlist_tracks(id).await
    }
    async fn saved_tracks(&self) -> Result<Vec<TrackSummary>> {
        self.inner.saved_tracks().await
    }
    async fn devices(&self) -> Result<Vec<PlaybackDevice>> {
        self.inner.devices().await
    }
    async fn select_device(&self, device_id: &str) -> Result<()> {
        // The SDK device is the canonical target; allow override but log it.
        self.inner.select_device(device_id).await
    }
    async fn play_uri(&self, uri: &str) -> Result<()> {
        if let Some(id) = self.sdk_device_id().await {
            self.inner
                .api
                .play(Some(&id), None, Some(vec![uri]), None, None)
                .await
        } else {
            self.inner.play_uri(uri).await
        }
    }
    async fn play_context(&self, context_uri: &str, offset_uri: Option<&str>) -> Result<()> {
        if let Some(id) = self.sdk_device_id().await {
            self.inner
                .api
                .play(Some(&id), Some(context_uri), None, offset_uri, None)
                .await
        } else {
            self.inner.play_context(context_uri, offset_uri).await
        }
    }
    async fn pause(&self) -> Result<()> {
        self.inner.pause().await
    }
    async fn resume(&self) -> Result<()> {
        self.inner.resume().await
    }
    async fn stop(&self) -> Result<()> {
        self.inner.stop().await
    }
    async fn next(&self) -> Result<()> {
        self.inner.next().await
    }
    async fn previous(&self) -> Result<()> {
        self.inner.previous().await
    }
    async fn seek(&self, position_ms: u64) -> Result<()> {
        self.inner.seek(position_ms).await
    }
    async fn set_volume(&self, v: u8) -> Result<()> {
        self.inner.set_volume(v).await
    }
    async fn set_shuffle(&self, e: bool) -> Result<()> {
        self.inner.set_shuffle(e).await
    }
    async fn set_repeat(&self, m: RepeatMode) -> Result<()> {
        self.inner.set_repeat(m).await
    }
    async fn queue(&self, uri: &str) -> Result<()> {
        self.inner.queue(uri).await
    }
    async fn current_state(&self) -> Result<Option<PlaybackState>> {
        self.inner.current_state().await
    }
}
