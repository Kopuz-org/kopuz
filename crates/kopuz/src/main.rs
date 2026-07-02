use components::{
    bottombar::Bottombar, compact_player::CompactPlayer, download_overlay::DownloadOverlay,
    fullscreen::Fullscreen, rightbar::Rightbar, sidebar::Sidebar, titlebar::Titlebar,
};
#[cfg(not(target_os = "android"))]
use dioxus::desktop::tao::dpi::LogicalSize;
#[cfg(target_os = "macos")]
use dioxus::desktop::tao::platform::macos::WindowBuilderExtMacOS;
#[cfg(target_os = "windows")]
use dioxus::desktop::tao::platform::windows::WindowExtWindows;
use dioxus::prelude::*;
use discord_presence::Presence;
use kopuz_route::Route;
use pages::server::download_manager::DownloadQueue;
use player::player::Player;
use queue_state::PersistedQueueState;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::Instrument;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;

mod app_db;
mod app_lifecycle;
#[cfg(not(target_os = "android"))]
mod artwork_protocol;
#[cfg(not(target_os = "android"))]
mod chrome_trace;
mod desktop_shell;
mod legacy;
mod logging;
#[cfg(not(target_os = "android"))]
mod pot_minter;
mod queue_state;
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

static PRESENCE: std::sync::OnceLock<Option<Arc<Presence>>> = std::sync::OnceLock::new();

