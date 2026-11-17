//! The tray icon `system.minimize_to_tray` asks for: playback controls, Show / Hide and Quit, and a closed window leaves Kopuz running.

use std::time::Instant;

use api::LoopMode;
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::{App, Global, QuitMode, Task};
use store::{ConfigModel, PlayerModel};

use components::format::t;

/// How far one notch of the wheel over the icon moves the volume.
#[cfg(target_os = "linux")]
const VOLUME_STEP: f32 = 0.05;

/// What the tray asks of the app, from whatever thread the platform delivers it on.
#[derive(Clone, Copy, Debug)]
enum Click {
    ShowHide,
    Quit,
    PlayPause,
    Next,
    Previous,
    Shuffle,
    Repeat,
    /// Only the Linux tray reports the wheel.
    #[cfg(target_os = "linux")]
    Volume(f32),
}

/// Everything the menu and tooltip show, built on the app's thread so the tray's own never touches i18n or models.
#[derive(Clone, Debug, PartialEq)]
struct Menu {
    /// "Title — Artist" while something is loaded.
    track: Option<String>,
    play_pause: String,
    next: String,
    previous: String,
    shuffle_label: String,
    shuffle: bool,
    repeat: String,
    show_hide: String,
    quit: String,
    tooltip: String,
}

impl Menu {
    fn current(cx: &App) -> Self {
        let player = PlayerModel::global(cx).read(cx);
        let track = player.track().map(|track| match track.artist.is_empty() {
            true => track.title.clone(),
            false => format!("{} — {}", track.title, track.artist),
        });
        let app = config::flavor::Flavor::current().display_name().to_string();
        Self {
            tooltip: track.clone().unwrap_or_else(|| app.clone()),
            track,
            play_pause: t(match player.is_playing() {
                true => "pause",
                false => "play",
            })
            .to_string(),
            next: t("next_track").to_string(),
            previous: t("previous_track").to_string(),
            shuffle_label: t("shuffle").to_string(),
            shuffle: player.shuffle(),
            repeat: t(match player.loop_mode() {
                LoopMode::None => "repeat_off",
                LoopMode::Queue => "repeat_queue",
                LoopMode::Track => "repeat_track",
            })
            .to_string(),
            show_hide: t("tray_show_hide").to_string(),
            quit: t("tray_quit").to_string(),
        }
    }

    fn has_track(&self) -> bool {
        self.track.is_some()
    }
}

/// The icon while it is up, and the menu it last showed.
struct Tray {
    clicks: mpsc::UnboundedSender<Click>,
    icon: Option<(platform::Icon, Menu)>,
}

impl Global for Tray {}

/// The task turning clicks into app actions.
struct Clicks {
    _listen: Task<()>,
}

impl Global for Clicks {}

/// Follow the setting and the player from now on; called once the main window is up.
pub fn init(cx: &mut App) {
    let (sender, mut clicks) = mpsc::unbounded::<Click>();
    platform::listen(sender.clone());
    let listen = cx.spawn(async move |cx| {
        while let Some(click) = clicks.next().await {
            cx.update(|cx| clicked(click, cx));
        }
    });
    cx.set_global(Clicks { _listen: listen });
    cx.set_global(Tray {
        clicks: sender,
        icon: None,
    });
    sync(cx);
    cx.observe(&ConfigModel::global(cx), |_, cx| sync(cx))
        .detach();
    cx.observe(&PlayerModel::global(cx), |_, cx| sync(cx))
        .detach();
}

fn clicked(click: Click, cx: &mut App) {
    let player = PlayerModel::global(cx);
    match click {
        Click::ShowHide => match crate::main_window(cx) {
            Some(window) => {
                if let Err(error) = window.update(cx, |_, window, _| window.remove_window()) {
                    tracing::warn!(%error, "could not hide the window to the tray");
                }
            }
            None => {
                crate::open_main_window(Instant::now(), None, cx);
                cx.activate(true);
            }
        },
        Click::Quit => cx.quit(),
        Click::PlayPause => player.update(cx, |player, cx| player.toggle(cx)),
        Click::Next => player.update(cx, |player, cx| player.next(cx)),
        Click::Previous => player.update(cx, |player, cx| player.previous(cx)),
        Click::Shuffle => player.update(cx, |player, cx| {
            let shuffle = !player.shuffle();
            player.set_shuffle(shuffle, cx);
        }),
        Click::Repeat => player.update(cx, |player, cx| player.cycle_loop_mode(cx)),
        #[cfg(target_os = "linux")]
        Click::Volume(step) => player.update(cx, |player, cx| player.nudge_volume(step, cx)),
    }
}

