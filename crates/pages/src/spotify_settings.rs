//! Spotify settings section: login/logout, backend selection, device picker,
//! Web Playback launcher, current playback, and actionable error banners.

use config::{AppConfig, SpotifyBackendKind};
use dioxus::prelude::*;
use std::sync::Arc;
use tokio::sync::Mutex;

use spotify::SpotifyError;
use spotify::auth::AuthCore;
use spotify::backends::connect::{LoginHandle, SpotifyConnectBackend};
use spotify::provider::{PlaybackDevice, PlaybackState, StreamingProvider};
use spotify::token_store::KeyringTokenStore;
use spotify::types::SpotifyConfig as SpCfg;

type BackendArc = Arc<SpotifyConnectBackend<KeyringTokenStore>>;

/// Cached Spotify backend keyed by a config fingerprint. We hold this in a
/// Dioxus context provider so that login, device select, and playback control
/// all hit the same underlying instance (one keyring entry, one HTTP client).
#[derive(Clone, Default)]
pub struct SpotifyBackendCache {
    inner: Arc<Mutex<Option<(String, BackendArc)>>>,
}

// Dioxus' #[component] macro requires PartialEq on prop types so the runtime
// can skip re-renders when props are unchanged. The cache is identified by
// pointer equality on its `Arc`; that's the right notion here.
impl PartialEq for SpotifyBackendCache {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl SpotifyBackendCache {
    fn fingerprint(cfg: &config::SpotifyConfig) -> String {
        // Anything that changes the backend identity belongs here.
        format!(
            "{}|{}|{}|{}|{}|{:?}",
            cfg.client_id,
            cfg.redirect_uri,
            cfg.device_name,
            cfg.default_device_id,
            cfg.market,
            cfg.backend
        )
    }

    pub async fn get_or_build(&self, cfg: &config::SpotifyConfig) -> Option<BackendArc> {
        if !cfg.enabled || cfg.client_id.trim().is_empty() {
            return None;
        }
        let fp = Self::fingerprint(cfg);
        let mut guard = self.inner.lock().await;
        if let Some((have_fp, backend)) = guard.as_ref() {
            if have_fp == &fp {
                return Some(backend.clone());
            }
        }
        let sp_cfg = SpCfg {
            enabled: cfg.enabled,
            client_id: cfg.client_id.clone(),
            redirect_uri: cfg.redirect_uri.clone(),
            backend: match cfg.backend {
                SpotifyBackendKind::Connect => spotify::types::SpotifyBackendKind::Connect,
                SpotifyBackendKind::WebPlayback => spotify::types::SpotifyBackendKind::WebPlayback,
            },
            device_name: cfg.device_name.clone(),
            default_device_id: cfg.default_device_id.clone(),
            market: cfg.market.clone(),
        };
        let store = Arc::new(KeyringTokenStore::new());
        let http = reqwest::Client::new();
        let auth = Arc::new(AuthCore::new(http, sp_cfg.client_id.clone(), store));
        let mut backend = SpotifyConnectBackend::new(sp_cfg, auth);
        backend.needs_streaming_scope = matches!(cfg.backend, SpotifyBackendKind::WebPlayback);
        let arc = Arc::new(backend);
        *guard = Some((fp, arc.clone()));
        Some(arc)
    }

