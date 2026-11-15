//! The Kopuz GPUI app: its setup and main window. `main.rs` runs it on the desktop platform; an Android
//! host hands in its own platform and the daemon core it runs in-process (`run_in_process`).

#[cfg(not(target_os = "android"))]
mod args;
mod backdrop;
mod handover;
#[cfg(not(target_os = "android"))]
mod instance;
#[cfg(not(target_os = "android"))]
mod links;
#[cfg(target_os = "macos")]
mod menus;
mod router;
mod shell;
mod system_ease;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod tray;

use std::time::Instant;

use gpui_kit::component::Root;
use gpui_kit::{
    App, AppContext as _, Bounds, Global, KeyBinding, TitlebarOptions, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};
use shell::{Back, Escape, Forward, QuickSearch, Shell, TogglePlayback, TogglePlaylistsPanel};

/// Keep the saved window large enough for the navigation rail and a useful page area.
const MAIN_WINDOW_MIN_SIZE: gpui_kit::Size<gpui_kit::Pixels> = size(px(720.), px(480.));

/// The bindings whose handlers live here or on the list; `commands::keymap` binds the registry's.
fn keymap(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("space", TogglePlayback, Some("Shell")),
        KeyBinding::new("cmd-k", QuickSearch, Some("Shell")),
        KeyBinding::new("ctrl-k", QuickSearch, Some("Shell")),
        KeyBinding::new("escape", Escape, Some("Shell")),
        KeyBinding::new("alt-left", Back, Some("Shell")),
        KeyBinding::new("alt-right", Forward, Some("Shell")),
        KeyBinding::new("ctrl-b", TogglePlaylistsPanel, Some("Shell")),
        KeyBinding::new(
            "ctrl-shift-p",
            debug_overlay::ToggleDebugOverlay,
            Some("Shell"),
        ),
        // Bindings beat key listeners, so without this the Shell's space would
        // toggle playback instead of typing one; NoAction suppresses it.
        KeyBinding::new("space", gpui_kit::NoAction, Some("Input")),
        // A focused track list walks with the arrows; Escape falls through to the
        // Shell when there is no selection to clear.
        KeyBinding::new(
            "down",
            components::track_list::CursorDown,
            Some("TrackList"),
        ),
        KeyBinding::new("up", components::track_list::CursorUp, Some("TrackList")),
        KeyBinding::new(
            "cmd-a",
            components::track_list::SelectAll,
            Some("TrackList"),
        ),
        KeyBinding::new(
            "ctrl-a",
            components::track_list::SelectAll,
            Some("TrackList"),
        ),
        KeyBinding::new(
            "escape",
            components::track_list::ClearSelection,
            Some("TrackList"),
        ),
    ]);
}

/// Where the daemon is, and whether this app starts its own there: only on the default socket, never on `KOPUZ_DAEMON_ADDR` or `KOPUZ_SOCKET`.
#[cfg(not(target_os = "android"))]
fn daemon_address() -> anyhow::Result<(link::Address, bool)> {
    if let Ok(host) = std::env::var("KOPUZ_DAEMON_ADDR") {
        return Ok((link::Address::Tcp(host), false));
    }
    if let Some(socket) = std::env::var_os("KOPUZ_SOCKET") {
        return Ok((link::Address::Socket(socket.into()), false));
    }
    let socket = link::default_socket_path()
        .ok_or_else(|| anyhow::anyhow!("no runtime directory to find the daemon socket in"))?;
    Ok((link::Address::Socket(socket), true))
}

