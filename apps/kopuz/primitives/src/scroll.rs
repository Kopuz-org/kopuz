//! Smooth scrolling. GPUI turns a wheel notch into `line_height * delta` and applies it at once,
//! so the view teleports; here the drawn offset glides toward where the wheel has put it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{App, ElementId, Pixels, ScrollHandle, UniformListScrollHandle, Window, point, px};

use crate::motion;

/// How long the view takes to catch up with the wheel. Long enough to read as motion, short
/// enough that the list still feels bolted to the pointer.
pub const GLIDE: Duration = Duration::from_millis(140);

/// How close the drawn offset has to be to the target before a row is allowed to settle on a tile.
const SETTLED: Pixels = px(0.5);

/// Where the wheel has put this scroller, and what was last drawn for it.
#[derive(Default, Clone, Copy)]
struct Glide {
    target: Pixels,
    drawn: Pixels,
    started: bool,
}

/// Glides `handle` toward wherever the wheel left it. Call it from the render that draws the
/// scroller, before the element is built.
///
/// GPUI owns the wheel: its handler adds the delta straight to the offset and there is no
/// cancelling it from a listener. So the offset it writes is read as the *target*, the tween
/// supplies what is actually drawn, and the difference between the two is how much wheel
/// arrived since the last frame.
pub fn smooth(id: impl Into<ElementId>, handle: &ScrollHandle, window: &mut Window, cx: &mut App) {
    let id: ElementId = id.into();
    // With motion off, leaving the offset alone is exactly GPUI's own scrolling.
    if motion::span(GLIDE, cx).is_none() {
        return;
    }
    let state = held(&id, "glide", window, cx);
    let offset = handle.offset();
    let limit = handle.max_offset().y;

    let mut target = match state.1.started {
        // Whatever GPUI added since the last frame is wheel the tween has not accounted for.
        true => state.1.target + (offset.y - state.1.drawn),
        false => offset.y,
    };
    // Offsets run negative going down, and `max_offset` is a magnitude.
    target = target.clamp(-limit, px(0.));

    let drawn = motion::length(id, target, GLIDE, window, cx);
    handle.set_offset(point(offset.x, drawn));
    record(state, target, drawn, cx);
}

/// The same glide sideways, settling on a multiple of `step` — a tile and its gap; `px(0.)` never settles.
pub fn smooth_x(
    id: impl Into<ElementId>,
    handle: &ScrollHandle,
    step: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let id: ElementId = id.into();
    if motion::span(GLIDE, cx).is_none() {
        return;
    }
    let state = held(&id, "glide-x", window, cx);
    let offset = handle.offset();
    let limit = handle.max_offset().x;

    let mut target = match state.1.started {
        true => state.1.target + (offset.x - state.1.drawn),
        false => offset.x,
    };
    target = target.clamp(-limit, px(0.));
    // Snapping mid-glide would fight the wheel, so the row only settles once it has caught up.
    if step > px(0.) && (target - state.1.drawn).abs() < SETTLED {
        target = settle(target, step, limit);
    }

    let drawn = motion::length(id, target, GLIDE, window, cx);
    handle.set_offset(point(drawn, offset.y));
    record(state, target, drawn, cx);
}

/// A row a list wants its page scroller to show. The page's glide lives in the page's own element
/// scope, so the list hands the row over and the page's [`smooth_revealing`] re-aims the glide.
#[derive(Clone)]
pub struct Reveal {
    handle: ScrollHandle,
    asked: Rc<Cell<Option<(Pixels, Pixels)>>>,
}

impl Reveal {
    pub fn new(handle: ScrollHandle) -> Self {
        Self {
            handle,
            asked: Rc::default(),
        }
    }

    /// The page's offset now, which a list records beside where it painted its rows.
    pub fn offset(&self) -> Pixels {
        self.handle.offset().y
    }

    /// Show a row `height` tall whose window top, with the page at offset zero, is `rest`.
    pub fn ask(&self, rest: Pixels, height: Pixels) {
        self.asked.set(Some((rest, height)));
    }
}

/// [`smooth`], first bringing into view the row a list asked for since the last frame.
pub fn smooth_revealing(
    id: impl Into<ElementId>,
    reveal: &Reveal,
    window: &mut Window,
    cx: &mut App,
) {
    let id: ElementId = id.into();
    let handle = &reveal.handle;
    if let Some((rest, height)) = reveal.asked.take() {
        let viewport = handle.bounds();
        // Room for the row's focus outline, which a flush edge would clip.
        let view = (viewport.top() + px(4.), viewport.bottom() - px(4.));
        let offset = handle.offset();
        let (target, drawn) = match motion::span(GLIDE, cx) {
            Some(_) => match held(&id, "glide", window, cx).1 {
                glide if glide.started => (glide.target, glide.drawn),
                _ => (offset.y, offset.y),
            },
            None => (offset.y, offset.y),
        };
        if let Some(y) = aim(target, drawn, offset.y, view, rest + drawn, height) {
            handle.set_offset(point(offset.x, y));
        }
    }
    smooth(id, handle, window, cx);
}