    pub async fn invalidate(&self) {
        *self.inner.lock().await = None;
    }
}

fn human_error(e: &SpotifyError) -> String {
    match e {
        SpotifyError::NotLoggedIn => "Not logged in to Spotify.".into(),
        SpotifyError::TokenRefreshFailed(m) => {
            format!("Token expired and refresh failed: {m}. Please log in again.")
        }
        SpotifyError::PremiumRequired => {
            "Spotify Premium is required to control playback.".into()
        }
        SpotifyError::Forbidden => {
            "Forbidden. If your app is in Development Mode, ensure this account is on the allowlist and the app owner has Premium.".into()
        }
        SpotifyError::NoActiveDevice => {
            "No active Spotify device. Open Spotify on a device or open the Web Playback page, then click Refresh.".into()
        }
        SpotifyError::RestrictedDevice => "The selected device is restricted.".into(),
        SpotifyError::DeviceUnavailable => "The selected device is unavailable.".into(),
        SpotifyError::RateLimited { retry_after } => {
            format!("Rate limited by Spotify. Retrying after {retry_after}s.")
        }
        SpotifyError::WebPlaybackUnavailable => {
            "Spotify Web Playback SDK is not available. The bridge server failed to start.".into()
        }
        SpotifyError::WebPlaybackAccountError => "Web Playback rejected the account (Premium required).".into(),
        SpotifyError::Auth(m) => format!("Authentication error: {m}"),
        other => other.to_string(),
    }
}

fn open_in_browser(url: &str) {
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

#[component]
pub fn SpotifySettings(config: Signal<AppConfig>) -> Element {
    // Backend cache lives at component scope. In a larger refactor this would
    // be provided at the App root via `use_context_provider`, but for now a
    // local instance per Settings mount is enough.
    let cache = use_hook(SpotifyBackendCache::default);

    let mut logged_in = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    let mut info = use_signal(|| Option::<String>::None);
    let mut devices = use_signal(Vec::<PlaybackDevice>::new);
    let mut state = use_signal(|| Option::<PlaybackState>::None);
    // When a PKCE flow is in progress we surface the authorize URL so the
    // user can copy it if their browser did not auto-open.
    let mut authorize_url = use_signal(|| Option::<String>::None);
    // Pending login handle. The flow proceeds in two steps so the UI can
    // render the authorize URL between "begin" and "await callback".
    let pending_login: Signal<Option<Arc<Mutex<Option<LoginHandle>>>>> = use_signal(|| None);

    // Initial probe of logged-in state.
    {
        let cache_init = cache.clone();
        use_effect(move || {
            let cfg = config.read().spotify.clone();
            let cache = cache_init.clone();
            spawn(async move {
                let Some(backend) = cache.get_or_build(&cfg).await else {
                    logged_in.set(false);
                    return;
                };
                match backend.is_logged_in().await {
                    Ok(v) => logged_in.set(v),
                    Err(e) => error.set(Some(human_error(&e))),
                }
            });
        });
    }

    let cfg_for_render = config.read().spotify.clone();
    let enabled = cfg_for_render.enabled;
    let has_client_id = !cfg_for_render.client_id.trim().is_empty();
    let backend_label = match cfg_for_render.backend {
        SpotifyBackendKind::Connect => "Connect",
        SpotifyBackendKind::WebPlayback => "Web Playback",
    };

    let do_begin_login = {
        let cache = cache.clone();
        move |_| {
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            let mut pending_login = pending_login;
            busy.set(true);
            error.set(None);
            info.set(None);
            authorize_url.set(None);
            spawn(async move {
                let Some(backend) = cache.get_or_build(&cfg).await else {
                    error.set(Some(
                        "Spotify is disabled or client_id is not set. Enable the toggle and paste your Spotify Dashboard client_id, then try again.".into(),
                    ));
                    busy.set(false);
                    return;
                };
                match backend.begin_login_flow().await {
                    Ok(handle) => {
                        authorize_url.set(Some(handle.authorize_url.clone()));
                        info.set(Some(
                            "Opening your browser to Spotify. If nothing opens, copy the URL below and open it manually.".into(),
                        ));
                        open_in_browser(&handle.authorize_url);
                        let pending = Arc::new(Mutex::new(Some(handle)));
                        pending_login.set(Some(pending.clone()));

                        // Drive the callback wait on the same backend.
                        let backend2 = backend.clone();
                        spawn(async move {
                            // Take the handle out so finish_login_flow can consume it.
                            let handle = {
                                let mut g = pending.lock().await;
                                g.take()
                            };
                            let Some(handle) = handle else {
                                busy.set(false);
                                return;
                            };
                            match backend2.finish_login_flow(handle).await {
                                Ok(()) => {
                                    logged_in.set(true);
                                    authorize_url.set(None);
                                    info.set(Some("Logged in to Spotify.".into()));
                                }
                                Err(e) => error.set(Some(human_error(&e))),
                            }
                            busy.set(false);
                            pending_login.set(None);
                        });
                    }
                    Err(e) => {
                        error.set(Some(human_error(&e)));
                        busy.set(false);
                    }
                }
            });
        }
    };

    let do_logout = {
        let cache = cache.clone();
        move |_| {
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            busy.set(true);
            error.set(None);
            info.set(None);
            spawn(async move {
                if let Some(backend) = cache.get_or_build(&cfg).await {
                    if let Err(e) = backend.logout().await {
                        error.set(Some(human_error(&e)));
                    } else {
                        logged_in.set(false);
                        devices.set(Vec::new());
                        state.set(None);
                        info.set(Some("Logged out of Spotify.".into()));
                    }
                }
                busy.set(false);
            });
        }
    };

    let refresh_devices = {
        let cache = cache.clone();
        move |_| {
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            busy.set(true);
            error.set(None);
            spawn(async move {
                if let Some(backend) = cache.get_or_build(&cfg).await {
                    match backend.devices().await {
                        Ok(d) => devices.set(d),
                        Err(e) => error.set(Some(human_error(&e))),
                    }
                    match backend.current_state().await {
                        Ok(s) => state.set(s),
                        Err(e) => error.set(Some(human_error(&e))),
                    }
                }
                busy.set(false);
            });
        }
    };

    rsx! {
        section {
            h2 {
                class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                "Spotify"
            }

            div { class: "flex flex-col gap-3",

                label { class: "flex items-center gap-2 text-white/80",
                    input {
                        r#type: "checkbox",
                        checked: enabled,
                        onchange: move |e| {
                            config.write().spotify.enabled = e.checked();
                        }
                    }
                    "Enable Spotify integration"
                }

                label { class: "flex flex-col gap-1 text-white/70 text-sm",
                    "Client ID (from Spotify Developer Dashboard)"
                    input {
                        class: "bg-black/30 border border-white/10 rounded px-2 py-1 text-white",
                        r#type: "text",
                        value: "{cfg_for_render.client_id}",
                        oninput: move |e| {
                            config.write().spotify.client_id = e.value();
                        }
                    }
                }

                label { class: "flex flex-col gap-1 text-white/70 text-sm",
                    "Redirect URI (must match Dashboard exactly)"
                    input {
                        class: "bg-black/30 border border-white/10 rounded px-2 py-1 text-white",
                        r#type: "text",
                        value: "{cfg_for_render.redirect_uri}",
                        oninput: move |e| {
                            config.write().spotify.redirect_uri = e.value();
                        }
                    }
                    span { class: "text-xs text-white/40",
                        "Use a loopback IP literal with an explicit port. Example: "
                        code { "http://127.0.0.1:8898/callback" }
                        ". Register this exact string in your Spotify app's redirect URIs. Do not use localhost."
                    }
                }

                label { class: "flex items-center gap-2 text-white/80",
                    "Playback backend:"
                    select {
                        class: "bg-black/30 border border-white/10 rounded px-2 py-1 text-white",
                        value: "{backend_label}",
                        onchange: move |e| {
                            let v = e.value();
                            let kind = if v == "Web Playback" {
                                SpotifyBackendKind::WebPlayback
                            } else {
                                SpotifyBackendKind::Connect
                            };
                            config.write().spotify.backend = kind;
                        },
                        option { value: "Connect", "Connect (control external device)" }
                        option { value: "Web Playback", "Web Playback (in-app, requires Premium)" }
                    }
                }

                label { class: "flex flex-col gap-1 text-white/70 text-sm",
                    "Market (optional 2-letter country code)"
                    input {
                        class: "bg-black/30 border border-white/10 rounded px-2 py-1 text-white w-24",
                        r#type: "text",
                        value: "{cfg_for_render.market}",
                        oninput: move |e| {
                            config.write().spotify.market = e.value();
                        }
                    }
                }

                div { class: "flex items-center gap-3 mt-2",
                    if logged_in() {
                        span { class: "text-green-400", "Logged in" }
                    } else {
                        span { class: "text-white/50", "Not logged in" }
                    }
                    span { class: "text-white/40 text-sm", "Backend: {backend_label}" }
                }

                div { class: "flex gap-2",
                    if !logged_in() {
                        button {
                            class: "px-3 py-1 bg-green-700 hover:bg-green-600 text-white rounded disabled:opacity-50",
                            disabled: busy() || !enabled || !has_client_id,
                            onclick: do_begin_login,
                            if busy() { "Working..." } else { "Log in to Spotify" }
                        }
                    } else {
                        button {
                            class: "px-3 py-1 bg-red-700 hover:bg-red-600 text-white rounded disabled:opacity-50",
                            disabled: busy(),
                            onclick: do_logout,
                            "Log out"
                        }
                        button {
                            class: "px-3 py-1 bg-white/10 hover:bg-white/20 text-white rounded disabled:opacity-50",
                            disabled: busy(),
                            onclick: refresh_devices,
                            "Refresh devices"
                        }
                        if matches!(cfg_for_render.backend, SpotifyBackendKind::WebPlayback) {
                            WebPlaybackLauncher { cache: cache.clone(), config }
                        }
                    }
                }

                if let Some(msg) = info() {
                    div { class: "bg-blue-900/30 border border-blue-700/50 text-blue-200 p-2 rounded text-sm", "{msg}" }
                }

                if let Some(url) = authorize_url() {
                    div { class: "bg-black/30 border border-white/10 rounded p-2 text-sm",
                        div { class: "text-white/70 mb-1",
                            "If your browser did not open, copy this URL:"
                        }
                        div { class: "flex gap-2 items-start",
                            textarea {
                                class: "flex-1 bg-black/40 border border-white/10 rounded px-2 py-1 text-white/90 text-xs",
                                readonly: true,
                                rows: 3,
                                "{url}"
                            }
                            button {
                                class: "px-2 py-1 bg-white/10 hover:bg-white/20 text-white rounded text-xs",
                                onclick: move |_| {
                                    let _ = open_in_browser(&url);
                                },
                                "Open again"
                            }
                        }
                    }
                }

                if let Some(msg) = error() {
                    div { class: "bg-red-900/40 border border-red-700/50 text-red-200 p-2 rounded text-sm",
                        "{msg}"
                    }
                }

                if logged_in() && !devices().is_empty() {
                    div { class: "mt-2",
                        div { class: "text-white/70 text-sm mb-1", "Spotify Connect devices" }
                        div { class: "flex flex-col gap-1",
                            for d in devices().iter().cloned() {
                                {
                                    let did = d.id.clone();
                                    let dname = d.name.clone();
                                    let dkind = d.kind.clone();
                                    let active = d.is_active;
                                    let restricted = d.is_restricted;
                                    let cache_btn = cache.clone();
                                    rsx!(
                                        button {
                                            key: "{did}",
                                            class: "text-left px-2 py-1 rounded border border-white/10 hover:bg-white/5 flex items-center justify-between",
                                            disabled: restricted || did.is_empty(),
                                            onclick: move |_| {
                                                let cfg = config.read().spotify.clone();
                                                let did = did.clone();
                                                let cache = cache_btn.clone();
                                                busy.set(true);
                                                error.set(None);
                                                spawn(async move {
                                                    if let Some(backend) = cache.get_or_build(&cfg).await {
                                                        match backend.select_device(&did).await {
                                                            Ok(()) => {
                                                                config.write().spotify.default_device_id = did;
                                                            }
                                                            Err(e) => error.set(Some(human_error(&e))),
                                                        }
                                                    }
                                                    busy.set(false);
                                                });
                                            },
                                            span { class: "text-white", "{dname}" }
                                            span { class: "text-white/40 text-xs",
                                                "{dkind}"
                                                if active { " · active" }
                                                if restricted { " · restricted" }
                                            }
                                        }
                                    )
                                }
                            }
                        }
                    }
                }

                if let Some(s) = state() {
                    div { class: "mt-3 text-sm text-white/70",
                        if let Some(t) = s.track {
                            div {
                                "Now playing: "
                                span { class: "text-white", "{t.title}" }
                                " — "
                                span { class: "text-white/60", "{t.artists.join(\", \")}" }
                            }
                        }
                        div {
                            if s.is_playing { "Playing" } else { "Paused" }
                            if let Some(dev) = s.device.as_ref() {
                                " on "
                                span { class: "text-white", "{dev.name}" }
                            }
                        }
                        div { class: "text-xs text-white/40",
                            "Audio is emitted by the Spotify device above, not by Kopuz."
                        }
                    }
                }

                p { class: "text-xs text-white/40 mt-3",
                    "Kopuz uses only Spotify's official APIs. It does not decode or proxy raw Spotify audio, and does not cache Spotify content offline. Premium is required for playback control."
                }
            }
        }
    }
}

/// Web Playback launcher: starts the local bridge HTTP server (which serves
/// the SDK host page) and opens it in the user's default browser. The browser
/// becomes the WebView; the SDK runs there with full media/DRM support.
#[component]
fn WebPlaybackLauncher(cache: SpotifyBackendCache, config: Signal<AppConfig>) -> Element {
    let mut url = use_signal(|| Option::<String>::None);
    let mut starting = use_signal(|| false);
    let mut err = use_signal(|| Option::<String>::None);

    let start = {
        let cache = cache.clone();
        move |_| {
            let cfg = config.read().spotify.clone();
            let cache = cache.clone();
            starting.set(true);
            err.set(None);
            spawn(async move {
                // We build the Web Playback backend on demand from the same
                // AuthCore the Connect backend uses, so tokens are shared via
                // the keyring entry.
                let Some(connect) = cache.get_or_build(&cfg).await else {
                    err.set(Some("Spotify integration is not configured.".into()));
                    starting.set(false);
                    return;
                };
                let auth = connect.auth.clone();
                let sp_cfg = spotify::types::SpotifyConfig {
                    enabled: cfg.enabled,
                    client_id: cfg.client_id.clone(),
                    redirect_uri: cfg.redirect_uri.clone(),
                    backend: spotify::types::SpotifyBackendKind::WebPlayback,
                    device_name: cfg.device_name.clone(),
                    default_device_id: cfg.default_device_id.clone(),
                    market: cfg.market.clone(),
                };
                let backend = Arc::new(
                    spotify::backends::web_playback::SpotifyWebPlaybackBackend::new(sp_cfg, auth),
                );
                match backend.start_bridge_server().await {
                    Ok(u) => {
                        open_in_browser(&u);
                        url.set(Some(u));
                        // Drive the event loop in the background. If it
                        // returns Premium/auth/init/playback error, surface it.
                        let backend2 = backend.clone();
                        spawn(async move {
                            if let Err(e) = backend2.run_event_loop().await {
                                err.set(Some(human_error(&e)));
                            }
                        });
                    }
                    Err(e) => err.set(Some(human_error(&e))),
                }
                starting.set(false);
            });
        }
    };

    rsx! {
        button {
            class: "px-3 py-1 bg-white/10 hover:bg-white/20 text-white rounded disabled:opacity-50",
            disabled: starting(),
            onclick: start,
            if starting() { "Starting..." } else if url().is_some() { "Reopen Web Playback page" } else { "Open Web Playback page" }
        }
        if let Some(u) = url() {
            span { class: "text-white/40 text-xs ml-2", "Hosted at {u}" }
        }
        if let Some(e) = err() {
            div { class: "bg-red-900/40 border border-red-700/50 text-red-200 p-2 rounded text-sm mt-1", "{e}" }
        }
    }
}
