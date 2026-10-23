//! The one tab strip: a row or column of tabs and the bar that slides to the chosen one.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Pixels, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, px, rems,
};

use crate::marker::{self, Along};
use crate::{Axis, Roving, Step, focus, icons, motion, style};

const HOVER: u64 = 0;
const WASH: u64 = 1;
const INK: u64 = 2;

/// `left-2 right-2`: the inset a pill bar keeps from a row tab's own corners.
const INSET: Pixels = px(8.);

/// How the strip draws its tabs and the bar under the chosen one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Bar {
    /// The navigation's rounded tabs, with a rounded bar in the text colour beside or under them.
    #[default]
    Pill,
    /// Flat tabs over a 2px rule in the accent that sits on the strip's own border.
    Rule,
}

impl Bar {
    fn thickness(self, along: Along) -> Pixels {
        match (self, along) {
            (Bar::Pill, Along::Column) => px(3.),
            _ => px(2.),
        }
    }

    fn ink(self, active: bool, hovered: bool, cx: &App) -> Hsla {
        match (self, active, hovered) {
            (_, true, _) => cx.theme().foreground,
            (Bar::Pill, false, true) => style::white(style::TEXT_STRONG, cx),
            (Bar::Pill, false, false) => style::slate_400(cx),
            (Bar::Rule, false, true) => style::white(0.78, cx),
            (Bar::Rule, false, false) => style::white(0.52, cx),
        }
    }

    fn wash(self, active: bool, hovered: bool, cx: &App) -> Hsla {
        style::white(
            match (self, active, hovered) {
                (Bar::Pill, true, _) => style::HOVER,
                (Bar::Pill, false, true) => style::RAISED,
                (Bar::Rule, true, _) => 0.03,
                (Bar::Rule, false, true) => 0.04,
                (_, false, false) => 0.,
            },
            cx,
        )
    }
}

/// One tab: the key the bar finds it by, its glyph, and its label unless the strip is compact.
pub struct Tab {
    key: SharedString,
    icon: &'static str,
    label: Option<SharedString>,
    tooltip: Option<SharedString>,
    ringed: bool,
}

impl Tab {
    pub fn new(key: impl Into<SharedString>, icon: &'static str) -> Self {
        Self {
            key: key.into(),
            icon,
            label: None,
            tooltip: None,
            ringed: false,
        }
    }

    /// The label beside the glyph; without one the tab is the glyph alone.
    pub fn label(mut self, label: Option<SharedString>) -> Self {
        self.label = label;
        self
    }

    pub fn tooltip(mut self, tooltip: Option<SharedString>) -> Self {
        self.tooltip = tooltip;
        self
    }

    /// The keyboard is on this tab.
    pub fn ringed(mut self, ringed: bool) -> Self {
        self.ringed = ringed;
        self
    }
}

type OnSelect = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

/// The tabs and their sliding bar; `roving` makes it one tab stop that the arrows walk.
#[derive(IntoElement)]
pub struct TabStrip {
    id: &'static str,
    along: Along,
    bar: Bar,
    current: Option<SharedString>,
    tabs: Vec<Tab>,
    select: Option<OnSelect>,
    roving: Option<&'static str>,
}

impl TabStrip {
    pub fn new(id: &'static str, along: Along, bar: Bar) -> Self {
        Self {
            id,
            along,
            bar,
            current: None,
            tabs: Vec::new(),
            select: None,
            roving: None,
        }
    }

    /// Which tab is chosen, by key; none hides the bar.
    pub fn current(mut self, key: Option<impl Into<SharedString>>) -> Self {
        self.current = key.map(Into::into);
        self
    }

    pub fn tab(mut self, tab: Tab) -> Self {
        self.tabs.push(tab);
        self
    }

    pub fn tabs(mut self, tabs: impl IntoIterator<Item = Tab>) -> Self {
        self.tabs.extend(tabs);
        self
    }

    /// One tab stop for the whole strip with arrows, home and end inside it, bound under `context`; enter selects.
    pub fn roving(mut self, context: &'static str) -> Self {
        self.roving = Some(context);
        self
    }

