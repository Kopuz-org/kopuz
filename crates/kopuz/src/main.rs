use components::{
    CoverArtBackground, QuickSearch, bottombar::Bottombar, compact_player::CompactPlayer,
    download_overlay::DownloadOverlay, fullscreen::Fullscreen, rightbar::Rightbar,
    sidebar::Sidebar, spotify_devices::SpotifyDevicesPanel, titlebar::ResizeHandles,
    titlebar::Titlebar,
};
#[cfg(not(target_os = "android"))]
use dioxus::desktop::tao::dpi::LogicalSize;
#[cfg(target_os = "macos")]
use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
#[cfg(target_os = "windows")]
use dioxus::desktop::tao::platform::windows::WindowExtWindows;
#[cfg(target_os = "linux")]
use dioxus::desktop::wry::WebViewExtUnix;
use dioxus::prelude::*;
use futures_util::StreamExt;
use hooks::downloads::DownloadQueue;
use kopuz_route::Route;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::Instrument;
#[cfg(target_os = "linux")]
use webkit2gtk::{SettingsExt, WebViewExt};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;

mod app_db;
mod app_lifecycle;
mod artwork_protocol;
#[cfg(not(target_os = "android"))]
mod chrome_trace;
mod desktop_shell;
#[cfg(not(target_os = "android"))]
mod exit_flush;
#[cfg(not(target_os = "android"))]
mod legacy;
mod logging;
#[cfg(not(target_os = "android"))]
mod ui_profile;
mod updates;
#[cfg(target_os = "windows")]
mod windows_titlebar;

const FAVICON: &str = include_str!(concat!(env!("OUT_DIR"), "/favicon.uri"));
// CSS/fonts are compiled in (not `asset!()`-collected) so styling works under a
// bare `cargo run` — see `build.rs::embed_fonts`, which bakes the font data: URIs.
// The `OUT_DIR` ones pass through it; main.css does too, for its nasin-nanpa
// @font-face (themes/tailwind/reduced have no font refs, so they're verbatim).
const MAIN_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/main.css"));
const THEME_CSS: &str = include_str!("../assets/themes.css");
const TAILWIND_CSS: &str = include_str!("../assets/tailwind.css");
const REDUCED_ANIMATIONS_CSS: &str = include_str!("../assets/reduced-animations.css");
const FONT_AWESOME_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/fontawesome.css"));
const JETBRAINS_MONO_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/jetbrains-mono.css"));
#[cfg(target_os = "windows")]
const TOOLBAR_ICONS: Asset = asset!("../assets/toolbar_icons", AssetOptions::folder());
/// Store saves (config/library/playlists/favorites) are full-replace and
/// expensive; bursts of mutations (batch downloads, syncs) coalesce into one
/// save per settle+cooldown window instead of one per mutation.
const STORE_SAVE_SETTLE_MS: u64 = 600;
const STORE_SAVE_COOLDOWN_MS: u64 = 2500;
/// How often the matugen/pywal palette is stat'd. The active rate is what a
/// wallpaper change costs before the colours follow; the idle one only exists to
/// notice the theme being switched on.
const LIVE_THEME_POLL_MS: u64 = 400;
const LIVE_THEME_IDLE_POLL_MS: u64 = 2000;

struct EmbeddedDaemon {
    #[cfg(debug_assertions)]
    db: db::Db,
    session: daemon::SessionHandle,
    queue_store: Arc<dyn daemon::QueueStore>,
    favorites: Arc<daemon::FavoritesService>,
    scrobbler: Arc<daemon::Scrobbler>,
    frontend: Arc<daemon::FrontendService>,
    api: Arc<dyn api::KopuzApi>,
}

fn frontend_config(mut config: config::AppConfig) -> config::AppConfig {
    config.server = None;
    config.servers.clear();
    config.musicbrainz_token.clear();
    config.lastfm_api_key.clear();
    config.lastfm_api_secret.clear();
    config.lastfm_session_key.clear();
    config.librefm_api_key.clear();
    config.librefm_api_secret.clear();
    config.librefm_session_key.clear();
    config.offline_tracks.clear();
    config
}

fn frontend_config_patch(
    config: &config::AppConfig,
    current: &serde_json::Value,
) -> serde_json::Value {
    let mut value = serde_json::to_value(config).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        for key in [
            "server",
            "servers",
            "musicbrainz_token",
            "lastfm_api_key",
            "lastfm_api_secret",
            "lastfm_session_key",
            "librefm_api_key",
            "librefm_api_secret",
            "librefm_session_key",
            "offline_tracks",
        ] {
            object.remove(key);
        }
        object.retain(|key, value| current.get(key) != Some(value));
    }
    value
}

async fn refresh_frontend_config(api: &dyn api::KopuzApi, mut signal: Signal<config::AppConfig>) {
    match api.config().await {
        Ok(view) => match serde_json::from_value::<config::AppConfig>(view.config) {
            Ok(updated) => {
                let updated = frontend_config(updated);
                if serde_json::to_value(&updated).ok() != serde_json::to_value(&*signal.peek()).ok()
                {
                    signal.set(updated);
                }
            }
            Err(error) => tracing::warn!(%error, "could not decode refreshed daemon config"),
        },
        Err(error) => tracing::warn!(%error, "could not refresh daemon config"),
    }
}

async fn refresh_frontend_sources(
    api: &dyn api::KopuzApi,
    mut signal: Signal<Vec<api::SourceInfo>>,
) {
    match api.sources().await {
        Ok(sources) => signal.set(sources),
        Err(error) => tracing::warn!(%error, "could not refresh daemon sources"),
    }
}

async fn refresh_frontend_downloads(
    api: &dyn api::KopuzApi,
    mut downloads: hooks::downloads::DownloadedTracks,
) {
    match api.downloads().await {
        Ok(keys) => downloads.0.set(keys.into_iter().collect()),
        Err(error) => tracing::warn!(%error, "could not refresh daemon downloads"),
    }
}

fn configured_local_sources(config: &config::AppConfig) -> Vec<(config::Source, Vec<PathBuf>)> {
    std::iter::once((config::Source::Local, config.music_directory.clone()))
        .chain(config.local_sources.iter().map(|source| {
            (
                config::Source::LocalLibrary(source.id.clone()),
                source.directories.clone(),
            )
        }))
        .collect()
}

/// Build the `@font-face` + `body`/`#app-root` override CSS for a user-picked
/// font file, inlining its bytes as a `data:` URI so no custom protocol handler
/// is needed. Returns `None` when the path is empty, unreadable, or an
/// unsupported extension — callers treat that as "no custom font".
fn build_custom_font_css(path: &str) -> Option<String> {
    use base64::Engine;
    if path.is_empty() {
        return None;
    }
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (mime, format) = match ext.as_str() {
        "woff2" => ("font/woff2", "woff2"),
        "woff" => ("font/woff", "woff"),
        "otf" => ("font/otf", "opentype"),
        "ttf" => ("font/ttf", "truetype"),
        _ => return None,
    };
    // Cap the file size before reading: the bytes end up base64-inlined in the
    // DOM, so an oversized (or wrongly-picked) file would bloat the document.
    const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;
    let len = std::fs::metadata(path).ok()?.len();
    if len > MAX_FONT_BYTES {
        tracing::warn!("[custom-font] ignoring {path}: {len} bytes exceeds {MAX_FONT_BYTES} limit");
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!(
        "@font-face {{ font-family: \"kopuz-custom-font\"; \
         src: url(data:{mime};base64,{b64}) format(\"{format}\"); \
         font-display: swap; }}\n\
         body, #app-root {{ font-family: \"kopuz-custom-font\", \"JetBrains Mono\", \
         \"Segoe UI\", Tahoma, Geneva, Verdana, sans-serif, \"nasin-nanpa\"; }}"
    ))
}

