//! The phone layout's measures, for the components and pages that draw a compact form.
//!
//! Only read behind [`is_compact`], which is true under the Android host alone, so desktop never sees them.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{
    AnyElement, App, Div, ElementId, InteractiveElement, IntoElement, LongPressEvent, Modifiers,
    MouseButton, MouseDownEvent, MouseUpEvent, ParentElement as _, Pixels, PlatformInput, Point,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, TouchPhase,
    Window, canvas, div, px,
};

use crate::{icons, style};

pub use theme::mobile::{TOUCH_TARGET, is_compact, is_touch};

/// A page's title on a phone, where the desktop's `text-3xl` would take a third of the width.
pub const TITLE: Pixels = px(22.);
/// Home's greeting on a phone.
pub const DISPLAY: Pixels = px(24.);
/// Body text: nothing a phone has to read is drawn smaller.
pub const BODY: Pixels = px(14.);
/// A list row's first line.
pub const ROW_TITLE: Pixels = px(15.);
/// The least a list row stands, so a thumb lands on one row and not its neighbour.
pub const ROW: Pixels = px(56.);
/// A page's padding at the sides and top.
pub const PAGE_PAD: Pixels = px(16.);
/// A detail page's cover, centred over its words.
pub const COVER: Pixels = px(200.);

/// The family for tabular digits: the theme's resolved mono family on a phone, which has no font named
/// `monospace` and would panic on the first line laid out in it; `monospace` on desktop, as before.
pub fn mono_family(cx: &App) -> SharedString {
    match is_compact(cx) {
        true => cx.theme().mono_font_family.clone(),
        false => SharedString::new_static("monospace"),
    }
}

/// A hit box no smaller than a touch target.
pub fn target(size: Pixels) -> Pixels {
    if size < TOUCH_TARGET {
        TOUCH_TARGET
    } else {
        size
    }
}

/// What a toolbar control looks like inside its touch target: smaller than the thumb's box, so a row of them reads light.
pub const FACE: Pixels = px(40.);
/// A toolbar glyph.
const GLYPH: Pixels = px(18.);

/// The face a toolbar control wears: a rounded square with its glyph, washed brighter when `lit`.
pub fn face(icon: &'static str, lit: bool, cx: &App) -> Div {
    let (wash, ink) = match lit {
        true => (style::white(style::HOVER, cx), cx.theme().foreground),
        false => (style::white(style::RAISED, cx), style::white(0.7, cx)),
    };
    div()
        .size(FACE)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(style::radius_lg())
        .bg(wash)
        .border_1()
        .border_color(style::white(style::BORDER_SOFT, cx))
        .text_color(ink)
        .child(icons::icon(icon).size(GLYPH).text_color(ink))
}

/// A phone toolbar's button: the touch target's box is the button itself, with the smaller face drawn inside,
/// so a row of them sits edge to edge and every press lands on exactly one handler.
pub fn tool_button(
    id: impl Into<ElementId>,
    icon: &'static str,
    label: impl Into<SharedString>,
    cx: &App,
) -> Button {
    let clear = gpui_kit::transparent_black();
    Button::new(id)
        .custom(style::bare_button(clear, cx))
        .tooltip(label)
        .size(TOUCH_TARGET)
        .p_0()
        .flex_shrink_0()
        .child(face(icon, false, cx))
}