/// Put the icon up or take it down as the setting says, and redraw its menu when what it shows changes.
fn sync(cx: &mut App) {
    let wanted = ConfigModel::global(cx)
        .read(cx)
        .config()
        .system
        .minimize_to_tray;
    let mut tray = cx.remove_global::<Tray>();
    let mut taken_down = false;
    match (wanted, tray.icon.as_mut()) {
        (false, Some(_)) => {
            tray.icon = None;
            taken_down = true;
        }
        (true, None) => {
            let menu = Menu::current(cx);
            tray.icon =
                platform::Icon::build(menu.clone(), tray.clicks.clone()).map(|icon| (icon, menu));
        }
        (true, Some((icon, shown))) => {
            // The player notifies with every position tick; most of those change nothing here.
            let menu = Menu::current(cx);
            if *shown != menu {
                icon.show(menu.clone());
                *shown = menu;
            }
        }
        (false, None) => {}
    }
    // Closing the window only hides Kopuz while the tray is there to bring it back.
    cx.set_quit_mode(match tray.icon.is_some() {
        true => QuitMode::Explicit,
        false => QuitMode::Default,
    });
    cx.set_global(tray);
    // With the icon gone, nothing else could bring a hidden window back.
    if taken_down && crate::main_window(cx).is_none() {
        crate::open_main_window(Instant::now(), None, cx);
    }
}

/// The logo as RGBA, or `None` with a warning.
fn logo() -> Option<image::RgbaImage> {
    match image::load_from_memory(include_bytes!("../../../legacy/kopuz/assets/logo-128.png")) {
        Ok(image) => Some(image.into_rgba8()),
        Err(error) => {
            tracing::warn!(%error, "could not read the tray icon");
            None
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use futures::channel::mpsc;
    use tray_icon::menu::{
        CheckMenuItem, ContextMenu, IsMenuItem, Menu as NativeMenu, MenuEvent, MenuItem,
        PredefinedMenuItem,
    };
    use tray_icon::{TrayIcon, TrayIconBuilder};

    use super::{Click, Menu};

    const SHOW_HIDE: &str = "kopuz-tray-show";
    const QUIT: &str = "kopuz-tray-quit";
    const TRACK: &str = "kopuz-tray-track";
    const PLAY_PAUSE: &str = "kopuz-tray-play-pause";
    const NEXT: &str = "kopuz-tray-next";
    const PREVIOUS: &str = "kopuz-tray-previous";
    const SHUFFLE: &str = "kopuz-tray-shuffle";
    const REPEAT: &str = "kopuz-tray-repeat";

    /// tray-icon reports every menu's clicks through one process-wide handler.
    pub(super) fn listen(clicks: mpsc::UnboundedSender<Click>) {
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let click = match event.id.as_ref() {
                SHOW_HIDE => Click::ShowHide,
                QUIT => Click::Quit,
                PLAY_PAUSE => Click::PlayPause,
                NEXT => Click::Next,
                PREVIOUS => Click::Previous,
                SHUFFLE => Click::Shuffle,
                REPEAT => Click::Repeat,
                _ => return,
            };
            clicks.unbounded_send(click).ok();
        }));
    }

    pub(super) struct Icon(TrayIcon);

    impl Icon {
        pub(super) fn build(menu: Menu, _clicks: mpsc::UnboundedSender<Click>) -> Option<Self> {
            let mut builder = TrayIconBuilder::new()
                .with_tooltip(&menu.tooltip)
                .with_menu(Box::new(native(&menu)?));
            if let Some(icon) = image() {
                builder = builder.with_icon(icon);
            }
            match builder.build() {
                Ok(tray) => Some(Self(tray)),
                Err(error) => {
                    tracing::warn!(%error, "could not put the tray icon up");
                    None
                }
            }
        }

        pub(super) fn show(&mut self, menu: Menu) {
            if let Err(error) = self.0.set_tooltip(Some(&menu.tooltip)) {
                tracing::warn!(%error, "could not update the tray tooltip");
            }
            self.0
                .set_menu(native(&menu).map(|menu| Box::new(menu) as Box<dyn ContextMenu>));
        }
    }

    fn native(menu: &Menu) -> Option<NativeMenu> {
        let playable = menu.has_track();
        let track = menu
            .track
            .as_ref()
            .map(|track| MenuItem::with_id(TRACK, track, false, None));
        let play_pause = MenuItem::with_id(PLAY_PAUSE, &menu.play_pause, playable, None);
        let next = MenuItem::with_id(NEXT, &menu.next, playable, None);
        let previous = MenuItem::with_id(PREVIOUS, &menu.previous, playable, None);
        let shuffle =
            CheckMenuItem::with_id(SHUFFLE, &menu.shuffle_label, true, menu.shuffle, None);
        let repeat = MenuItem::with_id(REPEAT, &menu.repeat, true, None);
        let show = MenuItem::with_id(SHOW_HIDE, &menu.show_hide, true, None);
        let quit = MenuItem::with_id(QUIT, &menu.quit, true, None);
        let (top, middle, bottom) = (
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::separator(),
        );
        let mut items: Vec<&dyn IsMenuItem> = Vec::new();
        if let Some(track) = &track {
            items.extend([track as &dyn IsMenuItem, &top]);
        }
        items.extend([
            &play_pause as &dyn IsMenuItem,
            &next,
            &previous,
            &middle,
            &shuffle,
            &repeat,
            &bottom,
            &show,
            &quit,
        ]);
        match NativeMenu::with_items(&items) {
            Ok(menu) => Some(menu),
            Err(error) => {
                tracing::warn!(%error, "could not build the tray menu");
                None
            }
        }
    }

    fn image() -> Option<tray_icon::Icon> {
        let image = super::logo()?;
        let (width, height) = image.dimensions();
        tray_icon::Icon::from_rgba(image.into_raw(), width, height)
            .inspect_err(|error| tracing::warn!(%error, "could not make the tray icon"))
            .ok()
    }
}

