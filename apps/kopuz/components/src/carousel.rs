//! The one sideways shelf: its heading, arrows, fades at its ends, and only the tiles in view built.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    ScrollHandle, ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled, Window,
    div, linear_color_stop, linear_gradient, px,
};

use crate::heading::{Heading, SectionHeading};
use crate::{icons, motion, scroll, style};

/// The carousel's own channels, keyed off its id.
const ROOT: u64 = 0;
const HOVER: u64 = 1;
const ARROWS: u64 = 2;
const LEFT: u64 = 3;
const RIGHT: u64 = 4;
const BACK: u64 = 5;
const FORWARD: u64 = 6;
const HANDLE: u64 = 7;
const ROW: u64 = 8;
const FRAME: u64 = 9;
const GLIDE: u64 = 40;
/// An arrow's hover and size ride on its own channel, offset by these.
const ARROW_HOVER: u64 = 10;
const ARROW_SIZE: u64 = 20;

/// How wide the fade at each end is.
const FADE: f32 = 48.;
/// `hover:scale-105` on an arrow that grows.
const ARROW_GROWTH: f32 = 1.05;

/// When a carousel's arrows show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Arrows {
    /// Under the pointer only, so a page of shelves is not a column of buttons.
    #[default]
    Hover,
    Always,
}

type BuildTile = Rc<dyn Fn(usize, &mut Window, &mut App) -> AnyElement>;

/// A row of equal tiles that slides sideways, building only what is on screen.
#[derive(IntoElement)]
pub struct Carousel {
    id: SharedString,
    heading: SectionHeading,
    action: Option<AnyElement>,
    arrows: Arrows,
    grow: bool,
    handle: Option<ScrollHandle>,
    tile: Pixels,
    gap: Pixels,
    count: usize,
    build: BuildTile,
}

impl Carousel {
    /// `count` tiles, each `tile` wide, built by `build` only while it is in view.
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        count: usize,
        tile: Pixels,
        build: impl Fn(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            heading: SectionHeading::new(title),
            action: None,
            arrows: Arrows::Hover,
            grow: false,
            handle: None,
            tile,
            gap: px(20.),
            count,
            build: Rc::new(build),
        }
    }

    /// The space between tiles; `gap-5` unless a page says otherwise.
    pub fn gap(mut self, gap: Pixels) -> Self {
        self.gap = gap;
        self
    }

    pub fn strapline(mut self, strapline: Option<impl Into<SharedString>>) -> Self {
        self.heading = self.heading.strapline(strapline);
        self
    }

    pub fn heading(mut self, heading: Heading) -> Self {
        self.heading = self.heading.heading(heading);
        self
    }

    /// Something before the arrows, such as a "show all" link.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub fn arrows(mut self, arrows: Arrows) -> Self {
        self.arrows = arrows;
        self
    }

    /// The arrows grow a little under the pointer, as upstream's catalog shelves do.
    pub fn grow_arrows(mut self, grow: bool) -> Self {
        self.grow = grow;
        self
    }

    /// A scroller the page keeps, so the row's place outlives a visit; without one the carousel keys its own.
    pub fn handle(mut self, handle: ScrollHandle) -> Self {
        self.handle = Some(handle);
        self
    }
}

impl RenderOnce for Carousel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let handle = match self.handle {
            Some(handle) => handle,
            None => window
                .use_keyed_state(motion::channel(&id, HANDLE), cx, |_, _| ScrollHandle::new())
                .read(cx)
                .clone(),
        };
        let stride = self.tile + self.gap;
        scroll::smooth_x(motion::channel(&id, GLIDE), &handle, stride, window, cx);
        let (over, pointer) = motion::hovered(motion::channel(&id, HOVER), window, cx);
        let (back, forward) = scroll::room_x(&handle);
        let (first, last) = window_of(&handle, stride, self.count);

        let mut cells: Vec<AnyElement> = Vec::with_capacity(last - first + 2);
        // The tiles out of view stand as one empty box each way, so the row keeps its true width.
        cells.push(spacer(stride * first as f32));
        for index in first..last {
            cells.push(
                div()
                    .flex_shrink_0()
                    .mr(self.gap)
                    .child((self.build)(index, window, cx))
                    .into_any_element(),
            );
        }
        cells.push(spacer(stride * (self.count - last) as f32));

        let shown = match (self.arrows, over) {
            (Arrows::Always, _) | (Arrows::Hover, true) => 1.,
            (Arrows::Hover, false) => 0.,
        };
        let shown = motion::amount(
            motion::channel(&id, ARROWS),
            shown,
            motion::FAST,
            window,
            cx,
        );
        let page = page_step(&handle, stride);
        let arrows = h_flex()
            .gap_2()
            .opacity(shown)
            .child(arrow(
                &id,
                BACK,
                icons::CHEVRON_LEFT,
                &handle,
                -page,
                back,
                self.grow,
                window,
                cx,
            ))
            .child(arrow(
                &id,
                FORWARD,
                icons::CHEVRON_RIGHT,
                &handle,
                page,
                forward,
                self.grow,
                window,
                cx,
            ));
        let compact = self.heading.is_compact();
        let heading = self.heading.trailing(
            h_flex()
                .items_center()
                .gap_4()
                .children(self.action)
                .child(arrows),
        );

        // The whole shelf is the hover, so reaching up for an arrow does not fade it away.
        v_flex()
            .id(motion::channel(&id, ROOT))
            .w_full()
            .flex_shrink_0()
            .map(|shelf| match compact {
                true => shelf.gap_4(),
                false => shelf.gap_6(),
            })
            .on_hover(motion::track(pointer))
            .child(heading)
            .child(
                div()
                    .id(motion::channel(&id, FRAME))
                    .relative()
                    .w_full()
                    .child(
                        h_flex()
                            .id(motion::channel(&id, ROW))
                            .w_full()
                            .pt_2()
                            .pb_6()
                            .items_start()
                            .overflow_x_scroll()
                            // Without this a bare wheel notch would scroll the page and slide the row at once.
                            .restrict_scroll_to_axis()
                            .on_scroll_wheel(wheel(handle.clone()))
                            .track_scroll(&handle)
                            .children(cells),
                    )
                    .child(fade(&id, LEFT, true, back, window, cx))
                    .child(fade(&id, RIGHT, false, forward, window, cx)),
            )
    }
}