    /// What a click, or enter on a roving strip, does for the tab at an index.
    pub fn on_select(mut self, select: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.select = Some(Rc::new(select));
        self
    }

    fn tab_element(
        &self,
        index: usize,
        tab: Tab,
        marks: gpui_kit::Entity<marker::Marks>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let key = SharedString::from(format!("{}-{}", self.id, tab.key));
        let active = self.current.as_ref() == Some(&tab.key);
        let (hovered, pointer) = motion::hovered(motion::channel(&key, HOVER), window, cx);
        let wash = motion::color(
            motion::channel(&key, WASH),
            self.bar.wash(active, hovered, cx),
            window,
            cx,
        );
        let ink = motion::color(
            motion::channel(&key, INK),
            self.bar.ink(active, hovered, cx),
            window,
            cx,
        );
        let vertical = self.along == Along::Column;
        let pad = match tab.label.is_some() {
            true => 3.,
            false => 2.,
        };
        let radius = match self.bar {
            Bar::Pill => style::radius_lg(),
            Bar::Rule => style::radius_md(),
        };
        let thickness = self.bar.thickness(self.along);
        let select = self.select.clone();
        // The tab carries no padding of its own: an absolute probe measures against the content box.
        let frame: Div = div().flex().relative().flex_none().bg(wash).text_color(ink);
        frame
            .id(ElementId::NamedChild(
                Arc::new(self.id.into()),
                tab.key.clone(),
            ))
            .cursor_pointer()
            .map(|frame| match (self.bar, vertical) {
                (Bar::Pill, true) => frame.rounded(radius).w_full().py_2(),
                (Bar::Pill, false) => frame.rounded(radius).h_full().py_1(),
                (Bar::Rule, _) => frame.h_11().items_center(),
            })
            .child(marker::probe(tab.key.clone(), self.along, marks))
            .when_some(tab.tooltip, |frame, tip| {
                frame.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .on_hover(motion::track(pointer))
            .when_some(select, |frame, select| {
                frame.on_click(move |_, window, cx| select(&index, window, cx))
            })
            .child(
                h_flex()
                    .size_full()
                    .gap_2()
                    .items_center()
                    .px(rems(pad * 0.25))
                    .map(|row| match self.bar {
                        Bar::Pill => row.text_sm().font_weight(FontWeight::MEDIUM),
                        Bar::Rule => row.text_xs().font_weight(FontWeight::SEMIBOLD),
                    })
                    .child(
                        icons::icon(tab.icon)
                            .when(self.bar == Bar::Pill, |glyph| glyph.size(px(16.)))
                            .flex_shrink_0(),
                    )
                    .when_some(tab.label, |row, label| {
                        row.child(div().flex_1().min_w_0().truncate().child(label))
                    }),
            )
            .when(vertical && active, |frame| {
                frame.child(
                    div()
                        .absolute()
                        .left(-thickness)
                        .top(px(6.))
                        .h(px(20.))
                        .w(thickness)
                        .rounded_full()
                        .bg(ink),
                )
            })
            .when(tab.ringed, |frame| frame.child(focus::ring(radius, cx)))
            .into_any_element()
    }
}

/// The keyboard half of a roving strip: where the arrows are, and whether a ring should show it.
struct Walk {
    roving: Roving,
    ringing: bool,
}

impl RenderOnce for TabStrip {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(context) = self.roving else {
            return self.draw(window, cx);
        };
        let vertical = self.along == Along::Column;
        let axis = match vertical {
            true => Axis::Column,
            false => Axis::Row,
        };
        let key = SharedString::from(format!("{}-roving", self.id));
        let walk: Entity<Walk> = window.use_keyed_state(key.clone(), cx, |_, cx| Walk {
            roving: Roving::new(axis, cx),
            ringing: false,
        });
        let current = self
            .current
            .as_ref()
            .and_then(|key| self.tabs.iter().position(|tab| &tab.key == key));
        let len = self.tabs.len();
        walk.update(cx, |walk, _| {
            walk.roving.set_axis(axis);
            walk.roving.set_len(len);
            // Off the keyboard the cursor follows the chosen tab; on it, the arrows own the cursor.
            if !walk.roving.is_focused(window)
                && let Some(at) = current
            {
                walk.roving.set_cursor(at);
            }
        });
        let (ringing, cursor) = {
            let walk = walk.read(cx);
            (walk.ringing, walk.roving.cursor())
        };
        let focused = walk.read(cx).roving.is_focused(window);
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            tab.ringed |= focused && ringing && cursor == index;
        }
        let owner = self.select.take();
        self.select = owner.clone().map(|owner| {
            let walk = walk.clone();
            Rc::new(move |index: &usize, window: &mut Window, cx: &mut App| {
                walk.update(cx, |walk, cx| {
                    walk.roving.set_cursor(*index);
                    walk.ringing = false;
                    cx.notify();
                });
                owner(index, window, cx);
            }) as OnSelect
        });
        let group = walk.read(cx).roving.group(div().id(key), context);
        group
            .map(|group| match vertical {
                true => group.w_full(),
                false => group.h_full().flex_none(),
            })
            .on_key_down({
                let walk = walk.clone();
                move |event: &KeyDownEvent, window, cx| {
                    let step = walk.update(cx, |walk, cx| {
                        walk.ringing = true;
                        let step = walk.roving.walk(event);
                        if matches!(step, Step::Moved(_)) {
                            cx.notify();
                        }
                        step
                    });
                    match step {
                        Step::Moved(_) => {}
                        Step::Activate(at) => {
                            if let Some(owner) = &owner {
                                owner(&at, window, cx);
                            }
                        }
                        Step::Ignored => cx.propagate(),
                    }
                }
            })
            .child(self.draw(window, cx))
            .into_any_element()
    }
}