/// The desktop app: what `main` runs.
#[cfg(not(target_os = "android"))]
pub fn run() -> anyhow::Result<()> {
    // Before logging starts: this process becomes kopuzd, which keeps its own log.
    let argv: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if let Some(code) = link::run_daemon_flag(&argv) {
        std::process::exit(code);
    }
    // Held until main returns: dropping it is what flushes the chrome trace.
    let _profile = profile::init();
    let started = std::time::Instant::now();

    let (address, start_own) = daemon_address()?;
    let args = args::Args::parse(std::env::args_os().skip(1));
    let _instance = match (instance::claim(&address), &args.link) {
        (Err(instance::Elsewhere::Running), Some(link)) => {
            // The daemon plays a listen link; any other is the open window's to show.
            match store::deep_link::is_listen(link) {
                true => instance::hand_off(&address, link.clone()),
                false => links::hand_off(&address, link),
            }
            return Ok(());
        }
        (claimed, _) => claimed.ok(),
    };

    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    // macOS hands a link to the running app here rather than on a command line.
    let (opened_tx, opened) = futures::channel::mpsc::unbounded::<String>();
    let relay = link::relay::path(&address);
    let relayed = opened_tx.clone();
    let (booted, boot) = futures::channel::oneshot::channel();
    application.on_open_urls(move |urls| {
        for url in urls.into_iter().filter(|url| args::is_link(url)) {
            let _ = opened_tx.unbounded_send(url);
        }
    });
    // A window closed to the tray, or with the red button, comes back from the Dock icon.
    application.on_reopen(|cx| {
        if main_window(cx).is_none() && cx.has_global::<Booted>() {
            open_main_window(Instant::now(), None, cx);
        }
    });
    application.run(move |cx| {
        if !connect(|cx| link::init(address, start_own, cx), cx) {
            return;
        }
        store::init(cx);
        if args.offline {
            store::SourcesModel::global(cx)
                .update(cx, |sources, cx| sources.offline_on_connect(cx));
        }
        if let Some(path) = relay {
            link::relay::listen(path, relayed, cx);
        }
        links::follow(args.link, opened, boot, cx);
        register(cx);
        let ready = store::LayoutModel::global(cx).update(cx, |layout, _| layout.take_ready());
        cx.spawn(async move |cx| {
            let timer = cx.background_executor().timer(PREFS_WAIT);
            if let Some(ready) = ready {
                futures::future::select(Box::pin(ready), Box::pin(timer)).await;
            }
            cx.update(|cx| {
                // After the preferences load, since the registry reads the saved keystroke overrides.
                commands::keymap(cx);
                cx.set_global(Booted);
                let _ = booted.send(());
                open_main_window(started, None, cx);
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                tray::init(cx);
            });
        })
        .detach();
    });
    Ok(())
}

/// The app on a platform the host made (gpui-pre-mobile's on Android), talking to the daemon core this process
/// runs. `opened` gets the main window once it is open, from the `run` callback rather than from a task.
#[cfg(target_os = "android")]
pub fn run_in_process(
    application: gpui_kit::Application,
    api: std::sync::Arc<dyn api::KopuzApi>,
    opened: impl FnOnce(gpui_kit::AnyWindowHandle, &mut App) + 'static,
) {
    let started = Instant::now();
    application
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            if !connect(|cx| link::init_in_process(api, cx), cx) {
                return;
            }
            store::init(cx);
            register(cx);
            commands::keymap(cx);
            cx.set_global(Booted);
            // Not after the preferences wait as on the desktop: gpui-pre-mobile 0.1.0 runs tasks under its
            // platform lock, which opening a window takes again, so a window opened from a task deadlocks.
            if let Some(handle) = open_main_window(started, None, cx) {
                opened(handle.into(), cx);
            }
        });
}

