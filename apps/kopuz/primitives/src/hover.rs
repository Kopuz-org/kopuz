//! Where a hover listener goes when the thing you want to hover already has one.

use gpui_kit::{
    Div, Entity, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, div,
};

/// `control` in a frame that owns the hover, since a Button with a tooltip has spent its one listener.
pub fn framed(id: &str, state: Entity<bool>, control: impl IntoElement) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("{id}-frame")))
        .flex()
        .on_hover(crate::motion::track(state))
        .child(control)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::component::button::Button;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Context, InputEvent as _, MouseMoveEvent, Render, TestAppContext, Window,
        point, px,
    };

    struct Fixture {
        hover: Entity<bool>,
        clicks: Rc<Cell<usize>>,
        loading: bool,
    }

    impl Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            div().size(px(200.)).child(framed(
                "action",
                self.hover.clone(),
                Button::new("action")
                    .label("Play")
                    .tooltip("Play playlist")
                    .loading(self.loading)
                    .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
            ))
        }
    }

    #[gpui_kit::test]
    fn a_framed_button_keeps_its_tooltip_its_hover_and_its_clicks(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for loading in [false, true] {
            let hover = cx.new(|_| false);
            let clicks = Rc::new(Cell::new(0));
            let view = cx.add_window({
                let hover = hover.clone();
                let clicks = clicks.clone();
                move |window, cx| {
                    let fixture = cx.new(|_| Fixture {
                        hover,
                        clicks,
                        loading,
                    });
                    Root::new(fixture, window, cx)
                }
            });
            cx.update_window(view.into(), |_, window, cx| {
                window.render_frame(cx);
                window.hover("action", cx);
                assert!(*hover.read(cx));
                window.click("action", cx);
                assert_eq!(clicks.get(), usize::from(!loading));
                window.dispatch_event(
                    MouseMoveEvent {
                        position: point(px(180.), px(180.)),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
                assert!(!*hover.read(cx));
            })
            .unwrap();
        }
    }
}