/// Which tiles this row can actually show; the rest are two empty boxes it never builds.
fn window_of(handle: &ScrollHandle, stride: Pixels, count: usize) -> (usize, usize) {
    let width = handle.bounds().size.width;
    // Before the first layout there is no viewport to measure, so nothing is left out.
    if width <= px(0.) || stride <= px(0.) {
        return (0, count);
    }
    let stride = stride.as_f32();
    let first = ((-handle.offset().x.as_f32() / stride).floor().max(0.) as usize).min(count);
    let reach = (width.as_f32() / stride).ceil() as usize + 2;
    (first, (first + reach).min(count))
}

/// Shift and the wheel drives the row sideways, since a bare notch belongs to the page.
fn wheel(handle: ScrollHandle) -> impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static {
    move |event, window, cx| {
        let delta = event.delta.pixel_delta(window.line_height());
        if !event.modifiers.shift || delta.y == px(0.) {
            return;
        }
        scroll::nudge_x(&handle, -delta.y);
        cx.stop_propagation();
    }
}

/// One arrow's nudge: a viewport's worth, so a click lands the next screenful of tiles.
fn page_step(handle: &ScrollHandle, stride: Pixels) -> Pixels {
    let width = handle.bounds().size.width;
    match width > stride {
        true => px((width.as_f32() / stride.as_f32()).floor() * stride.as_f32()),
        false => stride,
    }
}

fn spacer(width: Pixels) -> AnyElement {
    div().w(width).flex_shrink_0().into_any_element()
}

/// `w-9 h-9 rounded-full bg-white/5 hover:bg-white/10`: one carousel arrow, dimmed at its end.
#[allow(clippy::too_many_arguments)]
fn arrow(
    id: &SharedString,
    channel: u64,
    icon: &'static str,
    handle: &ScrollHandle,
    by: Pixels,
    enabled: bool,
    grow: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (over, pointer) = motion::hovered(motion::channel(id, channel + ARROW_HOVER), window, cx);
    let background = motion::color(
        motion::channel(id, channel),
        style::white(
            match (enabled, over) {
                (false, _) => style::BORDER_SOFT,
                (true, true) => style::HOVER,
                (true, false) => style::RAISED,
            },
            cx,
        ),
        window,
        cx,
    );
    let rest = style::to_px(style::icon_button(), window);
    let size = match grow {
        true => motion::length(
            motion::channel(id, channel + ARROW_SIZE),
            match enabled && over {
                true => rest * ARROW_GROWTH,
                false => rest,
            },
            motion::FAST,
            window,
            cx,
        ),
        false => rest,
    };
    let handle = handle.clone();
    div()
        .id(motion::channel(id, channel + ARROW_HOVER + ARROW_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .size(size)
        .rounded_full()
        .bg(background)
        .text_color(style::white(
            match enabled {
                true => style::TEXT_STRONG,
                false => style::TEXT_DIM,
            },
            cx,
        ))
        .when(enabled, |arrow| arrow.cursor_pointer())
        .on_hover(motion::track(pointer))
        .child(Icon::default().path(icon).size(px(14.)))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            if enabled {
                scroll::nudge_x(&handle, by);
            }
        })
        .into_any_element()
}

/// The page colour bled over the tiles at an end that has more behind it.
fn fade(
    id: &SharedString,
    channel: u64,
    left: bool,
    shown: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let amount = motion::amount(
        motion::channel(id, channel),
        match shown {
            true => 1.,
            false => 0.,
        },
        motion::FAST,
        window,
        cx,
    );
    let background = cx.theme().background;
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(FADE))
        .map(|edge| match left {
            true => edge.left_0(),
            false => edge.right_0(),
        })
        .opacity(amount)
        .bg(linear_gradient(
            match left {
                true => 90.,
                false => 270.,
            },
            linear_color_stop(background, 0.),
            linear_color_stop(background.opacity(0.), 1.),
        ))
        .into_any_element()
}
