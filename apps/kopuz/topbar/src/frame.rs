//! The window's frame as `ui.titlebar_mode` asks: decorations, the app-drawn titlebar, and the edges that resize it.

use components::format::t;
use components::{icons, style};
use config::TitlebarMode;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Decorations, Div, ElementId, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, Point, ResizeEdge, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, TitlebarOptions, Window, WindowControlArea,
    WindowDecorations, WindowOptions, div, point, px,
};
use store::ConfigModel;

/// `h-9`, the height the Dioxus app's titlebar strip was.
pub const TITLEBAR_HEIGHT: Pixels = px(36.);
/// `w-11`, one window button.
const BUTTON_WIDTH: Pixels = px(44.);
const BUTTON_GLYPH: Pixels = px(12.);
/// What macOS's three window buttons take from the bar's leading edge, margins included.
const TRAFFIC_LIGHTS: Pixels = px(80.);
const TRAFFIC_LIGHT_X: Pixels = px(14.);
/// The height of one traffic light, for centring the three in the bar.
const TRAFFIC_LIGHT_SIZE: Pixels = px(14.);
/// The resize edges: thin strips along the sides and larger squares at the corners.
const EDGE: Pixels = px(5.);
const CORNER: Pixels = px(12.);
// `bg-black/50` and `text-white/35`: the strip as the Dioxus app drew it.
const STRIP_SCRIM: f32 = 0.5;
const TITLE_INK: f32 = 0.35;
const BUTTON_INK: f32 = 0.25;
// `hover:bg-red-500/70` on the close button.
const CLOSE_HOVER: f32 = 0.7;

/// The titlebar the settings ask for.
pub fn mode(cx: &App) -> TitlebarMode {
    ConfigModel::global(cx).read(cx).config().ui.titlebar_mode
}

/// Whether the platform's titlebar is hidden and the window's content runs up under it (macOS and Windows).
pub fn transparent_titlebar(mode: TitlebarMode) -> bool {
    cfg!(any(target_os = "macos", target_os = "windows")) && mode != TitlebarMode::System
}

/// Whether the app draws the window's frame itself instead of the compositor (Linux).
pub fn client_decorated(mode: TitlebarMode) -> bool {
    cfg!(target_os = "linux") && mode != TitlebarMode::System
}

/// Whether the app draws a titlebar strip with its own window buttons.
pub fn draws_titlebar(mode: TitlebarMode) -> bool {
    cfg!(any(target_os = "linux", target_os = "windows")) && mode == TitlebarMode::Custom
}

/// The decorations Linux should give the window for `mode`.
pub fn decorations(mode: TitlebarMode) -> WindowDecorations {
    match client_decorated(mode) {
        true => WindowDecorations::Client,
        false => WindowDecorations::Server,
    }
}

/// The frame part of a window's options for `mode`; `bar_height` centres macOS's buttons in the bar.
pub fn window_options(mode: TitlebarMode, bar_height: Pixels, options: &mut WindowOptions) {
    let transparent = transparent_titlebar(mode);
    let traffic_lights: Option<Point<Pixels>> = (cfg!(target_os = "macos") && transparent)
        .then(|| point(TRAFFIC_LIGHT_X, (bar_height - TRAFFIC_LIGHT_SIZE) / 2.));
    options.titlebar = Some(TitlebarOptions {
        title: Some(config::flavor::Flavor::current().display_name().into()),
        appears_transparent: transparent,
        traffic_light_position: traffic_lights,
    });
    // macOS would otherwise hold back every click near the top while it waits to see if it is a double click.
    options.app_owns_titlebar_drag = cfg!(target_os = "macos") && transparent;
    options.window_decorations = Some(decorations(mode));
}

/// The room macOS's window buttons need at the bar's leading edge, or `None` where they are not over the content.
pub fn traffic_lights(window: &Window, cx: &App) -> Option<Pixels> {
    (cfg!(target_os = "macos") && transparent_titlebar(mode(cx)) && !window.is_fullscreen())
        .then_some(TRAFFIC_LIGHTS)
}

/// An empty stretch that moves the window when pressed and dragged and zooms it when double-clicked, unless the platform's titlebar is there to.
pub fn drag_region(id: impl Into<ElementId>, window: &Window, cx: &App) -> Stateful<Div> {
    let region = div().id(id);
    if !app_drags(mode(cx), window.window_decorations()) {
        return region;
    }
    region
        // Windows answers its own hit test from this, which is what gives the strip snap and a double click.
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count >= 2 {
                double_click(window);
            } else {
                window.start_window_move();
            }
        })
}

/// Whether the app moves the window itself: asked to, or left without a server titlebar.
fn app_drags(mode: TitlebarMode, decorations: Decorations) -> bool {
    mode != TitlebarMode::System || client_side(decorations)
}

/// Whether the app draws the frame, either by asking or because the compositor offers no server decorations.
fn client_side(decorations: Decorations) -> bool {
    matches!(decorations, Decorations::Client { .. })
}

fn double_click(window: &Window) {
    #[cfg(target_os = "macos")]
    window.titlebar_double_click();
    #[cfg(not(target_os = "macos"))]
    window.zoom_window();
}

