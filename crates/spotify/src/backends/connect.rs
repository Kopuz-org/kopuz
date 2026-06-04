//! Spotify Connect control backend.
//!
//! Pure Web API. This app does not emit Spotify audio in this mode; audio
//! plays on the user-selected Spotify Connect device.

use crate::auth::{self, AuthCore};
use crate::error::{Result, SpotifyError};
use crate::pkce;
use crate::provider::{
    PlaybackDevice, PlaybackState, PlaylistSummary, RepeatMode, SearchKind, SearchResult,
    StreamingProvider, TrackSummary, device_to_summary, repeat_from_api, repeat_to_api,
    track_to_summary,
};
use crate::token_store::TokenStore;
use crate::types::SpotifyConfig;
use crate::web_api::WebApi;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SpotifyConnectBackend<S: TokenStore> {
    pub config: SpotifyConfig,
    pub auth: Arc<AuthCore<S>>,
    pub api: WebApi<S>,
    selected_device: Mutex<Option<String>>,
    /// Set to true to also request `streaming` scope at login. Connect backend
    /// does not require it; the shared core may set this when the Web Playback
    /// backend is the active one.
    pub needs_streaming_scope: bool,
}

impl<S: TokenStore> SpotifyConnectBackend<S> {
    pub fn new(config: SpotifyConfig, auth: Arc<AuthCore<S>>) -> Self {
        let market = if config.market.is_empty() {
            None
        } else {
            Some(config.market.clone())
        };
        let api = WebApi::new(auth.clone(), market);
        let selected = if config.default_device_id.is_empty() {
            None
        } else {
            Some(config.default_device_id.clone())
        };
        Self {
            config,
            auth,
            api,
            selected_device: Mutex::new(selected),
            needs_streaming_scope: false,
        }
    }

    pub async fn selected_device(&self) -> Option<String> {
        self.selected_device.lock().await.clone()
    }

    async fn device_id(&self) -> Result<Option<String>> {
        let selected = self.selected_device.lock().await.clone();
        let devices = self.api.available_devices().await?;

        if let Some(selected_id) = selected.as_deref()
            && devices
                .iter()
                .any(|device| device.id.as_deref() == Some(selected_id) && !device.is_restricted)
        {
            return Ok(selected);
        }

        *self.selected_device.lock().await = None;
        Ok(None)
    }

    async fn playable_device_id(&self) -> Result<String> {
        self.device_id().await?.ok_or(SpotifyError::NoActiveDevice)
    }

    /// Begin the PKCE login flow without blocking on the callback.
    ///
    /// Returns a `LoginHandle` that holds the bound loopback listener and the
    /// authorize URL the user must visit. The caller decides whether to open
    /// the URL in the system browser, copy it for manual use, or both. Call
    /// `finish_login_flow` to await the callback and exchange the code.
    pub async fn begin_login_flow(&self) -> Result<LoginHandle> {
        if self.config.client_id.is_empty() {
            return Err(SpotifyError::Auth(
                "missing client_id in spotify config".into(),
            ));
        }
        let redirect_uri = self.config.redirect_uri.clone();
        // Validate and extract the port. The URI sent to Spotify must match
        // the one registered in the Dashboard byte-for-byte, so we use the
        // configured value verbatim and bind the same port locally.
        let port = auth::parse_redirect_port(&redirect_uri)?;

        let verifier = pkce::generate_verifier();
        let challenge = pkce::challenge_s256(&verifier);
        let state = pkce::generate_state();
        let scopes = auth::build_scopes(self.needs_streaming_scope);

        let (_, listener) = auth::start_loopback_listener(port).await.map_err(|e| {
            SpotifyError::Auth(format!(
                "could not bind loopback port {port}: {e}. Another process may be using it, or the configured redirect_uri's port does not match what is free on this machine."
            ))
        })?;

        let authorize_url = auth::build_authorize_url(
            &self.config.client_id,
            &redirect_uri,
            &scopes,
            &state,
            &challenge,
        );

        Ok(LoginHandle {
            authorize_url,
            redirect_uri,
            state,
            verifier,
            listener,
        })
    }

    /// Complete a previously-started login flow: wait for the OAuth callback,
    /// validate state, exchange the code for tokens, and persist.
    pub async fn finish_login_flow(&self, handle: LoginHandle) -> Result<()> {
        let q = auth::accept_callback(handle.listener).await?;
        if let Some(err) = q.get("error") {
            return Err(SpotifyError::Auth(format!("authorization denied: {err}")));
        }
        auth::validate_state(&handle.state, q.get("state").map(|s| s.as_str()))?;
        let code = q
            .get("code")
            .ok_or_else(|| SpotifyError::Auth("missing code in callback".into()))?;

        let tokens = auth::exchange_code(
            &self.auth.http,
            &self.config.client_id,
            code,
            &handle.redirect_uri,
            &handle.verifier,
        )
        .await?;
        self.auth.store.save(&tokens).await?;
        Ok(())
    }

    /// Convenience: begin + open browser + finish in one call.
    pub async fn run_login_flow(&self) -> Result<()> {
        let handle = self.begin_login_flow().await?;
        open_in_browser(&handle.authorize_url);
        tracing::info!(target: "spotify::auth", "if your browser did not launch, open this URL manually: {}", handle.authorize_url);
        self.finish_login_flow(handle).await
    }
}

