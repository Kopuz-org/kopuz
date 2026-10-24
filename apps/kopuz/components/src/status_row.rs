//! The one-line rows of a popover or a palette: a glyph, what it says, and what it adds.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled, Window, div,
};

use crate::{Keycap, focus, icons, style};

/// What a status row's glyph is drawn at.
const GLYPH: Pixels = gpui_kit::px(13.);

/// A glyph, a title, and optionally a detail line, an aside beside the title, and something at the end.
#[derive(IntoElement)]
pub struct StatusRow {
    id: SharedString,
    glyph: Option<(&'static str, Hsla)>,
    title: Option<SharedString>,
    title_ink: Option<Hsla>,
    aside: Option<SharedString>,
    detail: Option<(SharedString, Option<Hsla>)>,
    trailing: Option<AnyElement>,
    height: Option<Pixels>,
    wrap: bool,
}

impl StatusRow {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            glyph: None,
            title: None,
            title_ink: None,
            aside: None,
            detail: None,
            trailing: None,
            height: None,
            wrap: false,
        }
    }

    pub fn glyph(mut self, icon: &'static str, ink: Hsla) -> Self {
        self.glyph = Some((icon, ink));
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The title's colour; the foreground by default.
    pub fn title_ink(mut self, ink: Hsla) -> Self {
        self.title_ink = Some(ink);
        self
    }

    /// A quiet word at the end of the title's line, such as a format.
    pub fn aside(mut self, aside: impl Into<SharedString>) -> Self {
        self.aside = Some(aside.into());
        self
    }

    /// A second line under the title; `ink` is its colour, muted by default.
    pub fn detail(mut self, detail: impl Into<SharedString>, ink: Option<Hsla>) -> Self {
        self.detail = Some((detail.into(), ink));
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    /// A fixed row height, for rows that sit in a virtualized list.
    pub fn height(mut self, height: Pixels) -> Self {
        self.height = Some(height);
        self
    }

    /// Let the title run over several lines, with the glyph kept at the top.
    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }
}

impl RenderOnce for StatusRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let title_ink = self.title_ink.unwrap_or(cx.theme().foreground);
        let wrap = self.wrap;
        h_flex()
            .id(self.id)
            .w_full()
            .gap_2()
            .items_center()
            .px_3()
            .py_1p5()
            .when(wrap, |row| row.items_start())
            .when_some(self.height, |row, height| row.h(height))
            .children(self.glyph.map(|(icon, ink)| {
                icons::icon(icon)
                    .size(GLYPH)
                    .flex_shrink_0()
                    .text_color(ink)
            }))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(title_ink)
                                    .when(!wrap, |title| title.truncate())
                                    .children(self.title),
                            )
                            .children(self.aside.map(|aside| {
                                div()
                                    .flex_shrink_0()
                                    .text_size(style::text_10())
                                    .text_color(style::slate_500(cx))
                                    .child(aside)
                            })),
                    )
                    .children(self.detail.map(|(detail, ink)| {
                        div()
                            .truncate()
                            .text_size(style::text_10())
                            .text_color(ink.unwrap_or(style::white(style::TEXT_MUTED, cx)))
                            .child(detail)
                    })),
            )
            .children(self.trailing)
    }
}

/// The accent wash on the row the keyboard is on.
const CHOSEN: f32 = 0.16;
/// Results that answer the previous query stay up at this opacity rather than blanking.
const FADED: f32 = 0.55;
/// The widest a subtitle may get before the title stops shrinking for it.
const SUBTITLE: Pixels = gpui_kit::px(220.);

type OnHover = Rc<dyn Fn(&bool, &mut Window, &mut App)>;
type OnClick = Rc<dyn Fn(&gpui_kit::ClickEvent, &mut Window, &mut App)>;

/// One row of the command palette: a glyph, a title and subtitle, and the key that does it directly.
#[derive(IntoElement)]
pub struct CommandRow {
    id: ElementId,
    height: Pixels,
    icon: &'static str,
    title: SharedString,
    subtitle: Option<SharedString>,
    hint: Option<SharedString>,
    chosen: bool,
    faded: bool,
    on_hover: Option<OnHover>,
    on_click: Option<OnClick>,
}

impl CommandRow {
    pub fn new(
        id: impl Into<ElementId>,
        icon: &'static str,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            height: gpui_kit::px(40.),
            icon,
            title: title.into(),
            subtitle: None,
            hint: None,
            chosen: false,
            faded: false,
            on_hover: None,
            on_click: None,
        }
    }

    /// Every row of a virtualized list is the same height.
    pub fn height(mut self, height: Pixels) -> Self {
        self.height = height;
        self
    }

    pub fn subtitle(mut self, subtitle: Option<SharedString>) -> Self {
        self.subtitle = subtitle;
        self
    }

    /// The keystroke that runs it without the palette, as the keymap writes it.
    pub fn hint(mut self, hint: Option<SharedString>) -> Self {
        self.hint = hint;
        self
    }

    /// The keyboard is on this row.
    pub fn chosen(mut self, chosen: bool) -> Self {
        self.chosen = chosen;
        self
    }

    /// The row answers an older query than the one typed.
    pub fn faded(mut self, faded: bool) -> Self {
        self.faded = faded;
        self
    }

    pub fn on_hover(mut self, hover: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Rc::new(hover));
        self
    }

    pub fn on_click(
        mut self,
        click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(click));
        self
    }
}

impl RenderOnce for CommandRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let chosen = self.chosen;
        let glyph = match chosen {
            true => cx.theme().primary_hover,
            false => style::white(style::TEXT_MUTED, cx),
        };
        let ink = match chosen {
            true => cx.theme().foreground,
            false => style::white(style::TEXT_STRONG, cx),
        };
        let (hover, click) = (self.on_hover, self.on_click);
        div()
            .id(self.id)
            .h(self.height)
            .w_full()
            .px_2()
            .cursor_pointer()
            .tab_index(0)
            .when(self.faded, |row| row.opacity(FADED))
            .child(
                h_flex()
                    .relative()
                    .size_full()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .rounded(style::radius_lg())
                    .when(chosen, |row| {
                        row.bg(style::accent(CHOSEN, cx))
                            .child(focus::ring(style::radius_lg(), cx))
                    })
                    .child(
                        icons::icon(self.icon)
                            .with_size(style::text_11())
                            .text_color(glyph),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .items_baseline()
                            .child(
                                div()
                                    .flex_shrink(1.)
                                    .min_w_0()
                                    .text_sm()
                                    .truncate()
                                    .text_color(ink)
                                    .child(self.title),
                            )
                            .children(self.subtitle.map(|subtitle| {
                                div()
                                    .flex_shrink(1.)
                                    .min_w_0()
                                    .max_w(SUBTITLE)
                                    .text_xs()
                                    .truncate()
                                    .text_color(style::slate_500(cx))
                                    .child(subtitle)
                            })),
                    )
                    .children(self.hint.as_deref().map(Keycap::new)),
            )
            .on_hover(move |over, window, cx| {
                if let Some(hover) = &hover {
                    hover(over, window, cx);
                }
            })
            .on_click(move |event, window, cx| {
                if let Some(click) = &click {
                    click(event, window, cx);
                }
            })
    }
}