/// The app-drawn titlebar: a drag strip with the app's name over it and the window buttons at the end.
pub fn titlebar(window: &Window, cx: &App) -> Option<AnyElement> {
    if !draws_titlebar(mode(cx)) {
        return None;
    }
    let zoom_glyph = match window.is_maximized() {
        true => icons::WINDOW_RESTORE,
        false => icons::WINDOW_MAXIMIZE,
    };
    Some(
        h_flex()
            .relative()
            .w_full()
            .h(TITLEBAR_HEIGHT)
            .flex_shrink_0()
            .bg(style::black(STRIP_SCRIM, cx))
            .border_b_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .child(drag_region("titlebar-drag", window, cx).h_full().flex_1())
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .text_color(style::white(TITLE_INK, cx))
                    .child(config::flavor::Flavor::current().display_name()),
            )
            .child(
                h_flex()
                    .h_full()
                    .child(
                        window_button(
                            "titlebar-minimize",
                            icons::WINDOW_MINIMIZE,
                            t("minimize"),
                            cx,
                        )
                        .window_control_area(WindowControlArea::Min)
                        .on_click(|_, window, _| window.minimize_window()),
                    )
                    .child(
                        window_button("titlebar-zoom", zoom_glyph, t("maximize"), cx)
                            .window_control_area(WindowControlArea::Max)
                            .on_click(|_, window, _| window.zoom_window()),
                    )
                    .child(
                        window_button("titlebar-close", icons::CLOSE, t("close"), cx)
                            .window_control_area(WindowControlArea::Close)
                            .hover(|button| button.bg(cx.theme().danger.opacity(CLOSE_HOVER)))
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
            .into_any_element(),
    )
}

fn window_button(
    id: &'static str,
    glyph: &'static str,
    label: SharedString,
    cx: &App,
) -> Stateful<Div> {
    div()
        .id(id)
        .h_full()
        .w(BUTTON_WIDTH)
        .flex()
        .items_center()
        .justify_center()
        .text_color(style::white(BUTTON_INK, cx))
        .hover(|button| {
            button
                .text_color(style::white(style::TEXT_STRONG, cx))
                .bg(style::white(style::RAISED, cx))
        })
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(label.clone()).build(window, cx)
        })
        .child(icons::icon(glyph).size(BUTTON_GLYPH))
}

/// Invisible strips on the window's edges and corners that resize it, while the app draws the frame (Linux).
pub fn resize_handles(window: &Window) -> Option<AnyElement> {
    if !client_side(window.window_decorations()) || window.is_maximized() || window.is_fullscreen()
    {
        return None;
    }
    let handles = [
        (ResizeEdge::Top, "resize-top"),
        (ResizeEdge::Bottom, "resize-bottom"),
        (ResizeEdge::Left, "resize-left"),
        (ResizeEdge::Right, "resize-right"),
        (ResizeEdge::TopLeft, "resize-top-left"),
        (ResizeEdge::TopRight, "resize-top-right"),
        (ResizeEdge::BottomLeft, "resize-bottom-left"),
        (ResizeEdge::BottomRight, "resize-bottom-right"),
    ];
    Some(
        div()
            .absolute()
            .inset_0()
            .children(handles.into_iter().map(|(edge, id)| handle(edge, id)))
            .into_any_element(),
    )
}

fn handle(edge: ResizeEdge, id: &'static str) -> impl IntoElement {
    let strip = div().id(id).absolute().occlude();
    let strip = match edge {
        ResizeEdge::Top => strip.top_0().left_0().right_0().h(EDGE).cursor_ns_resize(),
        ResizeEdge::Bottom => strip
            .bottom_0()
            .left_0()
            .right_0()
            .h(EDGE)
            .cursor_ns_resize(),
        ResizeEdge::Left => strip.top_0().bottom_0().left_0().w(EDGE).cursor_ew_resize(),
        ResizeEdge::Right => strip
            .top_0()
            .bottom_0()
            .right_0()
            .w(EDGE)
            .cursor_ew_resize(),
        ResizeEdge::TopLeft => strip.top_0().left_0().size(CORNER).cursor_nwse_resize(),
        ResizeEdge::TopRight => strip.top_0().right_0().size(CORNER).cursor_nesw_resize(),
        ResizeEdge::BottomLeft => strip.bottom_0().left_0().size(CORNER).cursor_nesw_resize(),
        ResizeEdge::BottomRight => strip.bottom_0().right_0().size(CORNER).cursor_nwse_resize(),
    };
    strip.on_mouse_down(MouseButton::Left, move |_, window, cx| {
        cx.stop_propagation();
        window.start_window_resize(edge);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_decorations_leave_the_frame_to_the_platform() {
        assert!(!transparent_titlebar(TitlebarMode::System));
        assert!(!client_decorated(TitlebarMode::System));
        assert!(!draws_titlebar(TitlebarMode::System));
        assert_eq!(decorations(TitlebarMode::System), WindowDecorations::Server);
    }

    #[test]
    fn a_client_side_fallback_keeps_drag_and_resize() {
        let client = Decorations::Client {
            tiling: Default::default(),
        };
        assert!(app_drags(TitlebarMode::System, client));
        assert!(client_side(client));
        assert!(!app_drags(TitlebarMode::System, Decorations::Server));
        assert!(!client_side(Decorations::Server));
        assert!(app_drags(TitlebarMode::Custom, Decorations::Server));
    }

    #[test]
    fn only_custom_draws_a_strip() {
        assert!(!draws_titlebar(TitlebarMode::Off));
        assert_eq!(
            draws_titlebar(TitlebarMode::Custom),
            cfg!(any(target_os = "linux", target_os = "windows"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_hides_its_titlebar_unless_asked_for_the_system_one() {
        let mut options = WindowOptions::default();
        window_options(TitlebarMode::Custom, px(44.), &mut options);
        let titlebar = options.titlebar.expect("titlebar options");
        assert!(titlebar.appears_transparent);
        assert_eq!(
            titlebar.traffic_light_position,
            Some(point(px(14.), px(15.)))
        );
        assert!(options.app_owns_titlebar_drag);

        let mut options = WindowOptions::default();
        window_options(TitlebarMode::System, px(44.), &mut options);
        assert!(
            !options
                .titlebar
                .expect("titlebar options")
                .appears_transparent
        );
        assert!(!options.app_owns_titlebar_drag);
    }
}
