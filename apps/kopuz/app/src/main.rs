mod args;
mod backdrop;
mod handover;
mod router;
mod shell;
mod system_ease;

use std::time::Duration;

use gpui_kit::component::Root;
use gpui_kit::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, px,
    size,
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

/// Where the daemon is: `KOPUZ_DAEMON_ADDR` for a TCP daemon, `KOPUZ_SOCKET` for a socket elsewhere.
fn daemon_address() -> anyhow::Result<link::Address> {
    if let Ok(host) = std::env::var("KOPUZ_DAEMON_ADDR") {
        return Ok(link::Address::Tcp(host));
    }
    let socket = std::env::var_os("KOPUZ_SOCKET")
        .map(Into::into)
        .or_else(link::default_socket_path)
        .ok_or_else(|| anyhow::anyhow!("no runtime directory to find the daemon socket in"))?;
    Ok(link::Address::Socket(socket))
}

/// jemalloc, so the profiler can ask the allocator what is held instead of reading RSS.
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Hand freed pages back to the OS on a timer, so releasing a cover shows up in RSS rather than
/// leaving the process looking as large as it ever was.
#[unsafe(export_name = "malloc_conf")]
pub static MALLOC_CONF: &[u8] = b"background_thread:true,dirty_decay_ms:5000,muzzy_decay_ms:5000\0";

fn main() -> anyhow::Result<()> {
    // Held until main returns: dropping it is what flushes the chrome trace.
    let _profile = profile::init();
    let started = std::time::Instant::now();

    let address = daemon_address()?;
    let args = args::Args::parse(std::env::args_os().skip(1));

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            i18n::init("en");
            // Quitting here would look like the icon did nothing; say why instead.
            if let Err(error) = link::init(address, cx) {
                tracing::error!(%error, "could not set up the daemon link");
                theme::apply_named("default", None, cx);
                let reason = error.to_string();
                cx.open_window(
                    WindowOptions {
                        app_id: Some("kopuz".into()),
                        titlebar: Some(TitlebarOptions {
                            title: Some("Kopuz".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |window, cx| {
                        let screen: gpui_kit::AnyView =
                            cx.new(|_| status::StartupError::new(reason)).into();
                        cx.new(|cx| Root::new(screen, window, cx))
                    },
                )
                .expect("failed to open the error window");
                return;
            }
            store::init(cx);
            if args.offline {
                store::SourcesModel::global(cx)
                    .update(cx, |sources, cx| sources.offline_on_connect(cx));
            }
            metadata_dialog::register(cx);
            settings_sources::register(cx);
            settings_interface::init(cx);
            shortcuts::init(cx);
            library::register(cx);
            page_home::register(cx);
            page_home::register_catalog(
                |id, scroll, window, cx| {
                    cx.new(|cx| discover::CatalogPage::embedded(id, scroll, window, cx))
                        .into()
                },
                cx,
            );
            topbar::register_settings(cx);
            playlists_panel::register_settings(cx);
            palette::register_settings(cx);
            now_playing::register_settings(cx);
            store::register_mini_player(now_playing::toggle_mini, cx);
            status::register_settings(cx);
            status::register_url_download(cx);
            page_playlists::register(cx);
            commands::init(cx);
            theme::apply_named("default", None, cx);
            keymap(cx);
            let ready = store::LayoutModel::global(cx).update(cx, |layout, _| layout.take_ready());
            cx.spawn(async move |cx| {
                let timer = cx.background_executor().timer(PREFS_WAIT);
                if let Some(ready) = ready {
                    futures::future::select(Box::pin(ready), Box::pin(timer)).await;
                }
                cx.update(|cx| open_main_window(started, cx));
            })
            .detach();
        });
    Ok(())
}

/// How long the window waits for the daemon to say where it was left before opening anyway.
const PREFS_WAIT: Duration = Duration::from_millis(1500);

fn open_main_window(started: std::time::Instant, cx: &mut App) {
    // After the preferences load, since the registry reads the saved keystroke overrides.
    commands::keymap(cx);
    let options = WindowOptions {
        window_bounds: Some(topbar::saved_bounds(cx).unwrap_or_else(|| {
            WindowBounds::Windowed(Bounds::centered(None, size(px(1350.), px(800.)), cx))
        })),
        window_min_size: Some(MAIN_WINDOW_MIN_SIZE),
        window_background: gpui_kit::WindowBackgroundAppearance::Opaque,
        app_id: Some("kopuz".into()),
        titlebar: Some(TitlebarOptions {
            title: Some("Kopuz".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        let shell = cx.new(|cx| Shell::new(started, window, cx));
        cx.new(|cx| Root::new(shell, window, cx))
    })
    .expect("failed to open the main window");
}
