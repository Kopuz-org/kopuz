//! The mini player's own window: a layer-shell overlay where the compositor offers one, else a small window.

use components::style;
use gpui_kit::component::{ActiveTheme as _, Root};
#[cfg(target_os = "linux")]
use gpui_kit::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, Global, IntoElement,
    ParentElement as _, Pixels, Render, Size, Styled, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions, div,
    point, px, size,
};
use store::Chrome;

use crate::NowPlaying;

// The size the mini player is drawn for; it is geometry, not a preference.
const WIDTH: Pixels = px(380.);
const HEIGHT: Pixels = px(148.);
// How far the overlay sits from the corner it is anchored to.
#[cfg(target_os = "linux")]
const MARGIN: Pixels = px(24.);

/// The handle to the second window, while there is one.
#[derive(Default)]
struct MiniHandle(Option<AnyWindowHandle>);

impl Global for MiniHandle {}

/// The mini window's root: the shared player, drawn at its smallest.
pub struct MiniWindow {
    player: Entity<NowPlaying>,
}

impl MiniWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            player: cx.new(|cx| NowPlaying::mini(window, cx)),
        }
    }
}

impl Render for MiniWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .overflow_hidden()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(style::white(style::BORDER, cx))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.player.clone())
            .children(Root::render_notification_layer(window, cx))
    }
}

fn dimensions() -> Size<Pixels> {
    size(WIDTH, HEIGHT)
}

fn options(kind: WindowKind, bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Client),
        window_min_size: Some(dimensions()),
        app_id: Some(config::flavor::Flavor::current().name().into()),
        titlebar: Some(TitlebarOptions {
            title: Some(config::flavor::Flavor::current().display_name().into()),
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

fn open_window(kind: WindowKind, cx: &mut App) -> Option<AnyWindowHandle> {
    let bounds = Bounds {
        origin: point(px(0.), px(0.)),
        size: dimensions(),
    };
    cx.open_window(options(kind, bounds), |window, cx| {
        let mini = cx.new(|cx| MiniWindow::new(window, cx));
        cx.new(|cx| Root::new(mini, window, cx))
    })
    .map(|handle| handle.into())
    .ok()
}

/// Whether a mini player is up, in either of its forms.
pub fn is_open(cx: &App) -> bool {
    cx.try_global::<MiniHandle>()
        .is_some_and(|handle| handle.0.is_some())
        || Chrome::global(cx).read(cx).is_mini()
}

/// Shrink to the mini player: a window of its own where the compositor allows it, else in place.
pub fn open_mini(cx: &mut App) {
    if is_open(cx) {
        return;
    }
    // An always-on-top surface is a layer-shell one; a plain second window cannot ask to stay above.
    let handle = open_window(overlay(), cx).or_else(|| {
        tracing::info!("no layer shell for the mini player; opening a plain window instead");
        open_window(WindowKind::Normal, cx)
    });
    match handle {
        Some(handle) => cx.set_global(MiniHandle(Some(handle))),
        None => {
            tracing::warn!("no second window for the mini player; shrinking the main one");
            Chrome::global(cx).update(cx, |chrome, cx| chrome.set_mini(true, cx));
        }
    }
}

/// Put the full player back, whichever way the mini one was showing.
pub fn close_mini(cx: &mut App) {
    if let Some(handle) = cx.try_global::<MiniHandle>().and_then(|handle| handle.0) {
        cx.set_global(MiniHandle(None));
        // The restore button runs inside the mini window's own update, where GPUI cannot reach that window.
        cx.defer(move |cx| {
            if let Err(error) = handle.update(cx, |_, window, _| window.remove_window()) {
                tracing::warn!(%error, "could not close the mini player window");
            }
        });
    }
    Chrome::global(cx).update(cx, |chrome, cx| chrome.set_mini(false, cx));
}

pub fn toggle_mini(cx: &mut App) {
    if is_open(cx) {
        close_mini(cx);
    } else {
        open_mini(cx);
    }
}
