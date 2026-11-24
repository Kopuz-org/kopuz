use gpui_kit::component::{ActiveTheme as _, Root};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, Global, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, Styled, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions, div,
};

use crate::NowPlaying;

#[derive(Default)]
struct StageHandle(Option<AnyWindowHandle>);

impl Global for StageHandle {}

pub struct StageWindow {
    player: Entity<NowPlaying>,
}

impl Render for StageWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("stage-window")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_key_down(|event: &KeyDownEvent, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            })
            .child(self.player.clone())
            .children(Root::render_notification_layer(window, cx))
    }
}

fn open_stage(cx: &App) -> Option<AnyWindowHandle> {
    let handle = cx.try_global::<StageHandle>()?.0?;
    cx.windows()
        .iter()
        .any(|open| open.window_id() == handle.window_id())
        .then_some(handle)
}

/// Take the player off the other display, if it is there.
pub fn close_stage(cx: &mut App) {
    let Some(handle) = open_stage(cx) else {
        return;
    };
    cx.set_global(StageHandle(None));
    cx.defer(move |cx| {
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
    });
}

pub fn toggle_stage(cx: &mut App) {
    if open_stage(cx).is_some() {
        close_stage(cx);
        return;
    }
    let here = cx
        .active_window()
        .and_then(|active| active.update(cx, |_, window, cx| window.display(cx)).ok())
        .flatten()
        .map(|display| display.id());
    let displays = cx.displays();
    let Some(target) = displays
        .iter()
        .find(|display| Some(display.id()) != here)
        .or_else(|| displays.first())
        .cloned()
    else {
        return;
    };
    let flavor = config::flavor::Flavor::current();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Fullscreen(target.bounds())),
        display_id: Some(target.id()),
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some(flavor.name().into()),
        titlebar: Some(TitlebarOptions {
            title: Some(flavor.display_name().into()),
            ..Default::default()
        }),
        kind: WindowKind::Normal,
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let stage = cx.new(|cx| StageWindow {
            player: cx.new(|cx| NowPlaying::detached_sheet(window, cx)),
        });
        cx.new(|cx| Root::new(stage, window, cx))
    });
    match opened {
        Ok(handle) => cx.set_global(StageHandle(Some(handle.into()))),
        Err(error) => tracing::warn!(%error, "could not open the player on another display"),
    }
}