#[cfg(target_os = "windows")]
#[component]
fn WindowsToolbarIconAssets() -> Element {
    rsx! {
        div {
            hidden: true,
            "data-toolbar-icons": "{TOOLBAR_ICONS}",
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[component]
fn WindowsToolbarIconAssets() -> Element {
    rsx! {}
}

#[component]
fn StaticHeadAssets() -> Element {
    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Style { {MAIN_CSS} }
        document::Style { {THEME_CSS} }
        document::Style { {TAILWIND_CSS} }
        document::Style { {REDUCED_ANIMATIONS_CSS} }
        // fonts
        document::Style { {JETBRAINS_MONO_CSS} }
        document::Style { {FONT_AWESOME_CSS} }
    }
}

/// Hand the Android trust store to rustls before anything opens a TLS
/// connection. `rustls-platform-verifier` panics on first verification if it was
/// never given a JVM and Context, which takes down the first sync or cover fetch.
///
/// Failing here is not recoverable — every HTTPS request would abort the process
/// later, somewhere far less legible — so the caller stops startup instead.
#[cfg(target_os = "android")]
fn init_android_tls() -> Result<(), String> {
    let ctx = ndk_context::android_context();
    if ctx.vm().is_null() || ctx.context().is_null() {
        return Err("no android JVM or Context on the ndk context".to_string());
    }
    // `::jni` — `dioxus::prelude::*` re-exports its own, older `jni`, and a glob
    // import shadows the extern prelude.
    //
    // SAFETY: wry's activity populates ndk_context before `main` runs, and both
    // handles stay valid for the lifetime of the process.
    let vm = unsafe { ::jni::JavaVM::from_raw(ctx.vm().cast()) };
    let raw_context = ctx.context().cast();
    vm.attach_current_thread(|env| {
        // SAFETY: `raw_context` is the Activity's global Context reference.
        let context = unsafe { ::jni::objects::JObject::from_raw(env, raw_context) };
        rustls_platform_verifier::android::init_with_env(env, context)
    })
    .map_err(|e: ::jni::errors::Error| e.to_string())
}

fn main() {
    // `kopuz pause`, `kopuz status`, ...: act as a control client of the
    // running instance (or kopuzd) and exit, instead of starting the app.
    // Unknown args fall through to the normal launch, so nothing an OS or
    // launcher passes can hijack startup.
    #[cfg(not(target_os = "android"))]
    {
        let cli_args: Vec<String> = std::env::args().skip(1).collect();
        if let Some(code) = daemon::ctl::run(&cli_args) {
            std::process::exit(code);
        }
    }

    #[cfg(target_os = "android")]
    if let Err(e) = init_android_tls() {
        panic!("android certificate verifier failed to initialize: {e}");
    }

    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_FORCE_VBLANK_TIMER").is_none() {
        // SAFETY: first statement of main, before any thread is spawned.
        unsafe { std::env::set_var("WEBKIT_FORCE_VBLANK_TIMER", "1") };
    }

    #[cfg(not(target_os = "android"))]
    {
        let identity_migration = legacy::migrate_identity();

        let log_dir = directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("logs"))
            .unwrap_or_else(|| std::path::PathBuf::from("logs"));
        let _ = std::fs::create_dir_all(&log_dir);

        // A backend-selection failure (another instance holds the daemon
        // lock, unreachable daemon, ...) must not panic here: the tracing
        // subscriber doesn't exist yet, so nothing would be logged and no
        // window would explain anything. Record it and let App() render the
        // same error screen it uses for the other startup failures.
        let config_tracing_enabled = match app_db::select_desktop_backend() {
            Ok(enabled) => enabled,
            Err(error) => {
                app_db::set_startup_error(format!("Kopuz daemon ownership failed: {error}"));
                false
            }
        };

        // Guards live in a global inside `logging`; flushed by
        // logging::shutdown() after launch returns or on Ctrl+C.
        logging::init(&log_dir, config_tracing_enabled);

        for line in identity_migration {
            tracing::info!("{line}");
        }

        if let Some(error) = app_db::startup_error() {
            tracing::error!("{error}");
        } else if app_db::is_embedded() {
            legacy::migrate_locations();
            let _ = app_db::DB_HANDLE.set(app_db::init_blocking());
        } else {
            tracing::info!(mode = "attached", "using the discovered Kopuz daemon");
        }

        #[cfg(target_os = "macos")]
        if app_db::startup_error().is_none() && app_db::is_embedded() {
            player::systemint::init();
        }

        let mut window = dioxus::desktop::WindowBuilder::new()
            .with_title("Kopuz")
            .with_resizable(true)
            .with_inner_size(LogicalSize::new(1350.0, 800.0));

        if let Some(icon) = desktop_shell::build_window_icon() {
            window = window.with_window_icon(Some(icon));
        }

        #[cfg(target_os = "macos")]
        {
            window = window
                .with_title_hidden(true)
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true);
        }

        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            let initial_titlebar_mode = app_db::BOOT_CONFIG
                .get()
                .map(|config| config.titlebar_mode)
                .unwrap_or_default();
            window = window.with_decorations(initial_titlebar_mode == config::TitlebarMode::System);
        }

        let webview_data_dir = directories::ProjectDirs::from("moe", "kopuz", "kopuz")
            .map(|dirs| dirs.cache_dir().join("webview"))
            .unwrap_or_else(|| std::path::PathBuf::from("./cache/webview"));
        let _ = std::fs::create_dir_all(&webview_data_dir);

        let config = dioxus::desktop::Config::new()
            .with_custom_head(
                "<style>html,body{background:#000;margin:0;padding:0}body{opacity:0}</style>"
                    .to_string(),
            )
            .with_background_color((0, 0, 0, 255))
            .with_data_directory(webview_data_dir)
            .with_window(window)
            .with_asynchronous_custom_protocol(
                "artwork",
                |_id, request, responder: dioxus::desktop::RequestAsyncResponder| {
                    artwork_protocol::serve(request.uri().clone(), responder);
                },
            );

        #[cfg(target_os = "macos")]
        let config = {
            use dioxus::desktop::muda::{Menu, PredefinedMenuItem, Submenu};
            let menu = Menu::new();
            let window_menu = Submenu::new("Window", true);
            window_menu
                .append_items(&[
                    &PredefinedMenuItem::fullscreen(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &PredefinedMenuItem::maximize(None),
                    &PredefinedMenuItem::close_window(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::quit(None),
                ])
                .unwrap();
            let edit_menu = Submenu::new("Edit", true);
            edit_menu
                .append_items(&[
                    &PredefinedMenuItem::undo(None),
                    &PredefinedMenuItem::redo(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::cut(None),
                    &PredefinedMenuItem::copy(None),
                    &PredefinedMenuItem::paste(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::select_all(None),
                ])
                .unwrap();
            menu.append_items(&[&window_menu, &edit_menu]).unwrap();
            window_menu.set_as_windows_menu_for_nsapp();
            config.with_menu(Some(menu))
        };

        dioxus::LaunchBuilder::desktop()
            .with_cfg(config)
            .launch(App);
    }

    #[cfg(target_os = "android")]
    {
        // JNI media session + classloader cache. Player::new() also calls this (idempotent
        // OnceLock), but doing it up front means the session exists before first playback.
        player::systemint::init();

        let _ = app_db::DB_HANDLE.set(app_db::init_blocking());

        /// Dioxus gates all task polling on the webview acknowledging the previous
        /// edit batch, and the stock interpreter only sends that ack from a
        /// `requestAnimationFrame` callback — which Chromium suspends while the
        /// activity is backgrounded. One render after backgrounding, the ack never
        /// arrives, `poll_edits_flushed` stays pending forever, and every future in
        /// the app stalls (no queue advance, notification taps pile up undelivered).
        /// This script rebinds the edit path to apply batches and ack immediately —
        /// the interpreter's own headless behavior. The counter stops a stale rAF
        /// callback from acking twice, which would release the next batch before the
        /// DOM had it.
        const APPLY_EDITS_WITHOUT_RAF: &str = r#"<script>
(function () {
    var attempts = 0;
    function patch() {
        var i = window.interpreter;
        if (!i || !i.rafEdits || !i.markEditsFinished) {
            attempts += 1;
            if (attempts > 600) {
                console.error("kopuz: interpreter never appeared; edit-ack patch not applied");
                return;
            }
            setTimeout(patch, 50);
            return;
        }
        var pending = 0;
        var mark = i.markEditsFinished.bind(i);
        i.markEditsFinished = function () {
            if (pending > 0) {
                pending -= 1;
                mark();
            }
        };
        i.rafEdits = function (bytes) {
            pending += 1;
            i.enqueueBytes(bytes);
            i.flushQueuedBytes();
            i.markEditsFinished();
        };
    }
    patch();
})();
</script>"#;

        let config = dioxus::mobile::Config::new()
            .with_custom_head(APPLY_EDITS_WITHOUT_RAF.to_string())
            .with_background_color((0, 0, 0, 255))
            // artwork://local?p=<percent-encoded-absolute-path> — the Android WebView mostly
            // receives base64 data URLs from utils, but keep a synchronous handler for any
            // code path that still emits artwork:// URLs.
            .with_custom_protocol("artwork".to_string(), |_headers, request| {
                fn err_resp(status: u16) -> http::Response<std::borrow::Cow<'static, [u8]>> {
                    http::Response::builder()
                        .status(status)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(std::borrow::Cow::from(Vec::new()))
                        .unwrap_or_else(|_| {
                            http::Response::builder()
                                .status(500)
                                .header("Access-Control-Allow-Origin", "*")
                                .body(std::borrow::Cow::from(Vec::new()))
                                .expect("static fallback response")
                        })
                }

                if let Some(result) = artwork_protocol::fetch_api_sync(request.uri()) {
                    return match result {
                        Ok(payload) => http::Response::builder()
                            .header("Content-Type", payload.content_type)
                            .header("Access-Control-Allow-Origin", "*")
                            .body(std::borrow::Cow::from(payload.data))
                            .unwrap_or_else(|_| err_resp(500)),
                        Err(_) => err_resp(404),
                    };
                }
                let query = request.uri().query().unwrap_or("");
                let raw_p = query
                    .split('&')
                    .find_map(|kv| {
                        let mut parts = kv.splitn(2, '=');
                        if parts.next() == Some("p") {
                            parts.next()
                        } else {
                            None
                        }
                    })
                    .unwrap_or("");
                let decoded = percent_encoding::percent_decode_str(raw_p).decode_utf8_lossy();

                let mime = if decoded.ends_with(".png") {
                    "image/png"
                } else {
                    "image/jpeg"
                };

                let mut decoded_path = decoded.to_string();
                if decoded_path.starts_with("/~") {
                    if let Ok(home) = std::env::var("HOME") {
                        decoded_path = decoded_path.replacen("/~", &home, 1);
                    }
                } else if decoded_path.starts_with('~')
                    && let Ok(home) = std::env::var("HOME")
                {
                    decoded_path = decoded_path.replacen('~', &home, 1);
                }

                let read_result =
                    std::fs::read(std::path::Path::new(&decoded_path)).or_else(|_| {
                        if decoded_path.strip_prefix('/').is_some() {
                            std::fs::read(std::path::Path::new(&decoded_path[1..]))
                        } else {
                            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
                        }
                    });

                match read_result {
                    Ok(bytes) => http::Response::builder()
                        .header("Content-Type", mime)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(std::borrow::Cow::from(bytes))
                        .unwrap_or_else(|_| err_resp(500)),
                    Err(e) => {
                        let status = if e.kind() == std::io::ErrorKind::NotFound {
                            404
                        } else {
                            500
                        };
                        err_resp(status)
                    }
                }
            });

        dioxus::LaunchBuilder::mobile().with_cfg(config).launch(App);
    }
}

#[component]
fn App() -> Element {
    // tao's event loop calls process::exit() on window close, so the
    // logging::shutdown() after .launch() never runs and the chrome trace
    // would be left truncated (cut mid-event, unloadable). Flush on the
    // loop's final event so a normally-closed window still yields a valid
    // trace. (Ctrl+C is covered separately by the SIGINT handler.)
    // logging::shutdown() is called from the DB close-flush handler below —
    // wry handlers fire in registration order, and shutting logging down
    // first would leave the final queue/config persists (and any failure
    // warnings) out of latest.log and the trace.

    #[cfg(target_os = "android")]
    app_lifecycle::use_webview_decipher_engine();

    #[cfg(target_os = "linux")]
    use_hook(|| {
        let webview = dioxus::desktop::window().webview.webview();
        if let Some(settings) = webview.settings() {
            // Kopuz never navigates away from its single Dioxus document.
            settings.set_enable_page_cache(false);
        }
    });

    // The whole-Library signal is GONE — pages/components read the DB through
    // query hooks, and every track self-resolves its cover via the cover seam
    // (a local row's cover_path is projected from its album in the DB read layer).
    let mut current_route = use_signal(|| Route::Home);
    let mut scroll_positions: Signal<std::collections::HashMap<Route, f64>> =
        use_signal(std::collections::HashMap::new);
    // Album/artist list and detail share one Route, so detail scroll is kept in a
    // separate map keyed by `album:<id>` / `artist:<name>`. This stops a detail's
    // scroll from clobbering the list scroll the user expects back on return.
    let mut detail_scroll_positions: Signal<std::collections::HashMap<String, f64>> =
        use_signal(std::collections::HashMap::new);
    // Set by the source switcher's "Manage sources" to scroll Settings to a
    // section (an element id) instead of restoring its last scroll position.
    let mut settings_anchor: Signal<Option<String>> = use_signal(|| None);
    let cache_dir = use_memo(move || {
        // Android: external/ProjectDirs paths aren't writable; use the app-internal files
        // dir (getFilesDir via JNI) so saves don't fail with EACCES.
        #[cfg(target_os = "android")]
        {
            let mut path = player::systemint::get_files_dir()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("cache");
            if std::fs::create_dir_all(&path).is_err() {
                path = std::path::PathBuf::from("./cache");
                let _ = std::fs::create_dir_all(&path);
            }
            path
        }
        #[cfg(not(target_os = "android"))]
        {
            let path = directories::ProjectDirs::from("moe", "kopuz", "kopuz")
                .map(|dirs| dirs.cache_dir().to_path_buf())
                .unwrap_or_else(|| std::path::Path::new("./cache").to_path_buf());
            let _ = std::fs::create_dir_all(&path);
            path
        }
    });
    // ROOT-owned: detached tasks (download workers, close-flush) read/write
    // these after the spawning page — and in principle this component — is
    // gone; owning them at ROOT keeps Dioxus's cross-scope lint honest.
    let mut config = use_hook(|| {
        Signal::new_in_scope(
            frontend_config(app_db::BOOT_CONFIG.get().cloned().unwrap_or_default()),
            ScopeId::ROOT,
        )
    });
    // Snapshot of the file/env config layers (issue #530): which settings file
    // is in play, whether it is Nix-managed, and which keys are pinned by an
    // unwritable layer — the settings UI grays those out.
    use_context_provider(|| {
        let db_path = db::default_db_path();
        let db_dir = match db_path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => std::path::PathBuf::from("."),
        };
        config::store::FileLayers::read(&config::store::settings_path_for(&db_dir))
    });
    hooks::db_reactivity::use_generations_provider();

    // The PoToken minter isn't armed here: it's a headless deno_core runtime that
    // self-starts on the first `mint_content_pot` (only when YT demands a pot).
    let mut initial_load_done = use_signal(|| false);
    #[allow(unused_variables)]
    let cover_cache = use_memo(move || cache_dir().join("covers"));
    let _ = std::fs::create_dir_all(cover_cache());

    let embedded_result = use_hook(|| {
        if let Some(error) = app_db::startup_error() {
            return Err(error.to_string());
        }
        app_db::DB_HANDLE
            .get()
            .cloned()
            .map(|database| -> Result<Arc<EmbeddedDaemon>, String> {
            let seeded = app_db::BOOT_CONFIG.get().cloned().unwrap_or_default();
            let database_path = db::default_db_path();
            let configured_roots = configured_local_sources(&seeded)
                .iter()
                .map(|(_, roots)| roots.len())
                .sum::<usize>();
            tracing::info!(
                mode = "embedded",
                database = %database_path.display(),
                source = %daemon::active_source_label(&seeded),
                source_id = %seeded.active_source.as_str(),
                configured_roots,
                remote_api = seeded.remote_api_enabled,
                "daemon core starting"
            );
            let settings_path = config::store::settings_path_for(
                database_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(".")),
            );
            let config_service = Arc::new(daemon::ConfigService::new(
                database.clone(),
                settings_path,
                seeded.clone(),
            ));
            let registry = Arc::new(radio::registry::StationRegistry::default());
            let library = Arc::new(daemon::LibraryService::new(
                database.clone(),
                seeded.active_source.clone(),
                registry.clone(),
                cover_cache(),
            ));
            let initial_source: ::server::source::ActiveSource =
                Arc::from(::server::source::active(database.clone(), &seeded));
            let queue_store: Arc<dyn daemon::QueueStore> =
                Arc::new(daemon::DbQueueStore::new(database.clone()));
            let scrobbler = daemon::Scrobbler::new(database.clone());
            let recorder = Arc::new(daemon::SourceRecorder::new(database.clone()));
            let services = daemon::PlaybackServices {
                config: seeded,
                active_source: Some(initial_source),
                station_registry: registry,
                queue_store: Some(queue_store.clone()),
                recorder: Some(recorder.clone()),
                scrobbler: Some(scrobbler.clone()),
            };
            let session = match daemon::SessionHandle::try_spawn(library.clone(), services) {
                Ok(session) => session,
                Err(error) => {
                    tracing::error!(%error, "audio engine initialization failed");
                    return Err(format!("Audio engine initialization failed: {error}"));
                }
            };
            library.attach_session(session.clone());
            recorder.attach_session(session.clone());
            scrobbler.attach_session(session.clone());
            let jobs = Arc::new(daemon::JobRunner::new(session.clone()));
            let downloads = daemon::DownloadsService::new(
                database.clone(),
                session.clone(),
                config_service.clone(),
                cache_dir().join("offline_tracks"),
            );
            let favorites = daemon::FavoritesService::new(database.clone(), session.clone());
            favorites.spawn_reconciler();
            daemon::os_media::spawn(&session);
            daemon::integrations::spawn_jellyfin_reporter(
                &session,
                database.clone(),
                session.config_watch(),
            );
            daemon::integrations::spawn_discord_presence(&session, session.config_watch());
            daemon::integrations::spawn_credential_maintenance(
                config_service.clone(),
                session.clone(),
            );
            let artwork = daemon::ArtworkService::new(
                database.clone(),
                session.clone(),
                cache_dir().join("artwork"),
            );
            let frontend = daemon::FrontendService::new(
                database.clone(),
                config_service.clone(),
                library.clone(),
                session.clone(),
                cache_dir().join("uploaded_artwork"),
            );
            let api: Arc<dyn api::KopuzApi> = Arc::new(
                daemon::LocalApi::new(session.clone())
                    .with_library(library)
                    .with_config(config_service.clone())
                    .with_jobs(jobs)
                    .with_favorites(favorites.clone())
                    .with_downloads(downloads)
                    .with_frontend(frontend.clone())
                    .with_artwork(artwork),
            );
            tracing::info!(
                mode = "embedded",
                "daemon services ready: playback, library, config, jobs, downloads, favorites, artwork, scrobbling, integrations, OS media"
            );
            Ok(Arc::new(EmbeddedDaemon {
                #[cfg(debug_assertions)]
                db: database,
                session,
                queue_store,
                favorites,
                scrobbler,
                frontend,
                api,
            }))
        })
        .transpose()
    });
    let embedded = match embedded_result {
        Ok(embedded) => embedded,
        Err(error) => {
            return rsx! {
                main {
                    role: "alert",
                    style: "height: 100vh; display: flex; align-items: center; justify-content: center; padding: 2rem; text-align: center;",
                    "{error}"
                }
            };
        }
    };
    #[cfg(debug_assertions)]
    use_context_provider({
        let embedded = embedded.clone();
        move || embedded.as_ref().map(|daemon| daemon.db.clone())
    });
    let frontend_api = match app_db::remote_api()
        .or_else(|| embedded.as_ref().map(|daemon| daemon.api.clone()))
    {
        Some(api) => api,
        None => {
            tracing::error!("frontend API initialization failed");
            return rsx! {
                main {
                    role: "alert",
                    style: "height: 100vh; display: flex; align-items: center; justify-content: center; padding: 2rem; text-align: center;",
                    "Frontend API initialization failed"
                }
            };
        }
    };
    #[cfg(not(target_os = "android"))]
    if let Some(embedded) = embedded.as_ref() {
        exit_flush::install_queue_persistence(
            embedded.session.clone(),
            embedded.queue_store.clone(),
        );
    }
    #[cfg(not(target_os = "android"))]
    exit_flush::install_frontend_api(frontend_api.clone());
    artwork_protocol::install_api(frontend_api.clone());
    provide_context(frontend_api.clone());
    let mut downloaded_tracks = hooks::downloads::DownloadedTracks(use_signal(HashSet::new));
    provide_context(downloaded_tracks);
    let api_for_downloads = frontend_api.clone();
    use_future(move || {
        let api = api_for_downloads.clone();
        async move {
            match api.downloads().await {
                Ok(keys) => downloaded_tracks.0.set(keys.into_iter().collect()),
                Err(error) => tracing::warn!(%error, "could not load daemon downloads"),
            }
        }
    });
    let mut active_caps = use_signal(api::SourceCapabilities::default);
    let mut frontend_sources = use_signal(Vec::<api::SourceInfo>::new);
    provide_context(active_caps);
    provide_context(frontend_sources);
    let api_for_caps = frontend_api.clone();
    use_effect(move || {
        let source_id = config.read().active_source.as_str().to_string();
        let api = api_for_caps.clone();
        spawn(async move {
            match api.sources().await {
                Ok(sources) => {
                    if let Some(source) = sources.iter().find(|source| source.id == source_id) {
                        active_caps.set(source.capabilities.clone());
                    }
                    frontend_sources.set(sources);
                }
                Err(error) => tracing::warn!(%error, "could not load source capabilities"),
            }
        });
    });

    // The remote control API: serve the same gRPC surface kopuzd exposes
    // from this process, so custom frontends and the control CLI can attach
    // to the running app (loopback only, bearer token in the discovery
    // file). Sharing the process sidesteps the two-writers-on-one-SQLite
    // problem that running kopuzd next to the app would have.
    let mut remote_api_port = use_signal(|| None::<u16>);
    let mut remote_api_token = use_signal(|| None::<String>);
    {
        let embedded = embedded.clone();
        let mut server_task = use_signal(|| None::<dioxus_core::Task>);
        let remote_api_cfg = use_memo(move || {
            let cfg = config.read();
            (cfg.remote_api_enabled, cfg.remote_api_port)
        });
        use_effect(move || {
            let Some(embedded) = embedded.as_ref() else {
                return;
            };
            let (enabled, port) = *remote_api_cfg.read();
            if !*initial_load_done.read() {
                return;
            }
            if let Some(task) = server_task.take() {
                tracing::info!(
                    reason = if enabled {
                        "configuration changed"
                    } else {
                        "disabled"
                    },
                    "embedded daemon API stopping"
                );
                task.cancel();
                remote_api_port.set(None);
                remote_api_token.set(None);
            }
            if !enabled {
                tracing::info!("embedded daemon API disabled");
                return;
            }
            let session = embedded.session.clone();
            let api = embedded.api.clone();
            let task = spawn(async move {
                tracing::info!(requested_port = port, "embedded daemon API starting");
                let discovery_path = daemon::discovery::path();
                if let Some(path) = discovery_path.as_deref() {
                    match daemon::discovery::read(path) {
                        Some(existing) if daemon::discovery::is_serving(&existing).await => {
                            tracing::warn!(
                                port = existing.port,
                                "another daemon already serves the API; embedded daemon API will stay stopped"
                            );
                            return;
                        }
                        Some(existing) => {
                            let _ = daemon::discovery::remove_record(path, &existing);
                        }
                        None if path.exists() => {
                            let _ = daemon::discovery::remove_invalid(path);
                        }
                        None => {}
                    }
                }
                let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        tracing::warn!(%error, port, "embedded daemon API could not bind");
                        return;
                    }
                };
                let Ok(addr) = listener.local_addr() else {
                    return;
                };
                let token = daemon::discovery::random_token();
                let _discovery_lease = match discovery_path.as_deref() {
                    Some(path) => {
                        match daemon::discovery::DiscoveryLease::claim(path, addr.port(), &token) {
                            Ok(lease) => {
                                tracing::info!(
                                %addr,
                                path = %path.display(),
                                "embedded daemon API listening (token in the discovery file)"
                                );
                                Some(lease)
                            }
                            Err(error) => {
                                tracing::warn!(%error, "could not claim the discovery file");
                                return;
                            }
                        }
                    }
                    None => {
                        tracing::warn!("no usable directory for the discovery file");
                        None
                    }
                };
                remote_api_port.set(Some(addr.port()));
                remote_api_token.set(Some(token.clone()));
                let state = Arc::new(daemon::grpc::GrpcState {
                    api,
                    session,
                    token,
                    started: std::time::Instant::now(),
                    shutdown: None,
                });
                match daemon::grpc::serve(listener, state).await {
                    Ok(()) => tracing::info!("embedded daemon API stopped"),
                    Err(error) => tracing::warn!(%error, "embedded daemon API stopped"),
                }
                remote_api_port.set(None);
                remote_api_token.set(None);
            });
            server_task.set(Some(task));
        });
    }
    let download_queue = use_hook(|| Signal::new_in_scope(DownloadQueue::default(), ScopeId::ROOT));
    let mut trigger_rescan = use_signal(|| 0);
    let mut last_scan_key = use_signal(|| None::<String>);
    let mut scan_current_file = use_signal(|| Option::<String>::None);
    let current_playing = use_signal(|| 0);
    let current_song_cover_url = use_signal(String::new);
    let current_song_title = use_signal(String::new);
    let current_song_artist = use_signal(String::new);
    let current_song_album = use_signal(String::new);
    let current_song_duration = use_signal(|| 0u64);
    let current_song_khz = use_signal(|| 0u32);
    let current_song_bitrate = use_signal(|| 0u16);
    let current_song_progress = use_signal(|| 0u64);
    let current_track_snapshot = use_signal(|| None::<reader::Track>);
    let mut volume = use_signal(|| 1.0f32);
    let mut persisted_volume = use_signal(|| 1.0f32);
    let mut configured_local_libraries = use_signal(|| configured_local_sources(&config.peek()));

    let is_playing = use_signal(|| false);
    let mut is_fullscreen = use_signal(|| false);
    let mut compact_mode = use_signal(|| false);
    let is_rightbar_open = use_signal(|| false);
    let is_devices_open = use_signal(|| false);
    let rightbar_width = use_signal(|| 320usize);
    let mut palette = use_signal(|| Option::<Vec<utils::color::Color>>::None);
    // Config is the one remaining whole-value save: persisting a default that
    // exists only because the LOAD FAILED would wipe real settings/servers, so
    // its save stays disarmed unless the load demonstrably succeeded (a fresh
    // empty DB still counts). Library/playlists/favorites have no such flag
    // anymore — they're targeted per-row writes, never full-replace.
    let mut config_loaded_ok = use_signal(|| false);
    #[cfg(not(target_os = "android"))]
    let owns_daemon = embedded.is_some();

    // tao calls process::exit() after CloseRequested, killing the debounced
    // save loops — without this, the last debounce window of queue/store
    // changes was lost on every quit. The flush must run on a FRESH OS
    // thread: the main thread sits inside dioxus's tokio context, where
    // block_on panics ("cannot start a runtime from within a runtime") — the
    // flush silently never ran. Signals are peeked here (not Send), the
    // joined thread does the blocking DB work. Idempotent across
    // CloseRequested/LoopDestroyed.
    #[cfg(not(target_os = "android"))]
    dioxus::desktop::use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        if matches!(
            event,
            Event::LoopDestroyed
                | Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                }
        ) {
            let cfg = (*config_loaded_ok.peek()).then(|| {
                let mut cfg = config.peek().clone();
                cfg.volume = *volume.peek();
                cfg
            });
            exit_flush::persist_on_fresh_thread(owns_daemon && *initial_load_done.peek(), cfg);
            // A quitting app must not leave a discovery file pointing at a
            // dead port; frontends would keep trying to attach to it.
            if let Some(token) = remote_api_token.peek().clone()
                && let Some(path) = daemon::discovery::path()
            {
                let _ = daemon::discovery::remove_owned(&path, &token);
            }
            // After the persists, so they (and any failure warnings) land in
            // latest.log and the trace. Idempotent across CloseRequested/
            // LoopDestroyed; Ctrl+C is covered by the SIGINT handler.
            crate::logging::shutdown();
        }
    });

    #[cfg(target_os = "macos")]
    use_effect(move || {
        let _ = dioxus::document::eval(
            r#"(function(){
            try {
                var ctx = new (window.AudioContext||window.webkitAudioContext)({sampleRate:8000});
                var buf = ctx.createBuffer(1,1,8000);
                var src = ctx.createBufferSource();
                src.buffer = buf;
                src.loop = true;
                src.connect(ctx.destination);
                src.start(0);
                document.addEventListener('visibilitychange', function(){
                    if (ctx.state === 'suspended') ctx.resume();
                });
            } catch(e) {}
        })()"#,
        );
    });

    use_effect(move || {
        let _ = dioxus::document::eval(
            r#"(function(){
                function show(){document.body.style.transition='opacity .15s';document.body.style.opacity='1';}
                var links=document.querySelectorAll('link[rel="stylesheet"]');
                if(!links.length){show();return;}
                var loaded=0;
                function onLoad(){if(++loaded>=links.length)show();}
                links.forEach(function(l){if(l.sheet){onLoad();}else{l.addEventListener('load',onLoad);l.addEventListener('error',onLoad);}});
            })();"#,
        );
    });

    use_effect(move || {
        let _ = dioxus::document::eval(&format!(
            r#"document.addEventListener('error',function(e){{
                var t=e.target;
                if(t.tagName==='IMG'&&!t.dataset.fallback&&t.src){{
                    t.dataset.fallback='1';
                    t.src='{}';
                }}
            }},true);"#,
            utils::DEFAULT_COVER_SVG.replace('\'', "%27"),
        ));
    });

    use_effect(move || {
        let url = current_song_cover_url.read().clone();
        if !url.is_empty() {
            spawn(
                async move {
                    let colors =
                        utils::offload(
                            async move { utils::color::get_palette_from_url(&url).await },
                        )
                        .await;
                    if let Some(colors) = colors {
                        palette.set(Some(colors));
                    }
                }
                .instrument(tracing::info_span!("ui.palette_fetch")),
            );
        } else {
            palette.set(None);
        }
    });

    use_effect(move || {
        let next_sources = configured_local_sources(&config.read());
        if *configured_local_libraries.peek() != next_sources {
            configured_local_libraries.set(next_sources);
        }
    });

    let mut radio_loaded = use_signal(|| false);
    let embedded_for_registry = embedded.clone();
    use_effect(move || {
        if !*initial_load_done.read() || *radio_loaded.peek() {
            return;
        }
        radio_loaded.set(true);
        if let Some(embedded) = embedded_for_registry.as_ref() {
            let frontend = embedded.frontend.clone();
            spawn(async move {
                if let Err(error) = frontend.reload_radio().await {
                    tracing::warn!(%error, "could not load radio registries");
                }
            });
        }
    });

    let mut selected_album_id = use_signal(String::new);
    let mut selected_playlist_id = use_signal(|| None::<String>);
    let mut discover_selected_playlist_id = use_signal(|| None::<String>);
    let mut discover_selected_playlist_title = use_signal(|| None::<String>);
    // YT channel id corresponding to selected_artist_name when known
    // (Discover tile / mix entry carries it). Left None when the
    // click only had a name — the YT artist page resolves it via
    // search at render time.
    let mut selected_artist_channel_id = use_signal(|| None::<String>);
    let mut selected_artist_name = use_signal(String::new);
    let mut fetched_artist_images: Signal<::server::cover::FetchedArtistImages> =
        use_signal(Default::default);
    let mut search_query = use_signal(String::new);
    let mut last_backend_key = use_signal(|| None::<String>);
    let mut backend_key_initialized = use_signal(|| false);
    let queue = use_signal(Vec::<reader::Track>::new);
    let current_queue_index = use_signal(|| 0usize);

    let mut network_banner: Signal<Option<bool>> = use_signal(|| None);
    let mut update_banner: Signal<Option<updates::AvailableUpdate>> = use_signal(|| None);
    let mut did_check_updates = use_signal(|| false);
    let mut ctrl = hooks::use_player_controller(
        frontend_api.clone(),
        is_playing,
        queue,
        current_queue_index,
        current_song_title,
        current_song_artist,
        current_song_album,
        current_song_khz,
        current_song_bitrate,
        current_song_duration,
        current_song_progress,
        current_song_cover_url,
        current_track_snapshot,
        volume,
        config,
        config_loaded_ok,
    );

    // Generations handle the rescan task bumps after writing scanned tracks/albums,
    // so the DB-backed query hooks re-run and the UI refreshes.
    let gens_for_albums = hooks::db_reactivity::use_generations();

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        // Tokens rotate without changing the source identity, but changing the
        // selected source, server URL, or account must clear source-owned state.
        let current_backend_key = frontend_sources
            .read()
            .iter()
            .find(|source| source.active)
            .map(|source| {
                format!(
                    "{}|{:?}|{}|{}",
                    source.id,
                    source.service,
                    source.url.as_deref().unwrap_or_default(),
                    source.authenticated,
                )
            })
            .unwrap_or_else(|| "local".to_string());

        if !*backend_key_initialized.read() {
            last_backend_key.set(Some(current_backend_key));
            backend_key_initialized.set(true);
            return;
        }

        if last_backend_key.peek().as_deref() != Some(current_backend_key.as_str()) {
            last_backend_key.set(Some(current_backend_key));
            selected_album_id.set(String::new());
            selected_playlist_id.set(None);
            discover_selected_playlist_id.set(None);
            discover_selected_playlist_title.set(None);
            selected_artist_channel_id.set(None);
            selected_artist_name.set(String::new());
            fetched_artist_images.set(Default::default());
            ctrl.reset_for_backend_switch();
        }
    });

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        if !config.read().auto_check_updates {
            update_banner.set(None);
            if *did_check_updates.peek() {
                did_check_updates.set(false);
            }
            return;
        }

        if *did_check_updates.read() {
            return;
        }

        did_check_updates.set(true);
        spawn(
            async move {
                if let Some(update) = updates::fetch_available().await {
                    update_banner.set(Some(update));
                }
            }
            .instrument(tracing::info_span!("app.update_check")),
        );
    });

    // The store saves are FULL-REPLACE (hundreds-to-thousands of statements),
    // so saving on every signal mutation hammered the runtime — a batch
    // download bumping `offline_tracks` per finished song ran a complete
    // config save (≈840 listen-count upserts) per completion and starved the
    // audio stream into underruns. Each domain now marks itself dirty and a
    // debounced saver loop persists at most once per cooldown window,
    // coalescing bursts. The CloseRequested flush below covers quitting inside
    // the window.
    let mut config_dirty = use_signal(|| 0u64);
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let _ = config.read();
        config_dirty += 1;
    });
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let _ = *persisted_volume.read();
        config_dirty += 1;
    });
    #[cfg(not(target_os = "android"))]
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let mut snapshot = config.read().clone();
        let _ = *persisted_volume.read();
        snapshot.volume = *volume.peek();
        exit_flush::stash_config(snapshot);
    });
    let api_for_cfg_save = frontend_api.clone();
    use_future(move || {
        let api = api_for_cfg_save.clone();
        async move {
            let mut flushed = 0u64;
            loop {
                if *config_dirty.peek() == flushed {
                    utils::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
                utils::sleep(std::time::Duration::from_millis(STORE_SAVE_SETTLE_MS)).await;
                flushed = *config_dirty.peek();
                let mut snapshot = config.peek().clone();
                snapshot.volume = *volume.peek();
                match api.config().await {
                    Ok(current) => {
                        let patch = frontend_config_patch(&snapshot, &current.config);
                        if patch.as_object().is_some_and(|patch| !patch.is_empty())
                            && let Err(e) = api
                                .patch_config(patch)
                                .instrument(tracing::info_span!("config.persist"))
                                .await
                        {
                            tracing::error!("Failed to save config: {}", e);
                        }
                    }
                    Err(e) => tracing::error!("Failed to read daemon config: {}", e),
                }
                utils::sleep(std::time::Duration::from_millis(STORE_SAVE_COOLDOWN_MS)).await;
            }
        }
    });

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use_effect(move || {
        let mode = config.read().titlebar_mode;
        let win = dioxus::desktop::window();
        win.set_decorations(mode == config::TitlebarMode::System);
    });

    #[cfg(target_os = "windows")]
    use_effect(move || {
        let mode = config.read().titlebar_mode;
        let win = dioxus::desktop::window();
        let hwnd = HWND(win.window.hwnd() as _);
        windows_titlebar::install(hwnd);
        windows_titlebar::set_custom_titlebar_enabled(mode == config::TitlebarMode::Custom);
    });

    // Library/playlists/favorites have no save loops anymore — every mutation
    // commits as a targeted write at the call site and bumps a generation.

    #[cfg(not(target_os = "android"))]
    {
        use dioxus::desktop::trayicon::TrayIcon;
        use dioxus::desktop::{WindowCloseBehaviour, window};
        use std::cell::RefCell;
        use std::rc::Rc;

        let tray_slot: Rc<RefCell<Option<TrayIcon>>> = use_hook(|| Rc::new(RefCell::new(None)));
        let tray_warned: Rc<RefCell<bool>> = use_hook(|| Rc::new(RefCell::new(false)));

        const TRAY_SHOW_ID: &str = "kopuz-tray-show";
        const TRAY_QUIT_ID: &str = "kopuz-tray-quit";

        let win_ctx = window();
        let handle_menu = {
            let win_ctx = win_ctx.clone();
            move |id: &dioxus::desktop::trayicon::menu::MenuId| {
                tracing::debug!("tray menu event id={:?}", id);
                if *id == TRAY_SHOW_ID {
                    if win_ctx.is_visible() {
                        win_ctx.set_visible(false);
                    } else {
                        win_ctx.set_visible(true);
                        win_ctx.set_focus();
                    }
                } else if *id == TRAY_QUIT_ID {
                    win_ctx.set_close_behavior(WindowCloseBehaviour::WindowCloses);
                    win_ctx.close();
                }
            }
        };
        dioxus::desktop::use_tray_menu_event_handler({
            let handle_menu = handle_menu.clone();
            move |event| handle_menu(&event.id)
        });
        dioxus::desktop::use_muda_event_handler({
            let handle_menu = handle_menu.clone();
            move |event| handle_menu(&event.id)
        });

        use_effect({
            let tray_slot = tray_slot.clone();
            let tray_warned = tray_warned.clone();
            move || {
                use dioxus::desktop::trayicon::TrayIconBuilder;
                let want_tray = config.read().minimize_to_tray;
                let enabled = want_tray && desktop_shell::tray_backend_available();
                let mut warned = tray_warned.borrow_mut();
                if want_tray && !enabled {
                    tracing::error!(
                        "minimize_to_tray is enabled but no system tray backend was found. \
                         Install the appindicator library for your distro: \
                         libayatana-appindicator3 (Debian/Ubuntu/Arch: libayatana-appindicator), \
                         Fedora: libappindicator-gtk3. \
                         Closing the window will quit the app instead of hiding to tray."
                    );
                    if !*warned {
                        desktop_shell::show_tray_missing_popup();
                        *warned = true;
                    }
                } else {
                    *warned = false;
                }
                drop(warned);
                window().set_close_behavior(if enabled {
                    WindowCloseBehaviour::WindowHides
                } else {
                    WindowCloseBehaviour::WindowCloses
                });

                let mut slot = tray_slot.borrow_mut();
                match (enabled, slot.is_some()) {
                    (true, false) => {
                        use dioxus::desktop::trayicon::menu::{Menu, MenuItem};

                        let menu = Menu::new();
                        let show = MenuItem::with_id(TRAY_SHOW_ID, "Show / Hide Kopuz", true, None);
                        let quit = MenuItem::with_id(TRAY_QUIT_ID, "Quit Kopuz", true, None);
                        if let Err(e) = menu.append_items(&[&show, &quit]) {
                            tracing::warn!("Failed to build tray menu: {e}");
                        }

                        let mut builder = TrayIconBuilder::new()
                            .with_tooltip("Kopuz")
                            .with_menu(Box::new(menu))
                            .with_menu_on_left_click(false);
                        if let Some(icon) = desktop_shell::build_tray_icon() {
                            builder = builder.with_icon(icon);
                        }
                        match builder.build() {
                            Ok(tray) => *slot = Some(tray),
                            Err(e) => tracing::warn!("Failed to build tray icon: {e}"),
                        }
                    }
                    (false, true) => *slot = None,
                    _ => {}
                }
            }
        });
    }

    let mut offline_mode = app_lifecycle::use_connectivity_probe(frontend_sources, network_banner);

    let api_for_load = frontend_api.clone();
    let embedded_for_load = embedded.clone();
    use_hook(move || {
        let api = api_for_load;
        let embedded = embedded_for_load;

        spawn(async move {
            let restored_queue_tracks = if let Some(daemon) = embedded.as_ref() {
                match daemon.queue_store.load().await {
                    Some(snapshot) => {
                        let count = snapshot.queue.len();
                        if let Err(error) = daemon.session.restore_queue(snapshot).await {
                            tracing::warn!(%error, "queue restore failed");
                        }
                        count
                    }
                    None => 0,
                }
            } else {
                api.live_queue()
                    .await
                    .map(|snapshot| snapshot.tracks.len())
                    .unwrap_or_default()
            };

            let cfg_loaded = match api
                .config()
                .instrument(tracing::info_span!("startup.load_config"))
                .await
            {
                Ok(view) => match serde_json::from_value::<config::AppConfig>(view.config) {
                    Ok(config) => {
                        config_loaded_ok.set(true);
                        Some(config)
                    }
                    Err(error) => {
                        tracing::error!(%error, "failed to decode daemon config — config saves disabled this session");
                        None
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "failed to load daemon config — config saves disabled this session");
                    None
                }
            };

            let cfg_loaded = cfg_loaded
                .unwrap_or_else(|| app_db::BOOT_CONFIG.get().cloned().unwrap_or_default());
            let startup_source = daemon::active_source_label(&cfg_loaded);
            let startup_source_id = cfg_loaded.active_source.as_str().to_string();
            let startup_roots = configured_local_sources(&cfg_loaded)
                .iter()
                .map(|(_, roots)| roots.len())
                .sum::<usize>();
            {
                let _apply = tracing::info_span!("startup.apply_config").entered();
                let loaded = cfg_loaded;
                config.set(frontend_config(loaded.clone()));
                configured_local_libraries.set(configured_local_sources(&loaded));
                volume.set(loaded.volume);
                persisted_volume.set(loaded.volume);
                i18n::set_locale(&loaded.language);
            }

            initial_load_done.set(true);
            #[cfg(not(target_os = "android"))]
            if embedded.is_some() {
                exit_flush::enable_queue_flush();
            }
            let mode = if embedded.is_some() {
                "embedded"
            } else {
                "attached"
            };
            tracing::info!(
                mode,
                source = %startup_source,
                source_id = %startup_source_id,
                configured_roots = startup_roots,
                restored_queue_tracks,
                "daemon startup state restored"
            );
            if let Some(daemon) = embedded {
                daemon.favorites.nudge_activate();
                let scrobbler = daemon.scrobbler.clone();
                let drain_config = config.peek().clone();
                spawn(async move {
                    scrobbler.drain_queue(&drain_config).await;
                });
            }
            }
            .instrument(tracing::info_span!("startup.load")));
    });

    let api_for_play_album = frontend_api.clone();
    let api_for_scan = frontend_api.clone();
    use_effect(move || {
        // config_loaded_ok matters here: a defaulted config (load failure) has
        // an empty music_directory, and the daemon's no-dirs branch prunes the
        // local library - which must never happen off phantom state.
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let configured_sources = configured_local_libraries.read().clone();
        let trigger = *trigger_rescan.read();

        let scan_key = format!(
            "{}|{}",
            configured_sources
                .iter()
                .flat_map(|(source, dirs)| {
                    std::iter::once(source.as_str().to_string())
                        .chain(dirs.iter().map(|dir| dir.to_string_lossy().into_owned()))
                })
                .collect::<Vec<_>>()
                .join(","),
            trigger,
        );
        if *last_scan_key.peek() == Some(scan_key.clone()) {
            return;
        }
        last_scan_key.set(Some(scan_key));

        let api = api_for_scan.clone();
        spawn(async move {
            if let Err(error) = api.start_job(api::JobKind::Scan).await {
                tracing::warn!(%error, "library scan could not start");
            }
        });
    });

    // Feed daemon events back into the UI: library invalidations re-run the
    // query hooks, and scan job progress drives the scan indicator.
    {
        let gens = gens_for_albums;
        let api = frontend_api.clone();
        let downloads = downloaded_tracks;
        let sources = frontend_sources;
        use_future(move || {
            let api = api.clone();
            async move {
                let mut events = api.events();
                while let Some(event) = events.next().await {
                    match event {
                        api::ApiEvent::LibraryInvalidated { table, .. } => {
                            use hooks::db_reactivity::Table;
                            let mapped = match table {
                                api::Table::Tracks => Some(Table::Tracks),
                                api::Table::Albums => Some(Table::Albums),
                                api::Table::Playlists => Some(Table::Playlists),
                                api::Table::Favorites => Some(Table::Favorites),
                                api::Table::Folders => Some(Table::Folders),
                                api::Table::Servers => Some(Table::Servers),
                                api::Table::Recents => Some(Table::Recents),
                                _ => None,
                            };
                            if let Some(table) = mapped {
                                gens.bump_coalesced(table);
                            }
                            if table == api::Table::Servers {
                                refresh_frontend_sources(api.as_ref(), sources).await;
                            }
                        }
                        api::ApiEvent::JobProgress(progress)
                            if progress.kind == api::JobKind::Scan =>
                        {
                            scan_current_file.set(Some(progress.message.unwrap_or(progress.phase)));
                        }
                        api::ApiEvent::JobFinished {
                            kind: api::JobKind::Scan,
                            ..
                        } => {
                            scan_current_file.set(None);
                        }
                        api::ApiEvent::ConfigChanged { keys } => {
                            refresh_frontend_config(api.as_ref(), config).await;
                            refresh_frontend_sources(api.as_ref(), sources).await;
                            if keys.iter().any(|key| key == "offline_tracks") {
                                refresh_frontend_downloads(api.as_ref(), downloads).await;
                            }
                        }
                        api::ApiEvent::Resync => {
                            use hooks::db_reactivity::Table;
                            for table in [
                                Table::Tracks,
                                Table::Albums,
                                Table::Playlists,
                                Table::Favorites,
                                Table::Folders,
                                Table::Servers,
                                Table::Recents,
                            ] {
                                gens.bump(table);
                            }
                            refresh_frontend_config(api.as_ref(), config).await;
                            refresh_frontend_sources(api.as_ref(), sources).await;
                            refresh_frontend_downloads(api.as_ref(), downloads).await;
                            match api.jobs().await {
                                Ok(jobs) => {
                                    let scan = jobs.into_iter().find(|job| {
                                        job.kind == api::JobKind::Scan
                                            && job.state == api::JobState::Running
                                    });
                                    scan_current_file
                                        .set(scan.map(|job| job.message.unwrap_or(job.phase)));
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "could not refresh daemon jobs")
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    use_effect(move || {
        let route = *current_route.read();
        // Read detail selections so this re-runs on list<->detail toggle, not just
        // on route change (album/artist list and detail are the same Route).
        let album_sel = selected_album_id.read().clone();
        let artist_sel = selected_artist_name.read().clone();
        // A pending section anchor (peeked, so this effect doesn't subscribe to it)
        // takes over scrolling — skip the saved-scroll restore for this navigation.
        if settings_anchor.peek().is_some() {
            return;
        }
        let pos = match route {
            Route::Album if !album_sel.is_empty() => detail_scroll_positions
                .peek()
                .get(&format!("album:{album_sel}"))
                .copied()
                .unwrap_or(0.0),
            Route::Artist if !artist_sel.is_empty() => detail_scroll_positions
                .peek()
                .get(&format!("artist:{artist_sel}"))
                .copied()
                .unwrap_or(0.0),
            _ => scroll_positions.peek().get(&route).copied().unwrap_or(0.0),
        };
        let _ = dioxus::document::eval(&format!(
            "let el = document.getElementById('main-scroll-area'); if (el) el.scrollTop = {pos};"
        ));
    });

    // Scroll Settings to a requested section once the page is on screen, then
    // clear the request. Subscribes to the anchor so setting it (from any page)
    // drives the scroll; the restore effect above stands down while it's set.
    use_effect(move || {
        let anchor = settings_anchor.read().clone();
        if let Some(id) = anchor {
            let _ = dioxus::document::eval(&format!(
                "requestAnimationFrame(() => {{ const el = document.getElementById('{id}'); \
                 if (el) el.scrollIntoView({{ block: 'start' }}); }});"
            ));
            settings_anchor.set(None);
        }
    });

    provide_context(ctrl);
    provide_context(config);
    let discover_now_playing = use_signal(|| None::<String>);
    provide_context(pages::server::discover::DiscoverNowPlaying(
        discover_now_playing,
    ));
    let discover_prefetch_cache = use_signal(std::collections::HashMap::new);
    provide_context(pages::server::discover::DiscoverPrefetchCache(
        discover_prefetch_cache,
    ));
    provide_context(download_queue);
    provide_context(scroll_positions);
    provide_context(components::source_switcher::SettingsAnchor(settings_anchor));
    provide_context(fetched_artist_images);
    let mut nav_history = use_signal(Vec::<components::NavSnapshot>::new);
    let mut nav_restoring = use_signal(|| false);
    let mut nav_last = use_signal(|| None::<components::NavSnapshot>);
    use_effect(move || {
        let snap = components::NavSnapshot {
            route: *current_route.read(),
            album_id: selected_album_id.read().clone(),
            artist_name: selected_artist_name.read().clone(),
            artist_channel_id: selected_artist_channel_id.read().clone(),
            playlist_id: selected_playlist_id.read().clone(),
            discover_playlist_id: discover_selected_playlist_id.read().clone(),
            discover_playlist_title: discover_selected_playlist_title.read().clone(),
        };
        if *nav_restoring.peek() {
            nav_restoring.set(false);
            nav_last.set(Some(snap));
            return;
        }
        let prev = nav_last.peek().clone();
        match prev {
            Some(prev) if prev != snap => {
                nav_history.write().push(prev);
                nav_last.set(Some(snap));
            }
            None => nav_last.set(Some(snap)),
            _ => {}
        }
    });

    let nav_ctrl = components::NavigationController {
        current_route,
        selected_artist_name,
        selected_artist_channel_id,
        selected_album_id,
        selected_playlist_id,
        discover_playlist_id: discover_selected_playlist_id,
        discover_playlist_title: discover_selected_playlist_title,
        history: nav_history,
        restoring: nav_restoring,
    };
    provide_context(nav_ctrl);

    // Sidebar collapse state. On Android the sidebar is an overlay drawer that
    // starts collapsed and is toggled by the mobile header hamburger; the
    // Sidebar component reads this from context.
    let mut is_sidebar_collapsed = use_signal(|| cfg!(target_os = "android"));
    use_context_provider(|| components::sidebar::SidebarCollapsed(is_sidebar_collapsed));

    // Mirror of the drawer's swipe-left-to-close: a swipe right anywhere on the
    // page opens it. Horizontal carousels swallow their own touches so scrolling
    // one back to the start does not pull the drawer out with it.
    let mut open_swipe = components::gestures::use_swipe();
    let on_open_swipe = move |evt: TouchEvent| {
        if open_swipe.finish(&evt) == Some(components::gestures::SwipeDirection::Right)
            && cfg!(target_os = "android")
            && *is_sidebar_collapsed.peek()
        {
            is_sidebar_collapsed.set(false);
        }
    };

    use_context_provider(|| components::CompactMode(compact_mode));
    #[cfg(not(target_os = "android"))]
    {
        let mut saved_window_size = use_signal(|| None::<LogicalSize<f64>>);
        use_effect(move || {
            let active = *compact_mode.read();
            let win = dioxus::desktop::window();
            if active {
                let scale = win.window.scale_factor();
                let current = win.window.inner_size().to_logical::<f64>(scale);
                saved_window_size.set(Some(current));
                win.window.set_always_on_top(true);
                let compact_h = if cfg!(target_os = "macos") {
                    170.0
                } else {
                    148.0
                };
                win.window.set_resizable(true);
                win.window
                    .set_min_inner_size(Some(LogicalSize::new(260.0, compact_h)));
                win.window.set_max_inner_size(None::<LogicalSize<f64>>);
                win.window
                    .set_inner_size(LogicalSize::new(380.0, compact_h));
            } else {
                win.window.set_always_on_top(false);
                win.window.set_resizable(true);
                win.window.set_min_inner_size(None::<LogicalSize<f64>>);
                win.window.set_max_inner_size(None::<LogicalSize<f64>>);
                if let Some(size) = saved_window_size.take() {
                    win.window.set_inner_size(size);
                }
            }
        });
    }

    hooks::use_player_task(ctrl);

    // Inject CSS for all custom themes reactively
    let custom_themes_css = use_memo(move || {
        config
            .read()
            .custom_themes
            .iter()
            .map(|(id, ct)| utils::themes::custom_theme_to_css(id, &ct.vars))
            .collect::<Vec<_>>()
            .join("\n\n")
    });

    use_effect(move || {
        let css = custom_themes_css.read().clone();
        // Serialize as a JSON string literal so no CSS content can escape the JS context
        let css_json = serde_json::to_string(&css).unwrap_or_else(|_| "\"\"".to_string());
        let _ = dioxus::document::eval(&format!(
            r#"(function(){{
                let el = document.getElementById('custom-themes-style');
                if (!el) {{ el = document.createElement('style'); el.id = 'custom-themes-style'; document.head.appendChild(el); }}
                el.textContent = {css_json};
            }})()"#
        ));
    });

    // matugen and pywal rewrite their output on every wallpaper change, so the
    // palette is polled rather than read once: picking a new wallpaper recolours
    // Kopuz in place. Only while the theme is selected, otherwise this is a timer
    // nobody asked for.
    let mut live_theme_css = use_signal(String::new);
    use_future(move || async move {
        let mut last: Option<(PathBuf, String)> = None;
        loop {
            if config.peek().theme != utils::live_theme::THEME_ID {
                if last.take().is_some() {
                    live_theme_css.set(String::new());
                }
                utils::sleep(std::time::Duration::from_millis(LIVE_THEME_IDLE_POLL_MS)).await;
                continue;
            }
            let path = utils::live_theme::resolve_path(&config.peek().live_theme_path);
            let probe = path.clone();
            let raw = tokio::task::spawn_blocking(move || utils::live_theme::read(&probe))
                .await
                .unwrap_or_default();
            let current = raw.map(|raw| (path, raw));
            if current != last {
                last = current;
                let css = last
                    .as_ref()
                    .and_then(|(path, raw)| utils::live_theme::parse(raw, path))
                    .map(|vars| utils::live_theme::to_css(&vars))
                    .unwrap_or_default();
                live_theme_css.set(css);
            }
            utils::sleep(std::time::Duration::from_millis(LIVE_THEME_POLL_MS)).await;
        }
    });

    use_effect(move || {
        let css = live_theme_css.read().clone();
        let css_json = serde_json::to_string(&css).unwrap_or_else(|_| "\"\"".to_string());
        let _ = dioxus::document::eval(&format!(
            r#"(function(){{
                let el = document.getElementById('live-theme-style');
                if (!el) {{ el = document.createElement('style'); el.id = 'live-theme-style'; document.head.appendChild(el); }}
                el.textContent = {css_json};
            }})()"#
        ));
    });

    // Inject a user-picked UI font reactively, mirroring the custom-themes path
    // above: read the file, inline it as a data: URI, and swap the <style>'s text.
    let custom_font_path = use_memo(move || config.read().custom_font_path.clone());
    use_effect(move || {
        let path = custom_font_path.read().clone();
        spawn(async move {
            // Read + base64-encode on a blocking worker so a large font never
            // stalls the render thread this effect runs on.
            let css = tokio::task::spawn_blocking(move || {
                build_custom_font_css(&path).unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            let css_json = serde_json::to_string(&css).unwrap_or_else(|_| "\"\"".to_string());
            let _ = dioxus::document::eval(&format!(
                r#"(function(){{
                    let el = document.getElementById('custom-font-style');
                    if (!el) {{ el = document.createElement('style'); el.id = 'custom-font-style'; document.head.appendChild(el); }}
                    el.textContent = {css_json};
                }})()"#
            ));
        });
    });

    let theme_class = use_memo(move || {
        let theme = config.read().theme.clone();
        if theme == "album-art" {
            "theme-default".to_string()
        } else if theme == utils::live_theme::THEME_ID {
            // A palette can be partial, or not written yet, so the default sits
            // underneath to keep every var resolving. The injected `.theme-live`
            // block lands later in <head>, so it still wins.
            format!("theme-default theme-{theme}")
        } else {
            format!("theme-{theme}")
        }
    });

    let is_rtl = i18n::is_rtl();
    let dir = if is_rtl { "rtl" } else { "ltr" };
    let content_row_class = "flex flex-1 overflow-hidden";
    let update_banner_state = update_banner.read().clone();
    let update_banner_padding = if cfg!(target_os = "macos") {
        "pl-20 pr-4"
    } else {
        "px-4"
    };

    let background_style = use_memo(move || {
        let conf = config.read();
        if conf.theme == "album-art"
            && !conf.cover_art_background
            && conf.custom_background_path.is_empty()
        {
            utils::color::get_background_style(palette.read().as_deref())
        } else {
            "background-color: var(--color-black); background-image: none;".to_string()
        }
    });

    let cover_background = use_memo(move || {
        let conf = config.read();
        if !conf.custom_background_path.is_empty() {
            let path = std::path::PathBuf::from(&conf.custom_background_path);
            return utils::format_artwork_url(Some(&path)).map(|url| url.as_ref().to_string());
        }
        if conf.cover_art_background {
            let url = current_song_cover_url.read().clone();
            return (!url.is_empty()).then_some(url);
        }
        None
    });

    let reduce_animations = use_memo(move || config.read().reduce_animations);
    let active_source = use_memo(move || config.read().active_source.clone());
    let mut show_quick_search = use_signal(|| false);
    use_effect(move || {
        if !*show_quick_search.read() {
            let _ = dioxus::document::eval(
                "const el = document.getElementById('app-root'); if (el) el.focus();",
            );
        }
    });

    use_effect(move || {
        let mut ctrl = ctrl;
        spawn(async move {
            let mut eval = dioxus::document::eval(
                r#"(function(){
                    if (window.__kopuzSpaceHandler) {
                        document.removeEventListener('keydown', window.__kopuzSpaceHandler, true);
                    }
                    const isTextEntry = (el) => {
                        if (!el) return false;
                        if (el.isContentEditable) return true;
                        const tag = el.tagName;
                        return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
                    };
                    const handler = (e) => {
                        if (e.key !== ' ' && e.code !== 'Space') return;
                        if (e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return;
                        if (isTextEntry(e.target) || isTextEntry(document.activeElement)) return;
                        e.preventDefault();
                        e.stopPropagation();
                        if (e.repeat) return;
                        dioxus.send('toggle-play');
                    };
                    window.__kopuzSpaceHandler = handler;
                    document.addEventListener('keydown', handler, true);
                })()"#,
            );
            while let Ok(v) = eval.recv::<serde_json::Value>().await {
                if v.as_str() == Some("toggle-play") {
                    ctrl.toggle();
                }
            }
        });
    });

    rsx! {
        // we use this component here to prevent re-diffing to prevent warns in console
        StaticHeadAssets {}
        WindowsToolbarIconAssets {}

        div {
            id: "app-root",
            class: "relative z-0 flex flex-col h-screen text-white select-none overflow-x-hidden {theme_class}",
            // The activity draws edge to edge, so inset the whole column once here
            // instead of per element. `fixed` overlays escape it and carry their own.
            style: if cfg!(target_os = "android") {
                format!("{} padding-top: env(safe-area-inset-top);", background_style)
            } else {
                background_style.to_string()
            },
            dir: "{dir}",
            "data-platform": if cfg!(target_os = "android") { "android" } else { "desktop" },
            "data-reduce-animations": "{reduce_animations}",
            tabindex: "0",
            autofocus: true,
            onkeydown: move |evt| {
                use dioxus::prelude::Key;
                let key = evt.key();
                let mods = evt.modifiers();
                if key == Key::Escape {
                    is_fullscreen.set(false);
                    if *compact_mode.read() {
                        compact_mode.set(false);
                    }
                } else if (mods.meta() || mods.ctrl())
                    && matches!(&key, Key::Character(s) if s.eq_ignore_ascii_case("m"))
                {
                    let c = *compact_mode.read();
                    compact_mode.set(!c);
                    evt.prevent_default();
                } else if (mods.meta() || mods.ctrl())
                    && matches!(&key, Key::Character(s) if s.eq_ignore_ascii_case("k"))
                {
                    let c = *show_quick_search.read();
                    show_quick_search.set(!c);
                    evt.prevent_default();
                }
            },
            if let Some(cover) = cover_background() {
                CoverArtBackground { cover }
            }
            if cfg!(any(target_os = "linux", target_os = "windows")) {
                div { dir: "ltr", Titlebar {} }
            }

            if cfg!(target_os = "linux") {
                ResizeHandles {}
            }

            if active_source().is_local() {
                if let Some(file) = scan_current_file.read().clone() {
                    div {
                        class: "flex-shrink-0",
                        div {
                            class: "h-[2px] bg-white/5 overflow-hidden",
                            div { class: "h-full w-1/4 bg-[var(--color-primary,#6366f1)] animate-scan" }
                        }
                        div {
                            class: "px-3 py-[3px] flex items-center gap-2 bg-black/30 border-b border-white/5",
                            i { class: "fa-solid fa-compact-disc fa-spin text-[9px] text-white/30 flex-shrink-0" }
                            span {
                                class: "text-[10px] text-white/35 font-mono truncate",
                                if file.is_empty() {
                                    "Scanning library…"
                                } else {
                                    "{file}"
                                }
                            }
                        }
                    }
                }
            }

            // Only show playback errors when the active server is YouTube
            // Music — other backends (Jellyfin/Subsonic/Custom) surface
            // their own errors via the settings popup, and a lingering YT
            // error from a previous session shouldn't haunt a switched-to
            // server.
            if frontend_sources.read().iter().any(|source| {
                source.active
                    && matches!(
                        source.service,
                        Some(api::MusicService::YtMusic | api::MusicService::Spotify)
                    )
            })
            {
                if let Some(msg) = ctrl.playback_error.read().clone() {
                    div {
                        class: "flex-shrink-0",
                        div {
                            class: "flex items-center justify-between gap-3 px-4 py-2 bg-rose-500/15 border-b border-rose-500/20 text-rose-200 text-sm",
                            div {
                                class: "flex items-center gap-2 whitespace-pre-line",
                                i { class: "fa-solid fa-triangle-exclamation text-xs" }
                                span { "{msg}" }
                            }
                            button {
                                class: "opacity-50 hover:opacity-100 transition-opacity p-1",
                                onclick: move |_| ctrl.playback_error.set(None),
                                i { class: "fa-solid fa-xmark text-xs" }
                            }
                        }
                    }
                }
            }

            if let Some(is_offline) = *network_banner.read() {
                div {
                    class: "flex-shrink-0",
                    div {
                        class: if is_offline {
                            "flex items-center justify-between gap-3 px-4 py-2 bg-amber-500/15 border-b border-amber-500/20 text-amber-300 text-sm"
                        } else {
                            "flex items-center justify-between gap-3 px-4 py-2 bg-emerald-500/15 border-b border-emerald-500/20 text-emerald-300 text-sm"
                        },
                        div {
                            class: "flex items-center gap-2",
                            i { class: if is_offline { "fa-solid fa-wifi-slash text-xs" } else { "fa-solid fa-wifi text-xs" } }
                            span {
                                if is_offline {
                                    "No internet connection — switched to offline mode"
                                } else {
                                    "Back online — switched to server mode"
                                }
                            }
                            if is_offline {
                                button {
                                    class: "ml-2 text-xs underline opacity-70 hover:opacity-100 transition-opacity",
                                    onclick: move |_| {
                                        offline_mode.set(false);
                                        network_banner.set(None);
                                    },
                                    "Keep server mode"
                                }
                            }
                        }
                        button {
                            class: "opacity-50 hover:opacity-100 transition-opacity p-1",
                            onclick: move |_| network_banner.set(None),
                            i { class: "fa-solid fa-xmark text-xs" }
                        }
                    }
                }
            }

            if let Some(update) = update_banner_state.clone() {
                div {
                    class: "flex-shrink-0",
                    div {
                        class: "flex items-center justify-between gap-3 {update_banner_padding} py-2 bg-sky-500/15 border-b border-sky-500/20 text-sky-200 text-sm",
                        div {
                            class: "flex items-center gap-2",
                            i { class: "fa-solid fa-download text-xs" }
                            span { class: "font-medium", "{i18n::t(\"update_available\")} - " }
                            span { "{i18n::t_with(\"update_banner_message\", &[(\"version\", update.version.clone())])}" }
                            if !cfg!(target_os = "android") {
                                button {
                                    class: "ml-2 text-xs underline opacity-80 hover:opacity-100 transition-opacity",
                                    onclick: {
                                        let release_url = update.release_url.clone();
                                        move |_| {
                                            #[cfg(not(target_os = "android"))]
                                            if let Err(e) = webbrowser::open(&release_url) {
                                                tracing::error!("Failed to open release page: {}", e);
                                            }
                                            #[cfg(target_os = "android")]
                                            let _ = &release_url;
                                        }
                                    },
                                    "{i18n::t(\"view_release\")}"
                                }
                            }
                        }
                        button {
                            class: "opacity-50 hover:opacity-100 transition-opacity p-1",
                            onclick: move |_| update_banner.set(None),
                            i { class: "fa-solid fa-xmark text-xs" }
                        }
                    }
                }
            }

            if config.read().player_bar_position == config::PlayerBarPosition::Top {
                Bottombar {
                    config,
                    current_song_cover_url: current_song_cover_url,
                    current_song_title: current_song_title,
                    current_song_artist: current_song_artist,
                    is_playing: is_playing,
                    is_fullscreen: is_fullscreen,
                    current_song_duration: current_song_duration,
                    current_song_progress: current_song_progress,
                    queue: queue,
                    current_queue_index: current_queue_index,
                    volume: volume,
                    persisted_volume: persisted_volume,
                    is_rightbar_open: is_rightbar_open,
                    is_devices_open: is_devices_open,
                }
            }
            div {
                class: "{content_row_class}",
                ontouchstart: move |evt| open_swipe.start(&evt),
                ontouchmove: move |evt| open_swipe.update(&evt),
                ontouchend: on_open_swipe,
                ontouchcancel: move |_| open_swipe.reset(),
                Sidebar {
                    current_route,
                    on_navigate: move |route| {
                        if route == Route::Album {
                            selected_album_id.set(String::new());
                        }
                        if route == Route::Artist {
                            selected_artist_name.set(String::new());
                            selected_artist_channel_id.set(None);
                        }
                        current_route.set(route);
                    }
                }
                div {
                    id: "main-scroll-area",
                    class: if cfg!(target_os = "android") { "flex-1 min-h-0 flex flex-col overflow-hidden relative" } else { "flex-1 overflow-y-auto" },
                    onscroll: move |evt| {
                        let pos = evt.scroll_top();
                        let route = *current_route.peek();
                        let album_sel = selected_album_id.peek().clone();
                        let artist_sel = selected_artist_name.peek().clone();
                        match route {
                            Route::Album if !album_sel.is_empty() => {
                                detail_scroll_positions
                                    .write()
                                    .insert(format!("album:{album_sel}"), pos);
                            }
                            Route::Artist if !artist_sel.is_empty() => {
                                detail_scroll_positions
                                    .write()
                                    .insert(format!("artist:{artist_sel}"), pos);
                            }
                            _ => {
                                scroll_positions.write().insert(route, pos);
                            }
                        }
                    },

                    if cfg!(target_os = "android") {
                        {
                            let is_details = match *current_route.read() {
                                Route::Album => !selected_album_id.read().is_empty(),
                                Route::Artist => !selected_artist_name.read().is_empty(),
                                Route::Playlists => selected_playlist_id.read().is_some(),
                                _ => false,
                            };
                            let page_title = match *current_route.read() {
                                Route::Home => i18n::t("home"),
                                Route::Search => i18n::t("search"),
                                Route::Library => i18n::t("library"),
                                Route::Album => if is_details { i18n::t("album") } else { i18n::t("albums") },
                                Route::Artist => if is_details { i18n::t("artist") } else { i18n::t("artists") },
                                Route::Playlists => i18n::t("playlists"),
                                Route::Favorites => i18n::t("favorites"),
                                Route::Settings => i18n::t("settings"),
                                _ => i18n::t("home"),
                            };
                            let has_image_background = config.read().cover_art_background
                                || !config.read().custom_background_path.is_empty();
                            rsx! {
                                div { class: if has_image_background { "shrink-0 z-[60] bg-black/30 backdrop-blur-xl border-b border-white/5 flex items-center h-11 px-3" } else { "shrink-0 z-[60] bg-black/60 backdrop-blur-2xl border-b border-white/5 flex items-center h-11 px-3 shadow-xl" },
                                    if is_details {
                                        button {
                                            class: "w-10 h-10 flex items-center justify-center rounded-xl bg-white/5 text-white active:scale-95 transition-all border border-white/10",
                                            onclick: move |_| nav_ctrl.go_back(),
                                            i { class: "fa-solid fa-arrow-left text-lg" }
                                        }
                                    } else {
                                        button {
                                            class: "w-10 h-10 flex items-center justify-center rounded-xl bg-white/5 text-white active:scale-95 transition-all border border-white/10",
                                            onclick: move |_| is_sidebar_collapsed.toggle(),
                                            i { class: "fa-solid fa-bars text-lg" }
                                        }
                                    }
                                    div { class: "flex-1 flex justify-center pr-10",
                                        h2 {
                                            class: "text-[13px] font-black tracking-[0.2em] text-white/90 uppercase",
                                            style: "font-family: 'kopuz-custom-font', 'JetBrains Mono', monospace;",
                                            "{page_title}"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    div { class: if cfg!(target_os = "android") { "relative flex-1 min-h-0 overflow-y-auto" } else { "contents" },
                    match *current_route.read() {
                        Route::Home => rsx! {
                            pages::home::Home {
                                on_select_album: move |id: String| {
                                    selected_album_id.set(id);
                                    current_route.set(Route::Album);
                                },
                                on_play_album: move |id: String| {
                                    // Play only — navigation is `on_select_album`'s
                                    // job (the play buttons even stop_propagation to
                                    // avoid the card's open-album click). Key on the
                                    // active source, not an id-prefix sniff —
                                    // Subsonic/Custom album ids carry their own
                                    // prefixes and Home only emits the active
                                    // source's ids anyway.
                                    let api = api_for_play_album.clone();
                                    spawn(async move {
                                        let mut tracks = api
                                            .album_tracks(
                                                id,
                                                api::Page {
                                                    offset: 0,
                                                    limit: u32::MAX,
                                                },
                                            )
                                            .await
                                            .map(|page| {
                                                page.items
                                                    .into_iter()
                                                    .map(hooks::use_db_queries::track_from_api)
                                                    .collect::<Vec<_>>()
                                            })
                                            .unwrap_or_default();
                                        if !tracks.is_empty() {
                                            tracks.sort_by(|a, b| {
                                                let disc_cmp = a.disc_number.unwrap_or(1).cmp(&b.disc_number.unwrap_or(1));
                                                if disc_cmp == std::cmp::Ordering::Equal {
                                                    a.track_number.unwrap_or(0).cmp(&b.track_number.unwrap_or(0))
                                                } else {
                                                    disc_cmp
                                                }
                                            });
                                            ctrl.play_queue_at(tracks, 0);
                                        }
                                    });
                                },
                                on_select_playlist: move |id: String| {
                                    selected_playlist_id.set(Some(id));
                                    current_route.set(Route::Playlists);
                                },
                                on_search_artist: move |artist: String| {
                                    selected_artist_name.set(artist);
                                    selected_artist_channel_id.set(None);
                                    current_route.set(Route::Artist);
                                }
                            }
                        },
                        Route::Discover => rsx! {
                            pages::server::discover::DiscoverPage {
                                on_select_album: move |id: String| {
                                    selected_album_id.set(id);
                                    current_route.set(Route::Album);
                                },
                                on_select_playlist: move |(id, title): (String, String)| {
                                    discover_selected_playlist_id.set(Some(id));
                                    discover_selected_playlist_title.set(Some(title));
                                    current_route.set(Route::DiscoverPlaylist);
                                },
                                on_open_artist: move |(cid, name): (String, String)| {
                                    selected_artist_channel_id.set(Some(cid));
                                    selected_artist_name.set(name);
                                    current_route.set(Route::Artist);
                                },
                                on_search_artist: move |name: String| {
                                    search_query.set(name);
                                    current_route.set(Route::Search);
                                },
                            }
                        },
                        Route::DiscoverPlaylist => rsx! {
                            pages::server::discover::DiscoverPlaylistDetail {
                                selected_playlist_id: discover_selected_playlist_id,
                                selected_playlist_title: discover_selected_playlist_title,
                                on_back: move |_| nav_ctrl.go_back(),
                            }
                        },
                        Route::Search => rsx! {
                            pages::search::Search {
                                config: config,
                                search_query: search_query,
                                            is_playing: is_playing,
                                current_playing: current_playing,
                                current_song_cover_url: current_song_cover_url,
                                current_song_title: current_song_title,
                                current_song_artist: current_song_artist,
                                current_song_duration: current_song_duration,
                                current_song_progress: current_song_progress,
                                queue: queue,
                                current_queue_index: current_queue_index,
                                on_select_album: move |id: String| {
                                    selected_album_id.set(id);
                                    current_route.set(Route::Album);
                                },
                            }
                        },
                        Route::Library => rsx! {
                            pages::library::LibraryPage {
                                config: config,
                                on_rescan: move |_| *trigger_rescan.write() += 1,
                                            is_playing: is_playing,
                                current_playing: current_playing,
                                current_song_cover_url: current_song_cover_url,
                                current_song_title: current_song_title,
                                current_song_artist: current_song_artist,
                                current_song_duration: current_song_duration,
                                current_song_progress: current_song_progress,
                                queue: queue,
                                current_queue_index: current_queue_index,
                            }
                        },
                        Route::Album => rsx! {
                            pages::album::Album {
                                config: config,
                                album_id: selected_album_id,
                                queue: queue,
                                current_queue_index: current_queue_index,
                            }
                        },
                        Route::Artist => {
                            // YT Music gets the rich YT-backed profile (banner, top songs, albums, related) ONLY when an artist is actually selected. The Artists sidebar tab / back-to-list navigation
                            //  lands with both signals
                            // cleared — fall through to the library-driven
                            // grid in that case (populated on YT from followed
                            // artists + liked-song artists by the library
                            // sync). Local / Jellyfin / Subsonic keep the
                            // library-driven page in all cases.
                            // Route on the active source's capability, not the
                            // configured server: a YT server can be configured while
                            // Local is active, and the rich remote profile must not
                            // hijack the local artist page.
                            let remote_profile =
                                active_caps().artists == api::ArtistPresentation::Remote;
                            let has_selection = !selected_artist_name.read().is_empty()
                                || selected_artist_channel_id.read().is_some();
                            if remote_profile && has_selection {
                                rsx! {
                                    pages::server::discover::DiscoverArtistPage {
                                        selected_artist_id: selected_artist_channel_id,
                                        selected_artist_name: selected_artist_name,
                                        on_back: move |_| nav_ctrl.go_back(),
                                        on_select_album: move |id: String| {
                                            selected_album_id.set(id);
                                            current_route.set(Route::Album);
                                        },
                                        on_select_playlist: move |(id, title): (String, String)| {
                                            discover_selected_playlist_id.set(Some(id));
                                            discover_selected_playlist_title.set(Some(title));
                                            current_route.set(Route::DiscoverPlaylist);
                                        },
                                        on_open_artist: move |(cid, name): (String, String)| {
                                            selected_artist_channel_id.set(Some(cid));
                                            selected_artist_name.set(name);
                                        },
                                        on_search_artist: move |name: String| {
                                            search_query.set(name);
                                            current_route.set(Route::Search);
                                        },
                                    }
                                }
                            } else {
                                rsx! {
                                    pages::artist::Artist {
                                        config: config,
                                        artist_name: selected_artist_name,
                                                            on_navigate: move |album_id| {
                                            selected_album_id.set(album_id);
                                            current_route.set(Route::Album);
                                        },
                                        is_playing: is_playing,
                                        current_playing: current_playing,
                                        current_song_cover_url: current_song_cover_url,
                                        current_song_title: current_song_title,
                                        current_song_artist: current_song_artist,
                                        current_song_duration: current_song_duration,
                                        current_song_progress: current_song_progress,
                                        queue: queue,
                                        current_queue_index: current_queue_index,
                                    }
                                }
                            }
                        },
                        Route::Favorites => rsx! {
                            pages::favorites::FavoritesPage {
                                config,
                                is_playing,
                                current_playing,
                                current_song_cover_url,
                                current_song_title,
                                current_song_artist,
                                current_song_duration,
                                current_song_progress,
                                queue,
                                current_queue_index,
                            }
                        },
                        Route::Playlists => rsx! {
                            pages::playlists::PlaylistsPage {
                                config: config,
                                selected_playlist_id: selected_playlist_id,
                            }
                        },
                        Route::Activity => rsx! {
                          pages::activity::Activity {
                              config: config,
                          }
                        },
                        Route::Radio => rsx! {
                            pages::radio::Radio {
                                config: config,
                            }
                        },
                        #[cfg(not(target_os = "android"))]
                        Route::Ytdlp => rsx! { pages::ytdlp::YtdlpPage { config } },
                        Route::Settings => rsx! { pages::settings::Settings { config } },
                        #[cfg(not(target_os = "android"))]
                        Route::ThemeEditor => rsx! { pages::theme_editor::ThemeEditorPage { config } },
                    }
                    }
                }
                Rightbar {
                    is_rightbar_open: is_rightbar_open,
                    width: rightbar_width,
                    current_song_duration: current_song_duration,
                    current_song_progress: current_song_progress,
                    queue: queue,
                    current_queue_index: current_queue_index,
                    current_song_title: current_song_title,
                    current_song_artist: current_song_artist,
                    current_song_album: current_song_album,
                }
                SpotifyDevicesPanel {
                    is_devices_open: is_devices_open,
                    is_rightbar_open: is_rightbar_open,
                }
            }
            Fullscreen {
                is_playing: is_playing,
                is_fullscreen: is_fullscreen,
                current_song_duration: current_song_duration,
                current_song_progress: current_song_progress,
                queue: queue,
                current_song_album: current_song_album,
                current_queue_index: current_queue_index,
                current_song_title: current_song_title,
                current_song_bitrate: current_song_bitrate,
                current_song_artist: current_song_artist,
                current_song_cover_url: current_song_cover_url,
                volume: volume,
                persisted_volume: persisted_volume,
                palette: palette,
            }
            DownloadOverlay { queue: download_queue }
            CompactPlayer {}
            if *show_quick_search.read() {
                QuickSearch {
                    show: show_quick_search,
                    on_play: move |(track, fallback): (reader::Track, Vec<reader::Track>)| {
                        let api = consume_context::<Arc<dyn api::KopuzApi>>();
                        let filter = api::TrackFilter {
                            sort: hooks::use_db_queries::track_sort_fields(
                                &config.peek().library_sort,
                            ),
                            ..Default::default()
                        };
                        spawn(async move {
                            let all = api
                                .tracks(
                                    filter,
                                    api::Page {
                                        offset: 0,
                                        limit: u32::MAX,
                                    },
                                )
                                .await
                                .map(|page| {
                                    page.items
                                        .into_iter()
                                        .map(hooks::use_db_queries::track_from_api)
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            if let Some(idx) = all.iter().position(|t| t.id == track.id) {
                                ctrl.play_queue_at(all, idx);
                            } else if let Some(idx) = fallback.iter().position(|t| t.id == track.id) {
                                ctrl.play_queue_at(fallback, idx);
                            }
                        });
                    },
                }
            }
            if config.read().player_bar_position == config::PlayerBarPosition::Bottom {
                Bottombar {
                    config,
                    current_song_cover_url: current_song_cover_url,
                    current_song_title: current_song_title,
                    current_song_artist: current_song_artist,
                    is_playing: is_playing,
                    is_fullscreen: is_fullscreen,
                    current_song_duration: current_song_duration,
                    current_song_progress: current_song_progress,
                    queue: queue,
                    current_queue_index: current_queue_index,
                    volume: volume,
                    persisted_volume: persisted_volume,
                    is_rightbar_open: is_rightbar_open,
                    is_devices_open: is_devices_open,
                }
            }
        }
    }
}