impl TabStrip {
    fn draw(mut self, window: &mut Window, cx: &mut App) -> AnyElement {
        let marks = marker::marks((self.id, 0usize), window, cx);
        let tabs = std::mem::take(&mut self.tabs);
        let tabs: Vec<AnyElement> = tabs
            .into_iter()
            .enumerate()
            .map(|(index, tab)| self.tab_element(index, tab, marks.clone(), window, cx))
            .collect();
        let (start, extent) = match &self.current {
            Some(key) => marker::slide(self.id, &marks, key, window, cx),
            None => (px(0.), px(0.)),
        };
        let thickness = self.bar.thickness(self.along);
        let ink = match self.bar {
            Bar::Pill => cx.theme().foreground,
            Bar::Rule => cx.theme().primary_hover,
        };
        let rounded = self.bar == Bar::Pill;
        let mark = div()
            .flex_none()
            .bg(ink)
            .when(rounded, |mark| mark.rounded_full());
        let gap = |strip: Div| match self.bar {
            Bar::Pill => strip.gap_1(),
            Bar::Rule => strip.gap_0p5(),
        };
        let origin = marker::origin(self.along, marks);
        // The bar rides in the flow beside the tabs rather than over them, so it scrolls with them.
        match self.along {
            Along::Column => h_flex()
                .w_full()
                .items_start()
                .child(v_flex().flex_none().w(thickness))
                .child(gap(v_flex().w_full()).child(origin).children(tabs))
                .into_any_element(),
            Along::Row => {
                let inset = match self.bar {
                    Bar::Pill => INSET,
                    Bar::Rule => px(0.),
                };
                v_flex()
                    .h_full()
                    .flex_none()
                    .items_start()
                    .justify_center()
                    .child(
                        gap(h_flex().h_full().flex_none())
                            .child(origin)
                            .children(tabs),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .h(thickness)
                            // A rule overlaps the strip's border; a pill sits under the tabs.
                            .when(self.bar == Bar::Rule, |lane| lane.mt(-thickness))
                            .child(div().flex_none().w(start + inset))
                            .child(mark.w((extent - inset - inset).max(px(0.))).h_full()),
                    )
                    .into_any_element()
            }
        }
    }
}
