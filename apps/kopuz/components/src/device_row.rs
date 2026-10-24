//! A row in a list of places to play: a glyph tile, a name and what it is, lit while it is the one chosen.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::{icons, motion, style};

const WASH: u64 = 1;
const TILE: u64 = 2;
const MARK: u64 = 3;
const LABEL: u64 = 4;
const CAPTION: u64 = 5;

// `w-10 h-10 rounded-md` with a `text-sm` glyph: the tile in front of a device's name.
const ROW_TILE: Pixels = px(40.);
const ROW_ICON: Pixels = px(14.);

type Select = Rc<dyn Fn(&mut Window, &mut App)>;

/// One place audio can go; its colours tween between resting and chosen.
#[derive(IntoElement)]
pub struct DeviceRow {
    id: SharedString,
    name: SharedString,
    glyph: &'static str,
    subtitle: Option<SharedString>,
    chosen: bool,
    select: Option<Select>,
}

impl DeviceRow {
    pub fn new(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            glyph: icons::SPEAKER,
            subtitle: None,
            chosen: false,
            select: None,
        }
    }

    /// The glyph in the tile before the name.
    pub fn glyph(mut self, glyph: &'static str) -> Self {
        self.glyph = glyph;
        self
    }

    /// What kind of device it is, under the name.
    pub fn subtitle(mut self, subtitle: Option<SharedString>) -> Self {
        self.subtitle = subtitle;
        self
    }

    /// The device audio is going to now.
    pub fn chosen(mut self, chosen: bool) -> Self {
        self.chosen = chosen;
        self
    }

    pub fn on_select(mut self, select: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.select = Some(Rc::new(select));
        self
    }
}

impl RenderOnce for DeviceRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chosen = self.chosen;
        let accent = cx.theme().primary;
        let raised = style::white(style::RAISED, cx);
        let pick = |on: gpui_kit::Hsla, off: gpui_kit::Hsla| if chosen { on } else { off };
        let wash_target = pick(style::accent(style::PLAYING, cx), cx.theme().transparent);
        let tile_target = pick(style::accent(0.18, cx), style::white(0.06, cx));
        let mark_target = pick(accent, style::white(style::TEXT_SECONDARY, cx));
        let name_target = pick(accent, cx.theme().foreground);
        let subtitle_target = pick(accent.opacity(0.7), style::white(0.5, cx));
        let tween = |channel, target, window: &mut Window, cx: &mut App| {
            motion::color(motion::channel(&self.id, channel), target, window, cx)
        };
        let wash = tween(WASH, wash_target, window, cx);
        let tile = tween(TILE, tile_target, window, cx);
        let mark = tween(MARK, mark_target, window, cx);
        let label = tween(LABEL, name_target, window, cx);
        let caption = tween(CAPTION, subtitle_target, window, cx);
        let select = self.select;

        h_flex()
            .id(self.id.clone())
            .w_full()
            .gap_3()
            .px_3()
            .py_2p5()
            .rounded(style::radius_lg())
            .cursor_pointer()
            .bg(wash)
            .hover(move |row| row.bg(raised))
            .on_click(move |_, window, cx| {
                if let Some(select) = &select {
                    select(window, cx);
                }
            })
            .child(
                div()
                    .size(ROW_TILE)
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(style::thumb_radius())
                    .bg(tile)
                    .child(icons::icon(self.glyph).size(ROW_ICON).text_color(mark)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .justify_center()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(label)
                            .child(self.name),
                    )
                    .when_some(self.subtitle, |column, subtitle| {
                        column.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(caption)
                                .child(subtitle),
                        )
                    }),
            )
            .when(chosen, |row| {
                row.child(icons::icon(icons::VOLUME).size(ROW_ICON).text_color(accent))
            })
    }
}
