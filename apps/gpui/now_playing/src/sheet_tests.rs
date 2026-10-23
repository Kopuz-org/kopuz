use super::*;
use gpui_kit::{Render, ScrollDelta, ScrollWheelEvent, StatefulInteractiveElement as _};

struct Harness {
    modal: ModalFocus,
    background: FocusHandle,
    background_events: usize,
    player_events: usize,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id("background")
                    .size_full()
                    .track_focus(&self.background)
                    .on_any_mouse_down(cx.listener(|this, _, _, _| this.background_events += 1))
                    .on_scroll_wheel(cx.listener(|this, _, _, _| this.background_events += 1))
                    .on_key_down(cx.listener(|this, _, _, _| this.background_events += 1)),
            )
            .when(self.modal.open, |root| {
                root.child(
                    div().absolute().inset_0().child(
                        surface()
                            .on_key_down(cx.listener(|this, _, _, _| this.player_events += 1))
                            .child(
                                div()
                                    .id("player-control")
                                    .absolute()
                                    .top(px(100.))
                                    .left(px(100.))
                                    .size(px(40.))
                                    .on_click(cx.listener(|this, _, _, _| this.player_events += 1)),
                            )
                            .focus_trap("test-player-focus", &self.modal.handle),
                    ),
                )
            })
    }
}

#[gpui_kit::test]
fn fullscreen_blocks_background_input_and_restores_focus(cx: &mut gpui_kit::TestAppContext) {
    cx.update(gpui_kit::base::init);
    let (view, cx) = cx.add_window_view(|_, cx| Harness {
        modal: ModalFocus::new(cx),
        background: cx.focus_handle(),
        background_events: 0,
        player_events: 0,
    });
    cx.update(|window, cx| {
        view.read(cx).background.clone().focus(window, cx);
        window.draw(cx).clear(cx);
    });
    let scroll = ScrollWheelEvent {
        position: point(px(20.), px(20.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-50.))),
        ..Default::default()
    };
    cx.simulate_click(point(px(20.), px(20.)), Default::default());
    cx.simulate_event(scroll.clone());
    cx.simulate_keystrokes("a");
    assert_eq!(view.read_with(cx, |view, _| view.background_events), 3);

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.modal.set_open(true, window, cx);
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(gpui_kit::base::active_focus_trap(window, cx).is_some());
    });
    cx.simulate_click(point(px(20.), px(20.)), Default::default());
    cx.simulate_mouse_down(
        point(px(20.), px(20.)),
        gpui_kit::MouseButton::Right,
        Default::default(),
    );
    cx.simulate_mouse_up(
        point(px(20.), px(20.)),
        gpui_kit::MouseButton::Right,
        Default::default(),
    );
    cx.simulate_event(scroll);
    cx.simulate_keystrokes("a");
    cx.simulate_click(point(px(120.), px(120.)), Default::default());
    assert_eq!(view.read_with(cx, |view, _| view.background_events), 3);
    assert_eq!(view.read_with(cx, |view, _| view.player_events), 2);

    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.modal.set_open(false, window, cx);
            cx.notify();
        });
        window.draw(cx).clear(cx);
        assert!(view.read(cx).background.is_focused(window));
    });
    cx.simulate_click(point(px(20.), px(20.)), Default::default());
    assert_eq!(view.read_with(cx, |view, _| view.background_events), 4);
}