/// A StatusNotifierItem over D-Bus: appindicator would need a GTK main loop, and GPUI runs none.
#[cfg(target_os = "linux")]
mod platform {
    use futures::channel::mpsc;
    use ksni::blocking::{Handle, TrayMethods as _};
    use ksni::menu::{CheckmarkItem, StandardItem};

    use super::{Click, Menu, VOLUME_STEP};

    /// ksni hands each click to the item itself, so there is nothing process-wide to listen on.
    pub(super) fn listen(_clicks: mpsc::UnboundedSender<Click>) {}

    struct Item {
        menu: Menu,
        clicks: mpsc::UnboundedSender<Click>,
        pixmap: Vec<ksni::Icon>,
    }

    impl Item {
        fn send(&self, click: Click) {
            self.clicks.unbounded_send(click).ok();
        }
    }

    fn entry(label: &str, enabled: bool, click: Click) -> ksni::MenuItem<Item> {
        StandardItem {
            label: label.to_owned(),
            enabled,
            activate: Box::new(move |item: &mut Item| item.send(click)),
            ..Default::default()
        }
        .into()
    }

    impl ksni::Tray for Item {
        fn id(&self) -> String {
            config::flavor::Flavor::current().name().to_string()
        }

        fn title(&self) -> String {
            config::flavor::Flavor::current().display_name().to_string()
        }

        fn category(&self) -> ksni::Category {
            ksni::Category::ApplicationStatus
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.pixmap.clone()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: self.title(),
                description: self.menu.track.clone().unwrap_or_default(),
                ..Default::default()
            }
        }

        /// A left click shows or hides; the right click opens the menu.
        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(Click::ShowHide);
        }

        /// A middle click plays or pauses.
        fn secondary_activate(&mut self, _x: i32, _y: i32) {
            self.send(Click::PlayPause);
        }

        /// The wheel over the icon turns the volume.
        fn scroll(&mut self, delta: i32, orientation: ksni::Orientation) {
            if orientation != ksni::Orientation::Vertical || delta == 0 {
                return;
            }
            // Hosts report wheel-up as a negative delta.
            let step = if delta < 0 { VOLUME_STEP } else { -VOLUME_STEP };
            self.send(Click::Volume(step));
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            let menu = &self.menu;
            let playable = menu.has_track();
            let mut items = Vec::new();
            if let Some(track) = &menu.track {
                items.push(
                    StandardItem {
                        label: track.replace('_', "__"),
                        enabled: false,
                        ..Default::default()
                    }
                    .into(),
                );
                items.push(ksni::MenuItem::Separator);
            }
            items.extend([
                entry(&menu.play_pause, playable, Click::PlayPause),
                entry(&menu.next, playable, Click::Next),
                entry(&menu.previous, playable, Click::Previous),
                ksni::MenuItem::Separator,
                CheckmarkItem {
                    label: menu.shuffle_label.clone(),
                    checked: menu.shuffle,
                    activate: Box::new(|item: &mut Self| item.send(Click::Shuffle)),
                    ..Default::default()
                }
                .into(),
                entry(&menu.repeat, true, Click::Repeat),
                ksni::MenuItem::Separator,
                entry(&menu.show_hide, true, Click::ShowHide),
                entry(&menu.quit, true, Click::Quit),
            ]);
            items
        }
    }

    /// Taking the item off the bus when it is dropped is what removes the icon.
    pub(super) struct Icon(Handle<Item>);

    impl Drop for Icon {
        fn drop(&mut self) {
            // Not waited on: the service thread finishes unregistering on its own.
            drop(self.0.shutdown());
        }
    }

    impl Icon {
        pub(super) fn build(menu: Menu, clicks: mpsc::UnboundedSender<Click>) -> Option<Self> {
            let item = Item {
                menu,
                clicks,
                pixmap: pixmap(),
            };
            // A Flatpak may not own the well-known name the spec asks for.
            match item.disable_dbus_name(sandboxed()).spawn() {
                Ok(handle) => Some(Self(handle)),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "could not put the tray icon up; the desktop needs a StatusNotifierItem host"
                    );
                    None
                }
            }
        }

        pub(super) fn show(&mut self, menu: Menu) {
            self.0.update(|item| item.menu = menu);
        }
    }

    fn sandboxed() -> bool {
        std::path::Path::new("/.flatpak-info").exists()
    }

    /// SNI wants ARGB32 in network byte order.
    fn pixmap() -> Vec<ksni::Icon> {
        let Some(image) = super::logo() else {
            return Vec::new();
        };
        let (width, height) = image.dimensions();
        let mut data = image.into_raw();
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width: width as i32,
            height: height as i32,
            data,
        }]
    }
}