/// The offset to write so [`smooth`] lands where a row starting at `top` (as drawn at `drawn`)
/// is in `view`. [`smooth`] reads `offset - drawn` as fresh wheel on top of `target`.
fn aim(
    target: Pixels,
    drawn: Pixels,
    offset: Pixels,
    (view_top, view_bottom): (Pixels, Pixels),
    top: Pixels,
    height: Pixels,
) -> Option<Pixels> {
    let heading = target + (offset - drawn);
    let landed = top + (heading - drawn);
    let wanted = if landed < view_top {
        heading + (view_top - landed)
    } else if landed + height > view_bottom {
        heading - (landed + height - view_bottom)
    } else {
        return None;
    };
    Some(drawn + (wanted - target))
}

/// Move a horizontal scroller by `by`; [`smooth_x`] reads the new offset as target, so an arrow glides too.
pub fn nudge_x(handle: &ScrollHandle, by: Pixels) {
    let offset = handle.offset();
    let x = (offset.x - by).clamp(-handle.max_offset().x, px(0.));
    handle.set_offset(point(x, offset.y));
}

/// How much room a horizontal scroller has left, either way: what the edge fades and arrows read.
pub fn room_x(handle: &ScrollHandle) -> (bool, bool) {
    let offset = handle.offset().x;
    let limit = handle.max_offset().x;
    (offset < px(-1.), offset > px(1.) - limit)
}

/// The tile edge nearest `offset`, keeping the far end reachable when it is not a whole step away.
fn settle(offset: Pixels, step: Pixels, limit: Pixels) -> Pixels {
    let (step, limit) = (step.as_f32(), limit.as_f32());
    if offset <= px(step * 0.5 - limit) {
        return px(-limit);
    }
    px(-((-offset.as_f32() / step).round() * step)).clamp(px(-limit), px(0.))
}

/// The glide this scroller left behind last frame, keyed off the element's own id.
fn held(
    id: &ElementId,
    axis: &'static str,
    window: &mut Window,
    cx: &mut App,
) -> (gpui_kit::Entity<Glide>, Glide) {
    let key = ElementId::Name(format!("{id:?}-{axis}").into());
    let state = window.use_keyed_state(key, cx, |_, _| Glide::default());
    let held = *state.read(cx);
    (state, held)
}

/// Remember what this frame drew, so the next one can tell wheel from tween.
fn record(state: (gpui_kit::Entity<Glide>, Glide), target: Pixels, drawn: Pixels, cx: &mut App) {
    let (entity, held) = state;
    if held.target == target && held.drawn == drawn && held.started {
        return;
    }
    entity.update(cx, |glide, _| {
        glide.target = target;
        glide.drawn = drawn;
        glide.started = true;
    });
}

/// The same for a `uniform_list`, which keeps an ordinary handle underneath.
pub fn smooth_list(
    id: impl Into<ElementId>,
    handle: &UniformListScrollHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let base = handle.0.borrow().base_handle.clone();
    smooth(id, &base, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reveal_mid_glide_aims_the_glide_instead_of_adding_to_it() {
        let view = (px(100.), px(500.));
        // Resting: the row 20 below the viewport moves the page by exactly that.
        assert_eq!(
            aim(px(0.), px(0.), px(0.), view, px(480.), px(40.)),
            Some(px(-20.))
        );
        assert_eq!(aim(px(0.), px(0.), px(0.), view, px(200.), px(40.)), None);
        // Drawn at -100 but heading to -500: the glide is re-aimed to end with the row at the bottom.
        let written = aim(px(-500.), px(-100.), px(-100.), view, px(1000.), px(40.));
        assert_eq!(written, Some(px(-240.)));
        // What `smooth` then makes of it: the old target plus the new wheel.
        assert_eq!(px(-500.) + (px(-240.) - px(-100.)), px(-640.));
        // A row that lands in view once the glide ends needs nothing.
        assert_eq!(
            aim(px(-500.), px(-100.), px(-100.), view, px(660.), px(40.)),
            None
        );
    }

    #[test]
    fn a_row_settles_on_the_nearest_tile() {
        assert_eq!(settle(px(-220.), px(200.), px(1000.)), px(-200.));
        assert_eq!(settle(px(-120.), px(200.), px(1000.)), px(-200.));
        assert_eq!(settle(px(-80.), px(200.), px(1000.)), px(0.));
    }

    #[test]
    fn the_last_tile_stays_reachable() {
        // The end is 50px past the last whole step, which rounding would otherwise refuse to show.
        assert_eq!(settle(px(-1050.), px(200.), px(1050.)), px(-1050.));
    }

    #[test]
    fn a_row_never_settles_past_its_own_ends() {
        assert_eq!(settle(px(0.), px(200.), px(1000.)), px(0.));
        assert_eq!(settle(px(-1000.), px(200.), px(1000.)), px(-1000.));
    }
}