/// Opaque handle to an in-progress PKCE login flow.
///
/// Holds the bound loopback listener (one-shot) and all PKCE material needed
/// to complete the exchange. Dropping this without completing the flow simply
/// closes the listener.
pub struct LoginHandle {
    pub authorize_url: String,
    redirect_uri: String,
    state: String,
    verifier: String,
    listener: tokio::net::TcpListener,
}

fn open_in_browser(url: &str) {
    // We intentionally avoid pulling a heavy dependency for this one job.
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let _ = url;
}

#[async_trait]
impl<S: TokenStore + 'static> StreamingProvider for SpotifyConnectBackend<S> {
    async fn login(&self) -> Result<()> {
        self.run_login_flow().await
    }

    async fn logout(&self) -> Result<()> {
        self.auth.logout().await
    }

    async fn is_logged_in(&self) -> Result<bool> {
        self.auth.is_logged_in().await
    }

    async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let r = self.api.search(query).await?;
        let mut out = Vec::new();
        if let Some(tracks) = r.tracks {
            for t in tracks.items {
                let md = t.clone().into_metadata();
                out.push(SearchResult {
                    kind: SearchKind::Track,
                    id: md.id,
                    uri: md.uri,
                    title: md.title,
                    subtitle: md.artist_names.join(", "),
                    artwork_url: md.artwork_urls.into_iter().next(),
                    duration_ms: md.duration_ms,
                });
            }
        }
        Ok(out)
    }

    async fn user_playlists(&self) -> Result<Vec<PlaylistSummary>> {
        let p = self.api.current_user_playlists().await?;
        Ok(p.items
            .into_iter()
            .map(|pl| PlaylistSummary {
                id: pl.id,
                uri: pl.uri,
                name: pl.name,
                artwork_url: pl.images.into_iter().next().map(|i| i.url),
            })
            .collect())
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<TrackSummary>> {
        let p = self.api.playlist_items(playlist_id).await?;
        Ok(p.items
            .into_iter()
            .filter_map(|it| it.track)
            .map(track_to_summary)
            .collect())
    }

    async fn saved_tracks(&self) -> Result<Vec<TrackSummary>> {
        let p = self.api.saved_tracks().await?;
        Ok(p.items
            .into_iter()
            .map(|s| track_to_summary(s.track))
            .collect())
    }

    async fn devices(&self) -> Result<Vec<PlaybackDevice>> {
        Ok(self
            .api
            .available_devices()
            .await?
            .into_iter()
            .map(device_to_summary)
            .collect())
    }

    async fn select_device(&self, device_id: &str) -> Result<()> {
        self.api.transfer_playback(device_id, false).await?;
        *self.selected_device.lock().await = Some(device_id.to_string());
        Ok(())
    }

    async fn play_uri(&self, uri: &str) -> Result<()> {
        let device = self.playable_device_id().await?;
        self.api
            .play(Some(&device), None, Some(vec![uri]), None, None)
            .await
    }

    async fn play_context(&self, context_uri: &str, offset_uri: Option<&str>) -> Result<()> {
        let device = self.playable_device_id().await?;
        self.api
            .play(Some(&device), Some(context_uri), None, offset_uri, None)
            .await
    }

    async fn pause(&self) -> Result<()> {
        let d = self.device_id().await?;
        self.api.pause(d.as_deref()).await
    }

    async fn resume(&self) -> Result<()> {
        let d = self.playable_device_id().await?;
        self.api.play(Some(&d), None, None, None, None).await
    }

    async fn stop(&self) -> Result<()> {
        // Spotify has no explicit stop, pause is the closest analogue.
        self.pause().await
    }

    async fn next(&self) -> Result<()> {
        let d = self.device_id().await?;
        self.api.next(d.as_deref()).await
    }

    async fn previous(&self) -> Result<()> {
        let d = self.device_id().await?;
        self.api.previous(d.as_deref()).await
    }

    async fn seek(&self, position_ms: u64) -> Result<()> {
        let d = self.device_id().await?;
        self.api.seek(position_ms, d.as_deref()).await
    }

    async fn set_volume(&self, volume_percent: u8) -> Result<()> {
        let d = self.device_id().await?;
        self.api.set_volume(volume_percent, d.as_deref()).await
    }

    async fn set_shuffle(&self, enabled: bool) -> Result<()> {
        let d = self.device_id().await?;
        self.api.set_shuffle(enabled, d.as_deref()).await
    }

    async fn set_repeat(&self, mode: RepeatMode) -> Result<()> {
        let d = self.device_id().await?;
        self.api.set_repeat(repeat_to_api(mode), d.as_deref()).await
    }

    async fn queue(&self, uri: &str) -> Result<()> {
        let d = self.device_id().await?;
        self.api.add_to_queue(uri, d.as_deref()).await
    }

    async fn current_state(&self) -> Result<Option<PlaybackState>> {
        let pb = match self.api.current_playback().await? {
            Some(p) => p,
            None => return Ok(None),
        };
        Ok(Some(PlaybackState {
            is_playing: pb.is_playing,
            track: pb.item.map(track_to_summary),
            progress_ms: pb.progress_ms,
            device: pb.device.map(device_to_summary),
            shuffle: pb.shuffle_state.unwrap_or(false),
            repeat: repeat_from_api(pb.repeat_state.as_deref()),
        }))
    }
}