fn main() {
    #[cfg(not(target_os = "android"))]
    {
        let log_dir = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
            .map(|dirs| dirs.cache_dir().join("logs"))
            .unwrap_or_else(|| std::path::PathBuf::from("logs"));
        let _ = std::fs::create_dir_all(&log_dir);

        // Read the persisted tracing toggle from the DB before the app (and its
        // config Signal) exists — the subscriber is built once here, so the
        // setting is applied at startup. Missing DB/blob defaults to off.
        let config_tracing_enabled = db::peek_config(&db::default_db_path())
            .map(|c| c.tracing_enabled)
            .unwrap_or(false);

        // Guards live in a global inside `logging`; flushed by
        // logging::shutdown() after launch returns or on Ctrl+C.
        logging::init(&log_dir, config_tracing_enabled);

        legacy::migrate_locations();

        let _ = app_db::DB_HANDLE.set(app_db::init_blocking());

        let presence: Option<Arc<Presence>> = match Presence::new("1470087339639443658") {
            Ok(p) => {
                tracing::info!("Discord presence connected");
                Some(Arc::new(p))
            }
            Err(e) => {
                tracing::warn!("Failed to connect to Discord: {e}");
                None
            }
        };

        PRESENCE.set(presence).ok();

        #[cfg(target_os = "macos")]
        {
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
            let initial_titlebar_mode = desktop_shell::read_titlebar_mode_from_disk();
            window = window.with_decorations(initial_titlebar_mode == config::TitlebarMode::System);
        }

        let webview_data_dir = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
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
            // Anon PoToken minter: stand up the hidden music.youtube.com webview
            // once we have the event-loop target (issue #349).
            .with_custom_event_handler(|_event, _target| {
                crate::pot_minter::install_if_wanted(_target);
                crate::pot_minter::pump();
            })
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
        // Window closed → flush the log file tail + finalize the
        // chrome trace's closing bracket.
        logging::shutdown();
    }

    #[cfg(target_os = "android")]
    {
        // JNI media session + classloader cache. Player::new() also calls this (idempotent
        // OnceLock), but doing it up front means the session exists before first playback.
        player::systemint::init();

        let _ = app_db::DB_HANDLE.set(app_db::init_blocking());

        let config = dioxus::mobile::Config::new()
            .with_background_color((0, 0, 0, 255))
            // artwork://local?p=<percent-encoded-absolute-path> — the Android WebView mostly
            // receives base64 data URLs from utils, but keep a synchronous handler for any
            // code path that still emits artwork:// URLs.
            .with_custom_protocol("artwork".to_string(), |_headers, request| {
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
                } else if decoded_path.starts_with('~') {
                    if let Ok(home) = std::env::var("HOME") {
                        decoded_path = decoded_path.replacen('~', &home, 1);
                    }
                }

                let read_result =
                    std::fs::read(std::path::Path::new(&decoded_path)).or_else(|_| {
                        if decoded_path.strip_prefix('/').is_some() {
                            std::fs::read(std::path::Path::new(&decoded_path[1..]))
                        } else {
                            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
                        }
                    });

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

    app_lifecycle::use_webview_decipher_engine();

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
            let path = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
                .map(|dirs| dirs.cache_dir().to_path_buf())
                .unwrap_or_else(|| std::path::Path::new("./cache").to_path_buf());
            let _ = std::fs::create_dir_all(&path);
            path
        }
    });
    // ROOT-owned: detached tasks (download workers, close-flush) read/write
    // these after the spawning page — and in principle this component — is
    // gone; owning them at ROOT keeps Dioxus's cross-scope lint honest.
    let mut config = use_hook(|| Signal::new_in_scope(config::AppConfig::default(), ScopeId::ROOT));
    let db = app_db::DB_HANDLE
        .get()
        .cloned()
        .expect("db initialized in main before launch");
    // The UI reads through a read-only handle and operates through the cached
    // source handles below — it never gets a full `Db`, so it cannot reach a
    // write method (those live on `Storage`, not `ReadStore`). The full `Db` is
    // provided to the UI tree ONLY in debug builds, where the debug DB panel
    // needs it.
    use_context_provider(|| db.reads());
    #[cfg(debug_assertions)]
    use_context_provider(|| db.clone());
    hooks::db_reactivity::use_generations_provider();

    // The active source — the single source the UI operates through — resolved
    // ONCE and held, so call sites read this shared handle instead of rebuilding
    // the source (and, for a server, a fresh HTTP client) per operation. Rotation
    // isn't mutation: an identity change (source switch or cred change) rebuilds
    // and swaps the `Arc`. Capability gating guarantees an op is only reachable
    // when the active source can do it (no local-only op offered under a server).
    let active_source = {
        let db_init = db.clone();
        let mut active_source = use_signal(move || {
            ::server::source::ActiveSource::from(::server::source::active(
                db_init.clone(),
                &config.peek(),
            ))
        });
        // Only the resolution-relevant slice of config; a volume/theme change
        // must not rebuild the client. `Memo`'s `PartialEq` dedup gates the effect.
        let identity = use_memo(move || {
            (
                config.read().active_source.clone(),
                config.read().server.clone(),
            )
        });
        let db_eff = db.clone();
        use_effect(move || {
            let _ = identity.read();
            active_source.set(::server::source::ActiveSource::from(
                ::server::source::active(db_eff.clone(), &config.peek()),
            ));
        });
        use_context_provider(|| active_source)
    };

    // Capabilities of the active source — drives source-agnostic routing (e.g.
    // which artist view to render) without hardcoding services in the router.
    let active_caps = use_memo(move || active_source.read().capabilities());
    // Start the PoToken minter whenever a YouTube Music server is active — not
    // just anon. A *signed-in but non-Premium* account streams the same 251 as
    // anon and also needs a content pot for deep ranges; only true Premium
    // subscribers (itag 774) are pot-exempt, and we can't know that until a
    // track resolves. So run the minter for any YtMusic session; Premium just
    // leaves it idle. Reactive: fires when config loads or the server changes.
    #[cfg(not(target_os = "android"))]
    use_effect(move || {
        let yt_active = config
            .read()
            .server
            .as_ref()
            .is_some_and(|s| s.service == config::MusicService::YtMusic);
        if yt_active {
            crate::pot_minter::request();
        }
    });
    hooks::use_sync_task::use_sync_task(config, db.clone());
    let mut initial_load_done = use_signal(|| false);
    #[allow(unused_variables)]
    let cover_cache = use_memo(move || cache_dir().join("covers"));
    let _ = std::fs::create_dir_all(cover_cache());
    let download_queue = use_hook(|| Signal::new_in_scope(DownloadQueue::default(), ScopeId::ROOT));
    let download_progress =
        use_hook(|| Signal::new_in_scope(::server::DownloadProgress::default(), ScopeId::ROOT));
    pages::server::download_manager::register_progress_signal(download_progress);
    let mut trigger_rescan = use_signal(|| 0);
    let trigger_cover_reextract = use_signal(|| 0);
    use_context_provider(|| hooks::CoverReextractTrigger(trigger_cover_reextract));
    // Applies detached yt-dlp completions (history + rescan) in this scope —
    // the job drivers outlive the downloads page and can't write these.
    pages::ytdlp_jobs::use_ytdlp_completion_sink(config, trigger_rescan);
    let mut last_scan_key = use_signal(|| None::<String>);
    let mut scan_current_file = use_signal(|| Option::<String>::None);
    let current_playing = use_signal(|| 0);
    let mut player = use_signal(Player::new);
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
    let mut configured_music_dirs = use_signal(|| config.peek().music_directory.clone());

    let is_playing = use_signal(|| false);
    let mut is_fullscreen = use_signal(|| false);
    let mut compact_mode = use_signal(|| false);
    let is_rightbar_open = use_signal(|| false);
    let rightbar_width = use_signal(|| 320usize);
    let mut palette = use_signal(|| Option::<Vec<utils::color::Color>>::None);
    // Config is the one remaining whole-value save: persisting a default that
    // exists only because the LOAD FAILED would wipe real settings/servers, so
    // its save stays disarmed unless the load demonstrably succeeded (a fresh
    // empty DB still counts). Library/playlists/favorites have no such flag
    // anymore — they're targeted per-row writes, never full-replace.
    let mut config_loaded_ok = use_signal(|| false);

    let mut pending_queue_state_snapshot = use_signal(|| None::<PersistedQueueState>);
    let mut pending_queue_state_revision = use_signal(|| 0u64);

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
            if let Some(db) = app_db::DB_HANDLE.get() {
                let db = db.clone();
                // None = the queue is empty (a cleared queue must persist as
                // empty, not resurrect) — but only once the saved queue has
                // actually been restored, else a quit during startup would
                // wipe it.
                let queue_snap = (*initial_load_done.peek()).then(|| {
                    pending_queue_state_snapshot
                        .peek()
                        .clone()
                        .map(queue_state::snapshot)
                        .unwrap_or_default()
                });
                // Library/playlists/favorites need no flush — every mutation
                // already committed as a targeted write when it happened.
                let cfg = (*config_loaded_ok.peek()).then(|| {
                    let mut cfg = config.peek().clone();
                    cfg.volume = *volume.peek();
                    cfg
                });
                let _ = std::thread::spawn(move || {
                    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    else {
                        return;
                    };
                    rt.block_on(async move {
                        if let Some(snap) = queue_snap
                            && let Err(e) = db.save_queue(&snap).await
                        {
                            tracing::warn!(error = %e, "queue flush on close failed");
                        }
                        if let Some(cfg) = cfg {
                            let _ = db.save_config(&cfg).await;
                        }
                    });
                })
                .join();
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
        let _ = dioxus::document::eval(
            r#"document.addEventListener('error',function(e){
                var t=e.target;
                if(t.tagName==='IMG'&&!t.dataset.fallback&&t.src){
                    t.dataset.fallback='1';
                    t.src='data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%27400%27 height=%27400%27 viewBox=%270 0 400 400%27%3E%3Crect width=%27400%27 height=%27400%27 fill=%27%231e1b2e%27/%3E%3Ccircle cx=%27200%27 cy=%27180%27 r=%2770%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27/%3E%3Cpath d=%27M155 280 Q200 240 245 280%27 fill=%27none%27 stroke=%27%233d3466%27 stroke-width=%276%27 stroke-linecap=%27round%27/%3E%3C/svg%3E';
                }
            },true);"#,
        );
    });

    use_effect(move || {
        let url = current_song_cover_url.read().clone();
        if !url.is_empty() {
            spawn(
                async move {
                    if let Some(colors) = utils::color::get_palette_from_url(&url).await {
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
        let next_dirs = config.read().music_directory.clone();
        if *configured_music_dirs.peek() != next_dirs {
            configured_music_dirs.set(next_dirs);
        }
    });

    let presence = PRESENCE.get().cloned().flatten();
    provide_context(presence.clone());

    let mut station_registry = use_signal(radio::registry::StationRegistry::new);
    provide_context(station_registry);

    let mut last_radio_registry_key = use_signal(|| None::<String>);

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        let registry_paths: Vec<String> = config
            .read()
            .radio_registries
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.url.clone())
            .collect();

        let key = registry_paths.join(",");
        if *last_radio_registry_key.peek() == Some(key.clone()) {
            return;
        }
        last_radio_registry_key.set(Some(key));

        spawn(
            async move {
                let mut new_registry = radio::registry::StationRegistry::new();
                let mut import_count = 0;

                for path in registry_paths {
                    match new_registry.import_registry(&path).await {
                        Ok(_) => import_count += 1,
                        Err(e) => tracing::warn!("Failed to import registry from {}: {}", path, e),
                    }
                }

                station_registry.set(new_registry);

                if import_count > 0 {
                    tracing::info!("Imported {} external radio registries", import_count);
                }
            }
            .instrument(tracing::info_span!("radio.registry_load")),
        );
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
    let fetched_artist_images: Signal<std::collections::HashMap<String, String>> =
        use_signal(std::collections::HashMap::new);
    let mut search_query = use_signal(String::new);
    let mut last_server_playlist_key = use_signal(|| None::<String>);
    let mut server_playlist_key_initialized = use_signal(|| false);
    let mut queue = use_signal(Vec::<reader::Track>::new);
    let current_queue_index = use_signal(|| 0usize);

    let mut network_banner: Signal<Option<bool>> = use_signal(|| None);
    let mut update_banner: Signal<Option<updates::AvailableUpdate>> = use_signal(|| None);
    let mut did_check_updates = use_signal(|| false);
    let mut ctrl = hooks::use_player_controller(
        player,
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
        db.clone(),
    );

    // Generations handle the rescan task bumps after writing scanned tracks/albums,
    // so the DB-backed query hooks re-run and the UI refreshes.
    let gens_for_albums = hooks::db_reactivity::use_generations();

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        // Server identity excludes access_token: tokens rotate without making it a
        // different account, but their rotation would otherwise reset playback.
        let current_server_key = {
            let conf = config.read();
            conf.server.as_ref().map(|server| {
                format!(
                    "{:?}|{}|{}",
                    server.service,
                    server.url,
                    server.user_id.as_deref().unwrap_or_default(),
                )
            })
        };

        if !*server_playlist_key_initialized.read() {
            last_server_playlist_key.set(current_server_key);
            server_playlist_key_initialized.set(true);
            return;
        }

        if *last_server_playlist_key.read() != current_server_key {
            last_server_playlist_key.set(current_server_key);
            selected_playlist_id.set(None);
            ctrl.reset_for_backend_switch();
            // Nothing to reload: pages query by source, so switching servers is
            // just a key change — every hook re-queries the new server's rows.
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
    let db_for_cfg_save = db.clone();
    use_future(move || {
        let db = db_for_cfg_save.clone();
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
                if let Err(e) = db
                    .save_config(&snapshot)
                    .instrument(tracing::info_span!("config.persist"))
                    .await
                {
                    tracing::error!("Failed to save config: {}", e);
                }
                utils::sleep(std::time::Duration::from_millis(STORE_SAVE_COOLDOWN_MS)).await;
            }
        }
    });

    // Keepalive is rearm-on-account-change, not rearm-on-every-config-
    // write. Re-running the effect on every config save would spawn
    // a fresh loop that immediately fires run_rotation, spamming
    // /verify_session a dozen times a minute on any settings churn.
    //
    // The signal stores the YT identity (a stable hash of the SAPISID
    // cookie) we currently have a loop running against. The effect
    // re-runs cheap, but only spawns a new loop when the identity
    // changes (sign-in, account switch). Sign-out clears the
    // identity and the running loop exits on its next tick.
    let mut yt_keepalive_identity = use_signal(|| None::<String>);
    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }
        let yt_cookies: Option<String> = config.read().server.as_ref().and_then(|s| {
            (s.service == config::MusicService::YtMusic)
                .then(|| s.access_token.clone())
                .flatten()
                .filter(|t| !t.is_empty())
        });
        let live_identity = yt_cookies
            .as_deref()
            .and_then(server::ytmusic::derive_user_id);
        if live_identity == *yt_keepalive_identity.peek() {
            return;
        }
        // Identity changed (fresh sign-in, account switch, or
        // sign-out): the previously-running loop (if any) will read
        // the new identity on its next tick and exit. Update the
        // tracked identity; spawn a fresh loop only if we still have
        // valid auth.
        yt_keepalive_identity.set(live_identity.clone());
        let Some(my_identity) = live_identity else {
            return;
        };
        spawn(async move {
            updates::run_rotation(config).await;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                if yt_keepalive_identity.peek().as_deref() != Some(my_identity.as_str()) {
                    return;
                }
                updates::run_rotation(config).await;
            }
        });
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

    use_effect(move || {
        if !*initial_load_done.read() {
            return;
        }

        let queue_snapshot = queue.read().clone();
        let shuffle_order_snapshot = ctrl.shuffle_order.read().clone();
        let shuffle_enabled_snapshot = *ctrl.shuffle.read();

        let queue_state = queue_state::build_snapshot(
            &queue_snapshot,
            *current_queue_index.read(),
            *current_song_progress.read(),
            *is_playing.read(),
            &shuffle_order_snapshot,
            shuffle_enabled_snapshot,
        );

        if *pending_queue_state_snapshot.peek() != queue_state {
            pending_queue_state_snapshot.set(queue_state);
            pending_queue_state_revision.with_mut(|revision| *revision += 1);
        }
    });

    let db_for_queue_save = db.clone();
    use_future(move || {
        let db = db_for_queue_save.clone();
        async move {
            let mut flushed_revision = 0u64;

            loop {
                let pending_revision = *pending_queue_state_revision.read();
                if pending_revision == flushed_revision {
                    utils::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }

                utils::sleep(std::time::Duration::from_millis(
                    queue_state::SAVE_DEBOUNCE_MS,
                ))
                .await;

                let latest_revision = *pending_queue_state_revision.read();
                if latest_revision != pending_revision {
                    continue;
                }

                let snapshot = pending_queue_state_snapshot.read().clone();
                queue_state::persist_snapshot(db.clone(), snapshot)
                    .instrument(tracing::info_span!("queue.persist"))
                    .await;
                flushed_revision = latest_revision;
            }
        }
    });

    let _is_offline = app_lifecycle::use_connectivity_probe(config, network_banner);

    let db_for_load = db.clone();
    use_hook(move || {
        {
            let db = db_for_load;
            let mut ctrl = ctrl;

            spawn(async move {
                // Everything loads from the DB — the converted source of truth.
                // The legacy JSON files are never read or written; a fresh DB
                // with no blob yet just yields the default config.
                // Startup loads ONLY config + queue — everything else is queried
                // on demand by the page hooks. Config marks itself loaded ONLY
                // on success: its save is the one remaining whole-value write,
                // and persisting a default born of a read failure would wipe
                // real settings/servers.
                let cfg_loaded = match db.load_config().await {
                    Ok(c) => {
                        config_loaded_ok.set(true);
                        c
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to load config from db — config saves disabled this session");
                        None
                    }
                };
                let queue_loaded = db.load_queue().await.ok();

                let cfg_loaded = cfg_loaded.unwrap_or_default();
                {
                    let loaded = cfg_loaded;
                    config.set(loaded.clone());
                    configured_music_dirs.set(loaded.music_directory.clone());
                    volume.set(loaded.volume);
                    persisted_volume.set(loaded.volume);
                    player.write().set_volume(loaded.volume);
                    player.write().set_channel_mode(loaded.channel_mode);
                    player.write().set_equalizer(loaded.equalizer.clone());
                    i18n::set_locale(&loaded.language);
                }

                // Local is the source of truth: no auto-switch to a server on
                // startup. An unselected source stays Local (the config default);
                // the user picks a server explicitly via the sidebar.

                if let Some(snap) = queue_loaded
                    && let Some(queue_state) = queue_state::sanitize(PersistedQueueState {
                        version: snap.version,
                        queue: snap.queue,
                        current_queue_index: snap.current_queue_index,
                        progress_secs: snap.progress_secs,
                        shuffle_order: snap.shuffle_order,
                        shuffle_enabled: snap.shuffle_enabled,
                    })
                {
                    ctrl.restore_queue_state(
                        queue_state.queue,
                        queue_state.current_queue_index,
                        queue_state.progress_secs,
                        queue_state.shuffle_order,
                        queue_state.shuffle_enabled,
                    );
                }

                initial_load_done.set(true);
                // Kick one reconcile shortly after startup so pending offline
                // likes from the previous session push now, not on the first
                // multi-minute interval.
                hooks::use_sync_task::nudge_activate();
            }.instrument(tracing::info_span!("startup.load")));
        }
    });

    let db_for_rescan = db.clone();
    let db_for_play_album = db.clone();
    use_effect(move || {
        // config_loaded_ok matters here: a defaulted config (load failure) has
        // an empty music_directory, and the no-dirs branch below prunes the
        // local library — which must never happen off phantom state.
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        let configured_dirs = configured_music_dirs.read().clone();
        let trigger = *trigger_rescan.read();
        let fetch_covers = config.peek().auto_fetch_covers;
        let fetch_strategy = config.peek().cover_fetch_strategy;
        let lastfm_key = {
            let key = config.peek().lastfm_api_key.trim().to_owned();
            (!key.is_empty()).then_some(key)
        };

        let scan_key = format!(
            "{}|{}",
            configured_dirs
                .iter()
                .map(|d| d.to_string_lossy())
                .collect::<Vec<_>>()
                .join(","),
            trigger,
        );
        if *last_scan_key.peek() == Some(scan_key.clone()) {
            return;
        }
        last_scan_key.set(Some(scan_key));

        // Scans aren't cancelled, so two can overlap (a root removed mid-scan
        // respawns this effect). Only the newest may persist — a stale scan's
        // upserts + prune would resurrect the removed root.
        static SCAN_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let epoch = SCAN_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let scan_is_current =
            move || SCAN_EPOCH.load(std::sync::atomic::Ordering::Relaxed) == epoch;

        let db_scan = db_for_rescan.clone();
        let gens_scan = gens_for_albums;
        spawn(async move {
            let db = db_scan;
            let gens = gens_scan;
            let configured_dirs = configured_dirs;
            let scannable_dirs: Vec<PathBuf> = configured_dirs
                .iter()
                .filter(|d| d.exists())
                .cloned()
                .collect();
            // Seed the scan working set from the DB (the scanner skips files it
            // already knows; album-merge keeps manual covers). One folder query
            // per root, deduped by key in case roots nest.
            // An errored seed must abort the scan: the keep-set fed to
            // prune_source comes from these, and defaulting to empty would
            // turn one transient DB error into a pruned library.
            let mut seed_tracks: Vec<reader::Track> = Vec::new();
            let mut seen_keys = std::collections::HashSet::new();
            for dir in &configured_dirs {
                let mut prefix = dir.to_string_lossy().into_owned();
                if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
                    prefix.push(std::path::MAIN_SEPARATOR);
                }
                let found = match db.folder_tracks(&prefix).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::error!(error = %e, root = %prefix, "rescan: seed query failed — aborting scan");
                        return;
                    }
                };
                for t in found {
                    if seen_keys.insert(t.id.key().into_owned()) {
                        seed_tracks.push(t);
                    }
                }
            }
            let seed_albums = match db.albums(&db::Source::Local).await {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(error = %e, "rescan: album seed failed — aborting scan");
                    return;
                }
            };
            let mut current_lib = reader::Library {
                root_paths: configured_dirs.clone(),
                tracks: seed_tracks,
                albums: seed_albums,
                ..Default::default()
            };

            if !configured_dirs.is_empty() {
                scan_current_file.set(Some(String::new()));

                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                spawn(async move {
                    while let Some(file) = rx.recv().await {
                        scan_current_file.set(Some(file));
                    }
                    scan_current_file.set(None);
                });

                let progress_cb: std::sync::Arc<dyn Fn(String) + Send + Sync> =
                    std::sync::Arc::new(move |file: String| {
                        let _ = tx.send(file);
                    });
                for dir in &scannable_dirs {
                    let _ = reader::scan_directory(
                        dir.clone(),
                        cover_cache(),
                        &mut current_lib,
                        progress_cb.clone(),
                    )
                    .await;
                }

                current_lib.tracks.retain(|t| {
                    let in_configured_root = configured_dirs
                        .iter()
                        .any(|d| t.id.local_path().is_some_and(|p| p.starts_with(d)));
                    let in_scannable_root = scannable_dirs
                        .iter()
                        .any(|d| t.id.local_path().is_some_and(|p| p.starts_with(d)));

                    in_configured_root
                        && (!in_scannable_root || t.id.local_path().is_some_and(|p| p.exists()))
                });

                let valid_album_ids: std::collections::HashSet<_> = current_lib
                    .tracks
                    .iter()
                    .map(|t| t.album_id.clone())
                    .collect();
                current_lib
                    .albums
                    .retain(|a| valid_album_ids.contains(&a.id));

                // Persist the scan directly: chunked upserts, prune what's gone,
                // bump so the page hooks re-query. No in-memory mirror.
                if !scan_is_current() {
                    tracing::info!("rescan superseded by a newer scan — discarding results");
                    return;
                }
                for chunk in current_lib.tracks.chunks(100) {
                    let _ = db.upsert_tracks(&db::Source::Local, chunk).await;
                    gens.bump_coalesced(hooks::db_reactivity::Table::Tracks);
                }
                let _ = db.upsert_albums(&db::Source::Local, &current_lib.albums).await;
                let keep_keys: Vec<String> = current_lib
                    .tracks
                    .iter()
                    .map(|t| t.id.key().into_owned())
                    .collect();
                let keep_albums: Vec<String> =
                    current_lib.albums.iter().map(|a| a.id.clone()).collect();
                if !scan_is_current() {
                    tracing::info!("rescan superseded mid-persist — skipping prune");
                    return;
                }
                let _ = db
                    .prune_source(&db::Source::Local, &keep_keys, &keep_albums)
                    .await;
                for (artist, img) in &current_lib.local_artist_images {
                    let p = img.to_string_lossy().into_owned();
                    let _ = db.set_artist_image(artist, "local", Some(&p)).await;
                }
                // Drop stored local artist images whose file disappeared (the
                // old scan rebuilt the whole map each pass, self-healing this).
                if let Ok((_, photos)) = db.artist_images().await {
                    for (artist, photo) in photos {
                        if let reader::ArtistImageRef::Local(path) = photo
                            && !path.exists()
                        {
                            let _ = db.set_artist_image(&artist, "local", None).await;
                        }
                    }
                }
                gens.bump(hooks::db_reactivity::Table::Tracks);
                gens.bump(hooks::db_reactivity::Table::Albums);

                if fetch_covers {
                    // Fetch missing covers in the background so the UI stays responsive.
                    // Passing `progress_cb` into the task keeps the scan-progress bar
                    // alive during fetching; it disappears automatically when the task ends.
                    // Albums that HAD no cover before the fetch get the fetched one
                    // written straight to the DB (manual covers were never in the
                    // missing set, so they can't be overwritten).
                    let lib_for_fetch = current_lib;
                    let db = db.clone();
                    spawn(async move {
                        let fetcher = reader::cover_fetcher::CoverFetcher::new(
                            cover_cache(),
                            fetch_strategy,
                            lastfm_key,
                            progress_cb,
                        );
                        let mut lib = lib_for_fetch;
                        let missing_before: std::collections::HashSet<String> = lib
                            .albums
                            .iter()
                            .filter(|a| {
                                a.cover_path.as_ref().is_none_or(|p| !p.exists())
                                    && !a.manual_cover
                            })
                            .map(|a| a.id.clone())
                            .collect();
                        let report = fetcher.fetch_missing_covers(&mut lib).await;
                        tracing::info!(
                            "Cover auto-fetch: {} found, {} missing, {} errors",
                            report.found,
                            report.missing,
                            report.errors,
                        );
                        let mut changed = false;
                        for album in lib.albums.iter() {
                            if !missing_before.contains(&album.id) {
                                continue;
                            }
                            let Some(cover) = album.cover_path.as_ref() else {
                                continue;
                            };
                            let p = cover.to_string_lossy().into_owned();
                            if db
                                .update_album_cover(&db::Source::Local, &album.id, Some(&p), false)
                                .await
                                .is_ok()
                            {
                                changed = true;
                            }
                        }
                        if changed {
                            gens.bump(hooks::db_reactivity::Table::Albums);
                        }
                    }.instrument(tracing::info_span!("library.fetch_covers")));
                } else {
                    // No cover fetching — drop the callback so the progress bar closes.
                    drop(progress_cb);
                }
            } else {
                // No music directories configured: the local library is empty.
                let _ = db.prune_source(&db::Source::Local, &[], &[]).await;
                gens.bump(hooks::db_reactivity::Table::Tracks);
                gens.bump(hooks::db_reactivity::Table::Albums);
            }
        }.instrument(tracing::info_span!("library.rescan")));
    });

    // Force re-extraction of local covers (embedded art, then folder art) for
    // any album whose cached cover file is missing. No online fetch — this is
    // the "force rescan photos" action and is independent of auto_fetch_covers.
    let db_for_reextract = db.clone();
    let mut cover_reextract_in_flight = use_signal(|| false);
    use_effect(move || {
        if !*initial_load_done.read() || !*config_loaded_ok.read() {
            return;
        }
        if *trigger_cover_reextract.read() == 0 {
            return;
        }
        let db = db_for_reextract.clone();
        let gens = gens_for_albums;
        if *cover_reextract_in_flight.peek() {
            return;
        }
        cover_reextract_in_flight.set(true);
        let cover_cache_dir = cover_cache();
        spawn(
            async move {
                let albums = match db.albums(&db::Source::Local).await {
                    Ok(a) => a,
                    Err(e) => {
                        tracing::error!(error = %e, "cover re-extract: album query failed");
                        return;
                    }
                };
                let mut changed = 0u32;
                for album in &albums {
                    // Manual covers are user-set; a present file needs nothing.
                    if album.manual_cover || album.cover_path.as_ref().is_some_and(|p| p.exists()) {
                        continue;
                    }
                    let tracks = match db.album_tracks(&db::Source::Local, &album.id).await {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    let mut new_cover: Option<std::path::PathBuf> = None;
                    for track in &tracks {
                        let Some(path) = track.id.local_path() else {
                            continue;
                        };
                        // Embedded art first, then a folder image beside the track.
                        if let Some((data, _mime)) = reader::read_cover(path)
                            && let Ok(saved) =
                                reader::utils::save_cover(&album.id, &data, None, &cover_cache_dir)
                        {
                            new_cover = Some(saved);
                            break;
                        }
                        if let Some(folder) =
                            path.parent().and_then(reader::utils::find_folder_cover)
                        {
                            new_cover = Some(folder);
                            break;
                        }
                    }
                    if let Some(cover) = new_cover {
                        let p = cover.to_string_lossy().into_owned();
                        if db
                            .update_album_cover(&db::Source::Local, &album.id, Some(&p), false)
                            .await
                            .is_ok()
                        {
                            changed += 1;
                        }
                    }
                }
                if changed > 0 {
                    gens.bump(hooks::db_reactivity::Table::Albums);
                }
                tracing::info!(updated = changed, "cover re-extract: complete");
            }
            .instrument(tracing::info_span!("library.cover_reextract")),
        );
    });

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
    provide_context(download_progress);
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

    let theme_class = use_memo(move || {
        if config.read().theme == "album-art" {
            "theme-default".to_string()
        } else {
            format!("theme-{}", config.read().theme)
        }
    });

    let is_rtl = i18n::is_rtl();
    let dir = if is_rtl { "rtl" } else { "ltr" };
    let content_row_class = "flex flex-1 overflow-hidden";
    let update_banner_state = update_banner.read().clone();

    let background_style = use_memo(move || {
        if config.read().theme == "album-art" {
            utils::color::get_background_style(palette.read().as_deref())
        } else {
            "background-color: var(--color-black); background-image: none;".to_string()
        }
    });

    let reduce_animations = use_memo(move || config.read().reduce_animations);
    let active_source = use_memo(move || config.read().active_source.clone());
    let switch_source = hooks::source_switch::use_switch_source();

    rsx! {
        // we use this component here to prevent re-diffing to prevent warns in console
        StaticHeadAssets {}
        WindowsToolbarIconAssets {}

        div {
            class: "flex flex-col h-screen text-white select-none overflow-x-hidden {theme_class}",
            style: "{background_style}",
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
                } else if key == Key::Character(" ".into()) {
                    ctrl.toggle();
                    evt.prevent_default();
                }
            },
            if cfg!(any(target_os = "linux", target_os = "windows")) {
                div { dir: "ltr", Titlebar {} }
            }

            if active_source == config::Source::Local {
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
            if config
                .read()
                .server
                .as_ref()
                .map(|s| s.service == config::MusicService::YtMusic)
                .unwrap_or(false)
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
                                        let target = config.peek().server_toggle_target();
                                        if let Some(s) = target {
                                            switch_source(s);
                                        }
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
                        class: "flex items-center justify-between gap-3 px-4 py-2 bg-sky-500/15 border-b border-sky-500/20 text-sky-200 text-sm",
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
                    player: player,
                    is_playing: is_playing,
                    is_fullscreen: is_fullscreen,
                    current_song_duration: current_song_duration,
                    current_song_progress: current_song_progress,
                    queue: queue,
                    current_queue_index: current_queue_index,
                    volume: volume,
                    persisted_volume: persisted_volume,
                    is_rightbar_open: is_rightbar_open,
                }
            }
            div {
                class: "{content_row_class}",
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
                            rsx! {
                                div { class: "shrink-0 z-[60] bg-black/60 backdrop-blur-2xl border-b border-white/5 pt-[env(safe-area-inset-top)] flex items-center h-[calc(env(safe-area-inset-top)_+_2.75rem)] px-3 shadow-xl",
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
                                            style: "font-family: 'JetBrains Mono', monospace;",
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
                                    let source = config.peek().active_source.clone();
                                    let db = db_for_play_album.clone();
                                    spawn(async move {
                                        let mut tracks =
                                            db.album_tracks(&source, &id).await.unwrap_or_default();
                                        if !tracks.is_empty() {
                                            tracks.sort_by(|a, b| {
                                                let disc_cmp = a.disc_number.unwrap_or(1).cmp(&b.disc_number.unwrap_or(1));
                                                if disc_cmp == std::cmp::Ordering::Equal {
                                                    a.track_number.unwrap_or(0).cmp(&b.track_number.unwrap_or(0))
                                                } else {
                                                    disc_cmp
                                                }
                                            });
                                            queue.set(tracks);
                                            ctrl.play_track(0);
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
                                player: player,
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
                                player: player,
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
                                active_caps().artist_view == ::server::source::ArtistView::Remote;
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
                                        player: player,
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
                                player,
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
            }
            Fullscreen {
                player: player,
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
            if config.read().player_bar_position == config::PlayerBarPosition::Bottom {
                Bottombar {
                    config,
                    current_song_cover_url: current_song_cover_url,
                    current_song_title: current_song_title,
                    current_song_artist: current_song_artist,
                    player: player,
                    is_playing: is_playing,
                    is_fullscreen: is_fullscreen,
                    current_song_duration: current_song_duration,
                    current_song_progress: current_song_progress,
                    queue: queue,
                    current_queue_index: current_queue_index,
                    volume: volume,
                    persisted_volume: persisted_volume,
                    is_rightbar_open: is_rightbar_open,
                }
            }
        }
    }
}