/// Grid and list as two touch targets side by side, the picked one's face lit; `set` is told whether grid was picked.
pub fn view_switch(
    id: impl Into<SharedString>,
    grid: bool,
    set: impl Fn(bool, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let id = id.into();
    let set = Rc::new(set);
    let option = |key: &str, icon: &'static str, tip: SharedString, pick: bool| {
        let set = set.clone();
        div()
            .id(SharedString::from(format!("{id}-{key}")))
            .size(TOUCH_TARGET)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .tooltip(move |window: &mut Window, cx: &mut App| {
                gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
            .on_click(move |_, _, cx| set(pick, cx))
            .child(face(icon, grid == pick, cx))
    };
    div()
        .flex()
        .flex_shrink_0()
        .child(option(
            "grid",
            icons::GRID,
            crate::format::t("view_grid"),
            true,
        ))
        .child(option(
            "list",
            icons::ROWS,
            crate::format::t("view_list"),
            false,
        ))
        .into_any_element()
}

/// How far a pan has to travel along its axis before it counts as a swipe.
pub const SWIPE: Pixels = px(56.);

/// A touch pan across an element, measured from where the finger landed.
#[derive(Clone, Copy, Debug)]
pub struct Pan {
    pub offset: Point<Pixels>,
    pub phase: TouchPhase,
}

/// The way a finished pan swept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swipe {
    Left,
    Right,
    Up,
    Down,
}

impl Pan {
    /// Whether the finger has lifted, or the system took the touch away.
    pub fn is_over(&self) -> bool {
        matches!(self.phase, TouchPhase::Ended | TouchPhase::Cancelled)
    }

    /// The way a pan that ended swept, once it went [`SWIPE`] along its axis; a cancelled one never swept.
    pub fn swipe(&self) -> Option<Swipe> {
        if self.phase != TouchPhase::Ended {
            return None;
        }
        let Point { x, y } = self.offset;
        let (along, back, forth) = match x.abs() >= y.abs() {
            true => (x, Swipe::Left, Swipe::Right),
            false => (y, Swipe::Up, Swipe::Down),
        };
        match along {
            along if along <= -SWIPE => Some(back),
            along if along >= SWIPE => Some(forth),
            _ => None,
        }
    }
}

type PanListener = Rc<dyn Fn(Pan, &mut Window, &mut App)>;

thread_local! {
    /// The pan in flight and the element it began on: one finger pans at a time.
    static PAN: RefCell<Option<(Point<Pixels>, PanListener)>> = const { RefCell::new(None) };
}

/// `element` hears the touch pans that begin on it, from the first move to the lift; on a touch screen only.
///
/// GPUI turns a pan into scroll events hit-tested where the touch began, so an element that does not
/// scroll can follow one without claiming the touch, and taps on its buttons still land. Put it only on
/// parts that do not scroll, since a scroll container lets the same events through. The pan's later
/// events go to the element it began on through [`touch_gestures`], even once what it moves has slid
/// out from under where the touch began.
pub fn on_pan<E: InteractiveElement>(
    element: E,
    listener: impl Fn(Pan, &mut Window, &mut App) + 'static,
    cx: &App,
) -> E {
    if !is_touch(cx) {
        return element;
    }
    let listener: PanListener = Rc::new(listener);
    element.on_scroll_wheel(move |event, window, cx| {
        if event.touch_phase != TouchPhase::Started {
            return;
        }
        let offset = event.delta.pixel_delta(px(16.));
        PAN.with_borrow_mut(|pan| *pan = Some((offset, listener.clone())));
        cx.stop_propagation();
        listener(
            Pan {
                offset,
                phase: TouchPhase::Started,
            },
            window,
            cx,
        );
    })
}

/// Hands a pan's moves and lift to the element it began on, ahead of anything under it now; a fling's
/// coasting steps arrive after the lift, with no pan left, and scroll as usual.
fn route_pans(window: &mut Window) {
    window.on_mouse_event(|event: &ScrollWheelEvent, phase, window, cx| {
        if !phase.capture() || event.touch_phase == TouchPhase::Started {
            return;
        }
        let delta = event.delta.pixel_delta(px(16.));
        let routed = PAN.with_borrow_mut(|pan| {
            let (offset, listener) = pan.as_mut()?;
            if event.touch_phase != TouchPhase::Cancelled {
                *offset += delta;
            }
            let routed = (
                Pan {
                    offset: *offset,
                    phase: event.touch_phase,
                },
                listener.clone(),
            );
            if routed.0.is_over() {
                *pan = None;
            }
            Some(routed)
        });
        let Some((pan, listener)) = routed else {
            return;
        };
        cx.stop_propagation();
        listener(pan, window, cx);
    });
}

/// How long, and how far, a touch that stops a coasting fling may last to still count as a tap.
const CATCH_TAP: std::time::Duration = std::time::Duration::from_millis(300);
const CATCH_SLOP: Pixels = px(8.);

/// What [`catch_taps`] has seen of the scroll stream.
#[derive(Default)]
struct Catching {
    /// A finger is down on a pan; otherwise scroll steps are a fling coasting.
    finger: bool,
    /// When a fling last coasted.
    coasted: Option<std::time::Instant>,
    /// A touch that stopped a fling: where it landed, when, and how far it has moved since.
    caught: Option<(Point<Pixels>, std::time::Instant, Pixels)>,
}

thread_local! {
    static CATCHING: RefCell<Catching> = RefCell::new(Catching::default());
}

/// GPUI turns a touch that lands on a coasting fling into a pan that only stops it, never a tap,
/// and a fling coasts on after the list looks still, or has hit its end. So one tap stopped the
/// list and a second one pressed what was under it. A quick touch that stops a fling without moving
/// is replayed as the click it was meant to be.
fn catch_taps(window: &mut Window) {
    window.on_mouse_event(|event: &ScrollWheelEvent, phase, window, cx| {
        if !phase.capture() {
            return;
        }
        let delta = event.delta.pixel_delta(px(16.));
        let tap = CATCHING.with_borrow_mut(|catching| {
            let now = std::time::Instant::now();
            match event.touch_phase {
                TouchPhase::Started => {
                    let coasting = catching.coasted.is_some_and(|at| {
                        now.duration_since(at) < std::time::Duration::from_millis(100)
                    });
                    catching.finger = true;
                    catching.coasted = None;
                    catching.caught = (coasting && delta == Point::default()).then_some((
                        event.position,
                        now,
                        Pixels::ZERO,
                    ));
                    None
                }
                TouchPhase::Moved if catching.finger => {
                    if let Some((_, _, moved)) = catching.caught.as_mut() {
                        *moved += delta.x.abs() + delta.y.abs();
                        if *moved > CATCH_SLOP {
                            catching.caught = None;
                        }
                    }
                    None
                }
                TouchPhase::Moved => {
                    catching.coasted = Some(now);
                    None
                }
                TouchPhase::Ended | TouchPhase::Cancelled if catching.finger => {
                    catching.finger = false;
                    let caught = catching.caught.take();
                    let released = event.touch_phase == TouchPhase::Ended;
                    caught.filter(|(_, at, moved)| {
                        released
                            && now.duration_since(*at) < CATCH_TAP
                            && *moved + delta.x.abs() + delta.y.abs() <= CATCH_SLOP
                    })
                }
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    catching.coasted = Some(now);
                    None
                }
            }
        });
        let Some((position, _, _)) = tap else {
            return;
        };
        window.defer(cx, move |window, cx| {
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
        });
    });
}

