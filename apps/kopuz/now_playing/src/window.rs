//! The mini player's own window: a layer-shell overlay where the compositor offers one, else a small window.

use components::style;
use gpui_kit::component::{ActiveTheme as _, Root};
#[cfg(target_os = "linux")]
use gpui_kit::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Global,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Pixels,
    Render, Size, Styled, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea, WindowDecorations, WindowKind, WindowOptions, div, point, px, size,
};
use store::Chrome;

use crate::NowPlaying;

// The size the mini player is drawn for; it is geometry, not a preference.
const WIDTH: Pixels = px(380.);
const HEIGHT: Pixels = px(148.);
const DRAG_STRIP: Pixels = px(10.);
// How far the overlay sits from the corner it is anchored to.
#[cfg(target_os = "linux")]
const MARGIN: Pixels = px(24.);

/// The handle to the second window, while there is one.
#[derive(Default)]
struct MiniHandle(Option<AnyWindowHandle>);

#[derive(Default)]
struct Minimized(Vec<AnyWindowHandle>);

impl Global for Minimized {}

impl Global for MiniHandle {}

/// The mini window's root: the shared player, drawn at its smallest.
pub struct MiniWindow {
    player: Entity<NowPlaying>,
    frameless: bool,
    focus: FocusHandle,
}

impl MiniWindow {
    fn new(frameless: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            player: cx.new(|cx| NowPlaying::mini(window, cx)),
            frameless,
            focus,
        }
    }
}

impl Render for MiniWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The app-wide shortcuts are bound in the main window's context; wearing it here lets them
        // reach their handlers from this window too.
        div()
            .id("mini-window")
            .track_focus(&self.focus)
            .key_context("Shell")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "space" && this.focus.is_focused(window) {
                    store::PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx));
                    cx.stop_propagation();
                }
            }))
            .relative()
            .size_full()
            .overflow_hidden()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(style::white(style::BORDER, cx))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.player.clone())
            .children(self.frameless.then(drag_strip))
            .children(Root::render_notification_layer(window, cx))
    }
}

fn drag_strip() -> impl IntoElement {
    div()
        .id("mini-drag")
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h(DRAG_STRIP)
        .occlude()
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
}

fn dimensions() -> Size<Pixels> {
    size(WIDTH, HEIGHT)
}

fn options(kind: WindowKind, bounds: Bounds<Pixels>, frameless: bool) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Client),
        window_min_size: Some(dimensions()),
        app_id: Some(config::flavor::Flavor::current().name().into()),
        titlebar: Some(TitlebarOptions {
            title: Some(config::flavor::Flavor::current().display_name().into()),
            appears_transparent: frameless,
            ..Default::default()
        }),
        is_resizable: false,
        kind,
        ..Default::default()
    }
}

/// Anchored to the top-right of the display, on the layer every compositor paints above windows.
fn overlay() -> WindowKind {
    #[cfg(target_os = "linux")]
    {
        WindowKind::LayerShell(LayerShellOptions {
            namespace: "kopuz-mini".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::RIGHT,
            margin: Some((MARGIN, MARGIN, MARGIN, MARGIN)),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            ..Default::default()
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        WindowKind::Floating
    }
}

fn open_window(kind: WindowKind, frameless: bool, cx: &mut App) -> Option<AnyWindowHandle> {
    let bounds = Bounds {
        origin: point(px(0.), px(0.)),
        size: dimensions(),
    };
    cx.open_window(options(kind, bounds, frameless), |window, cx| {
        window.on_window_should_close(cx, |_, cx| {
            cx.defer(restore_main);
            true
        });
        let mini = cx.new(|cx| MiniWindow::new(frameless, window, cx));
        cx.new(|cx| Root::new(mini, window, cx))
    })
    .map(|handle| handle.into())
    .ok()
}

/// Whether a mini player is up, in either of its forms.
pub fn is_open(cx: &App) -> bool {
    mini_window(cx).is_some() || Chrome::global(cx).read(cx).is_mini()
}

fn mini_window(cx: &App) -> Option<AnyWindowHandle> {
    let handle = cx.try_global::<MiniHandle>()?.0?;
    cx.windows()
        .iter()
        .any(|open| open.window_id() == handle.window_id())
        .then_some(handle)
}

/// Shrink to the mini player: a window of its own where the compositor allows it, else in place.
pub fn open_mini(cx: &mut App) {
    if is_open(cx) {
        return;
    }
    // An always-on-top surface is a layer-shell one; a plain second window cannot ask to stay above.
    let prefs = store::layout(cx).player.clone();
    let frameless = prefs.mini_frameless;
    let handle = open_window(overlay(), frameless, cx).or_else(|| {
        tracing::info!("no layer shell for the mini player; opening a plain window instead");
        open_window(WindowKind::Normal, frameless, cx)
    });
    match handle {
        Some(handle) => {
            cx.set_global(MiniHandle(Some(handle)));
            if prefs.mini_hides_main {
                cx.defer(move |cx| minimize_main(handle, cx));
            }
        }
        None => {
            tracing::warn!("no second window for the mini player; shrinking the main one");
            Chrome::global(cx).update(cx, |chrome, cx| chrome.set_mini(true, cx));
        }
    }
}

/// Put the full player back, whichever way the mini one was showing.
pub fn close_mini(cx: &mut App) {
    let open = mini_window(cx);
    cx.set_global(MiniHandle(None));
    if let Some(handle) = open {
        // The restore button runs inside the mini window's own update, where GPUI cannot reach that window.
        cx.defer(move |cx| {
            if let Err(error) = handle.update(cx, |_, window, _| window.remove_window()) {
                tracing::warn!(%error, "could not close the mini player window");
            }
        });
    }
    cx.defer(restore_main);
    Chrome::global(cx).update(cx, |chrome, cx| chrome.set_mini(false, cx));
}

fn minimize_main(mini: AnyWindowHandle, cx: &mut App) {
    let others: Vec<AnyWindowHandle> = cx
        .windows()
        .into_iter()
        .filter(|window| window.window_id() != mini.window_id())
        .collect();
    for window in &others {
        if let Err(error) = window.update(cx, |_, window, _| window.minimize_window()) {
            tracing::warn!(%error, "could not minimise the main window");
        }
    }
    cx.set_global(Minimized(others));
}

fn restore_main(cx: &mut App) {
    let windows = cx
        .try_global::<Minimized>()
        .map(|held| held.0.clone())
        .unwrap_or_default();
    cx.set_global(Minimized::default());
    for window in windows {
        if let Err(error) = window.update(cx, |_, window, _| window.activate_window()) {
            tracing::warn!(%error, "could not bring back the main window");
        }
    }
}

pub fn toggle_mini(cx: &mut App) {
    if is_open(cx) {
        close_mini(cx);
    } else {
        open_mini(cx);
    }
}
