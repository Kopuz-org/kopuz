//! The one segmented toggle: a raised track holding a few choices, the picked one washed.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::{focus, icons, motion, skin, style};

const ROOT: u64 = 0;
const HOVER: u64 = 1;
const WASH: u64 = 2;
const INK: u64 = 3;

/// `text-white/40`: an unpicked segment at rest.
const IDLE: f32 = 0.40;
/// `hover:text-white/80` on that same segment.
const HOT: f32 = 0.80;

/// Material's segmented button: 40dp, full shape, outlined.
const M3_HEIGHT: Pixels = px(40.);
/// Material's outline: the text colour at 45% over the surface.
const M3_OUTLINE: f32 = 0.45;
/// Material's secondary container, as the accent's tonal wash.
const M3_CONTAINER: f32 = 0.24;

/// One choice: a glyph, a label, or both; a glyph alone should carry a tooltip.
pub struct Segment {
    key: SharedString,
    icon: Option<&'static str>,
    label: Option<SharedString>,
    tooltip: Option<SharedString>,
}

impl Segment {
    /// `key` keys its hover and focus, so it must be stable and unique within the toggle.
    pub fn new(key: impl Into<SharedString>) -> Self {
        Self {
            key: key.into(),
            icon: None,
            label: None,
            tooltip: None,
        }
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }
}

type OnSelect = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

/// A few mutually exclusive choices side by side: a view mode, a facet, a panel tab.
#[derive(IntoElement)]
pub struct Segmented {
    id: SharedString,
    selected: Option<usize>,
    segments: Vec<Segment>,
    pill: bool,
    select: Option<OnSelect>,
}

impl Segmented {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            selected: None,
            segments: Vec::new(),
            pill: false,
            select: None,
        }
    }

    pub fn segment(mut self, segment: Segment) -> Self {
        self.segments.push(segment);
        self
    }

    pub fn segments(mut self, segments: impl IntoIterator<Item = Segment>) -> Self {
        self.segments.extend(segments);
        self
    }

    /// Which segment is picked, by position.
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index;
        self
    }

    /// A fully rounded track, as a header's toggles wear.
    pub fn pill(mut self, pill: bool) -> Self {
        self.pill = pill;
        self
    }

    /// What picking the segment at an index does.
    pub fn on_select(mut self, select: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.select = Some(Rc::new(select));
        self
    }

    fn segment_element(
        &self,
        index: usize,
        count: usize,
        segment: Segment,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let material = skin::material();
        let key = SharedString::from(format!("{}-{}", self.id, segment.key));
        let active = self.selected == Some(index);
        let (hovered, pointer) = motion::hovered(motion::channel(&key, HOVER), window, cx);
        let focus = focus::handle(format!("segment-focus:{key}"), window, cx);
        let ringed = focus.is_focused(window) && focus::keyboard(cx);
        let lit = hovered || ringed;
        let wash = motion::color(
            motion::channel(&key, WASH),
            match (material, active, lit) {
                // Material's picked segment is the secondary container.
                (true, true, _) => cx.theme().primary.opacity(M3_CONTAINER),
                (_, true, _) => style::white(style::HOVER, cx),
                (_, false, true) => style::white(style::RAISED, cx),
                (_, false, false) => style::white(0., cx),
            },
            window,
            cx,
        );
        let ink = motion::color(
            motion::channel(&key, INK),
            match (active, lit) {
                (true, _) => cx.theme().foreground,
                (false, true) => style::white(HOT, cx),
                (false, false) => style::white(IDLE, cx),
            },
            window,
            cx,
        );
        let height = match material {
            true => M3_HEIGHT,
            false => style::chrome().tab_height,
        };
        let (first, last) = (index == 0, index + 1 == count);
        let checked = material && active && segment.label.is_some();
        let glyph_only = segment.label.is_none();
        let activate = self.select.clone();
        let click = self.select.clone();
        focus::stop(
            div().id(motion::channel(&key, ROOT)),
            &focus,
            move |window, cx| {
                if let Some(select) = activate.as_ref() {
                    select(&index, window, cx);
                }
            },
        )
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap_1p5()
        .h(height)
        .map(|frame| match glyph_only {
            true => frame.w(height),
            false => frame.px_3(),
        })
        .map(|frame| match material {
            // The outline is the container's; a segment rounds only the ends it owns.
            true => frame
                .when(first, |frame| frame.rounded_l(M3_HEIGHT / 2.))
                .when(last, |frame| frame.rounded_r(M3_HEIGHT / 2.))
                .when(!first, |frame| {
                    frame
                        .border_l_1()
                        .border_color(style::white(M3_OUTLINE, cx))
                })
                .text_sm(),
            false => frame.rounded(style::thumb_radius()).text_xs(),
        })
        .bg(wash)
        .text_color(ink)
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .on_hover(motion::track(pointer))
        .when_some(segment.tooltip, |frame, tip| {
            frame.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .when_some(click, |frame, select| {
            frame.on_click(move |_, window, cx| select(&index, window, cx))
        })
        .when(checked, |frame| {
            frame.child(icons::icon(icons::CHECK).size(px(14.)).flex_shrink_0())
        })
        .when_some(segment.icon, |frame, icon| {
            frame.child(icons::icon(icon).size(px(14.)).flex_shrink_0())
        })
        .when_some(segment.label, |frame, label| frame.child(label))
        .when(ringed, |frame| {
            frame.child(focus::ring(style::radius_md(), cx))
        })
        .into_any_element()
    }
}

/// Grid ⇄ list, the toggle every page that can draw either wears; `set` is told whether grid was picked.
pub fn view_toggle(
    id: impl Into<SharedString>,
    grid: bool,
    set: impl Fn(bool, &mut App) + 'static,
) -> Segmented {
    Segmented::new(id)
        .segment(
            Segment::new("grid")
                .icon(icons::GRID)
                .tooltip(crate::format::t("view_grid")),
        )
        .segment(
            Segment::new("list")
                .icon(icons::ROWS)
                .tooltip(crate::format::t("view_list")),
        )
        .selected(Some(match grid {
            true => 0,
            false => 1,
        }))
        .on_select(move |index, _, cx| set(*index == 0, cx))
}

impl RenderOnce for Segmented {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let segments = std::mem::take(&mut self.segments);
        let count = segments.len();
        let children: Vec<AnyElement> = segments
            .into_iter()
            .enumerate()
            .map(|(index, segment)| self.segment_element(index, count, segment, window, cx))
            .collect();
        if skin::material() {
            return div()
                .flex()
                .flex_none()
                .items_center()
                .h(M3_HEIGHT)
                .rounded(M3_HEIGHT / 2.)
                .border_1()
                .border_color(style::white(M3_OUTLINE, cx))
                .children(children)
                .into_any_element();
        }
        let radius = match self.pill {
            true => cx.theme().radius_full(),
            false => style::to_px(style::radius_lg(), window),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .p_1()
            .rounded(radius)
            .bg(style::white(style::RAISED, cx))
            .border_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .children(children)
            .into_any_element()
    }
}