/// The touch screen's window-wide gestures, mounted once by the shell, first, under everything: a tap
/// that stops a fling still pressing what it landed on ([`catch_taps`]), pans routed to the element
/// they began on ([`on_pan`]), and a press held on anything with a context menu opening it, as a
/// right-click does on desktop.
///
/// The menus open on a right mouse-down, so the press is replayed as one where it landed, once the
/// gesture's own dispatch is over. Inputs and tooltips that answer a long press themselves claim it
/// before it gets here. Claiming it keeps the lift from also tapping.
pub fn touch_gestures() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _| {
            catch_taps(window);
            route_pans(window);
            window.on_mouse_event(|event: &LongPressEvent, phase, window, cx| {
                if !phase.bubble()
                    || event.phase != TouchPhase::Started
                    || window.default_prevented()
                {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                let position = event.start_position;
                window.defer(cx, move |window, cx| {
                    window.dispatch_event(
                        PlatformInput::MouseDown(MouseDownEvent {
                            button: MouseButton::Right,
                            position,
                            modifiers: Modifiers::default(),
                            click_count: 1,
                            first_mouse: false,
                        }),
                        cx,
                    );
                    window.dispatch_event(
                        PlatformInput::MouseUp(MouseUpEvent {
                            button: MouseButton::Right,
                            position,
                            modifiers: Modifiers::default(),
                            click_count: 1,
                        }),
                        cx,
                    );
                });
            });
        },
    )
    .absolute()
    .size_0()
}

/// How far a detail page's rows scroll before its header folds into [`slim_header`].
pub const COLLAPSE: Pixels = px(24.);

/// Whether rows scrolled `scrolled` from the top fold their page's header away.
pub fn collapsed(scrolled: Pixels) -> bool {
    scrolled > COLLAPSE
}