/// Set up the kit and the daemon link; when the link cannot be set up, say why in a window and return false.
fn connect(init: impl FnOnce(&mut App) -> anyhow::Result<()>, cx: &mut App) -> bool {
    gpui_kit::init(cx);
    i18n::init("en");
    // Quitting here would look like the icon did nothing; say why instead.
    let Err(error) = init(cx) else {
        return true;
    };
    tracing::error!(%error, "could not set up the daemon link");
    theme::apply_named("default", None, cx);
    let reason = error.to_string();
    cx.open_window(
        WindowOptions {
            app_id: Some(config::flavor::Flavor::current().name().into()),
            titlebar: Some(TitlebarOptions {
                title: Some(config::flavor::Flavor::current().display_name().into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            let screen: gpui_kit::AnyView = cx.new(|_| status::StartupError::new(reason)).into();
            cx.new(|cx| Root::new(screen, window, cx))
        },
    )
    .expect("failed to open the error window");
    false
}

/// Register the pages, settings and commands, and apply the theme and the keymap.
fn register(cx: &mut App) {
    metadata_dialog::register(cx);
    settings_sources::register(cx);
    settings_interface::init(cx);
    shortcuts::init(cx);
    library::register(cx);
    page_home::register(cx);
    page_home::register_catalog(
        |id, scroll, window, cx| {
            let feed = cx.new(|cx| discover::CatalogPage::embedded(id, scroll, window, cx));
            let chips = cx.new(|cx| discover::CatalogChips::new(feed.clone(), cx));
            let page = feed.clone();
            page_home::CatalogViews {
                chips: chips.into(),
                feed: feed.into(),
                strapline_to_host: std::rc::Rc::new(move |on, cx| {
                    page.update(cx, |page, cx| page.hand_strapline_to_host(on, cx));
                }),
            }
        },
        cx,
    );
    topbar::register_settings(cx);
    playlists_panel::register_settings(cx);
    palette::register_settings(cx);
    now_playing::register_settings(cx);
    // The mini player is a second window, and Android has only the one.
    #[cfg(not(target_os = "android"))]
    store::register_mini_player(now_playing::toggle_mini, cx);
    status::register_settings(cx);
    status::register_url_download(cx);
    page_playlists::register(cx);
    commands::init(cx);
    theme::apply_named("default", None, cx);
    keymap(cx);
    #[cfg(target_os = "macos")]
    menus::init(cx);
}

/// The app is set up and the main window may be opened again after it was closed.
struct Booted;

impl Global for Booted {}

/// The main window, so the tray and the Dock can tell whether it is open.
#[cfg_attr(target_os = "android", allow(dead_code))]
struct MainWindow(WindowHandle<Root>);

impl Global for MainWindow {}

/// The main window while it is open.
#[cfg_attr(target_os = "android", allow(dead_code))]
pub(crate) fn main_window(cx: &App) -> Option<WindowHandle<Root>> {
    let handle = cx.try_global::<MainWindow>()?.0;
    cx.windows()
        .iter()
        .any(|window| window.window_id() == handle.window_id())
        .then_some(handle)
}

/// How long the window waits for the daemon to say where it was left before opening anyway.
#[cfg(not(target_os = "android"))]
const PREFS_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

/// Open the main window at `bounds`, or where it was left; `started` times the loader.
pub(crate) fn open_main_window(
    started: Instant,
    bounds: Option<WindowBounds>,
    cx: &mut App,
) -> Option<WindowHandle<Root>> {
    let mode = topbar::frame::mode(cx);
    let mut options = WindowOptions {
        window_bounds: Some(
            bounds
                .or_else(|| topbar::saved_bounds(cx))
                .unwrap_or_else(|| {
                    WindowBounds::Windowed(Bounds::centered(None, size(px(1350.), px(800.)), cx))
                }),
        ),
        window_min_size: Some(MAIN_WINDOW_MIN_SIZE),
        window_background: gpui_kit::WindowBackgroundAppearance::Opaque,
        app_id: Some(config::flavor::Flavor::current().name().into()),
        ..Default::default()
    };
    topbar::frame::window_options(mode, components::style::chrome().bar_height, &mut options);
    tracing::info!(?mode, "opening the main window");
    match cx.open_window(options, |window, cx| {
        let shell = cx.new(|cx| Shell::new(started, mode, window, cx));
        cx.new(|cx| Root::new(shell, window, cx))
    }) {
        Ok(handle) => {
            cx.set_global(MainWindow(handle));
            Some(handle)
        }
        Err(error) => {
            tracing::error!(%error, "could not open the main window");
            None
        }
    }
}