/// A detail page's header once its rows scroll: a small cover, the title and play, so the rows get the
/// screen; scrolling back to the top unfolds the full header.
pub fn slim_header(
    id: impl Into<SharedString>,
    title: SharedString,
    artwork: Option<&api::ArtworkRef>,
    round: bool,
    play: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let id = id.into();
    let cover = crate::Cover::new(artwork).size(px(44.));
    let cover = match round {
        true => cover.rounded(px(22.)),
        false => cover.rounded(style::thumb_radius()),
    };
    gpui_kit::component::h_flex()
        .w_full()
        .h(px(56.))
        .flex_shrink_0()
        .items_center()
        .gap_3()
        .child(cover)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(ROW_TITLE)
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            Button::new(SharedString::from(format!("{id}-play")))
                .primary()
                .rounded_full()
                .size(TOUCH_TARGET)
                .icon(icons::icon(icons::PLAY))
                .on_click(move |_, window, cx| play(window, cx)),
        )
        .text_color(cx.theme().foreground)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Context, Render, ScrollDelta, TestAppContext, TouchEvent, TouchId, point};
    use std::cell::Cell;

    struct GestureFixture {
        events: Rc<RefCell<Vec<Pan>>>,
        clicks: Rc<Cell<usize>>,
        offset: Pixels,
    }

    impl Render for GestureFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            let owner = cx.entity().downgrade();
            let target = div()
                .id("pan-target")
                .relative()
                .top(self.offset)
                .w(px(200.))
                .h(px(200.))
                .on_click(move |_, _, _| clicks.set(clicks.get() + 1));
            div()
                .relative()
                .size(px(500.))
                .child(touch_gestures())
                .child(on_pan(
                    target,
                    move |pan, _, cx| {
                        owner
                            .update(cx, |this, cx| {
                                this.events.borrow_mut().push(pan);
                                this.offset = pan.offset.y;
                                cx.notify();
                            })
                            .ok();
                    },
                    cx,
                ))
        }
    }

    #[gpui_kit::test]
    fn a_pan_finishes_after_its_target_moves_away(cx: &mut TestAppContext) {
        PAN.with_borrow_mut(|pan| *pan = None);
        CATCHING.with_borrow_mut(|state| *state = Catching::default());
        cx.update(|cx| theme::mobile::set_mobile(Default::default(), cx));
        let events = Rc::new(RefCell::new(Vec::new()));
        let clicks = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view(|_, _| GestureFixture {
            events: events.clone(),
            clicks: clicks.clone(),
            offset: px(0.),
        });
        cx.run_until_parked();
        for (phase, y) in [
            (TouchPhase::Started, 50.),
            (TouchPhase::Moved, 150.),
            (TouchPhase::Moved, 250.),
            (TouchPhase::Ended, 280.),
        ] {
            cx.simulate_event(TouchEvent {
                id: TouchId(1),
                phase,
                position: point(px(50.), px(y)),
                ..Default::default()
            });
            cx.run_until_parked();
        }
        let events = events.borrow();
        assert_eq!(
            events.first().map(|pan| pan.phase),
            Some(TouchPhase::Started)
        );
        let last = events.last().expect("the release reaches the moved target");
        assert_eq!(last.phase, TouchPhase::Ended);
        assert_eq!(last.offset.y, px(230.));
        assert_eq!(last.swipe(), Some(Swipe::Down));
        assert_eq!(clicks.get(), 0);
    }

    #[test]
    fn cancelled_and_short_pans_never_skip_tracks() {
        for phase in [
            TouchPhase::Started,
            TouchPhase::Moved,
            TouchPhase::Cancelled,
        ] {
            assert_eq!(
                Pan {
                    offset: gpui_kit::point(px(100.), px(0.)),
                    phase
                }
                .swipe(),
                None
            );
        }
        assert_eq!(
            Pan {
                offset: gpui_kit::point(SWIPE - px(1.), px(0.)),
                phase: TouchPhase::Ended,
            }
            .swipe(),
            None
        );
    }

    #[gpui_kit::test]
    fn catching_a_fling_clicks_once_on_release(cx: &mut TestAppContext) {
        PAN.with_borrow_mut(|pan| *pan = None);
        CATCHING.with_borrow_mut(|state| *state = Catching::default());
        cx.update(|cx| theme::mobile::set_mobile(Default::default(), cx));
        let clicks = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view(|_, _| GestureFixture {
            events: Rc::default(),
            clicks: clicks.clone(),
            offset: px(0.),
        });
        cx.run_until_parked();
        for phase in [
            TouchPhase::Moved,
            TouchPhase::Ended,
            TouchPhase::Started,
            TouchPhase::Ended,
        ] {
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(50.), px(50.)),
                delta: ScrollDelta::Pixels(Point::default()),
                touch_phase: phase,
                ..Default::default()
            });
        }
        cx.run_until_parked();
        assert_eq!(clicks.get(), 1);
    }
}
