//! The theme picker as a grid of small windows, each drawn in its own theme's colours.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled, Window, div, px, relative,
    rems,
};

use crate::color_field::parse_color;
use crate::settings::Lockable;
use crate::{icons, style};

/// How many examples a row holds unless the caller says otherwise.
const COLUMNS: u16 = 4;
/// The example window's height.
const WINDOW: f32 = 5.;
/// A gallery a managed settings file pins is drawn faded.
const LOCKED_OPACITY: f32 = 0.5;

/// One theme the gallery offers: the value written when it is picked, its name, its colours.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeOption {
    pub id: String,
    pub label: SharedString,
    pub colors: theme::Colors,
}

type Write = Rc<dyn Fn(String, &mut App)>;

#[derive(IntoElement)]
pub struct ThemeGallery {
    id: ElementId,
    current: String,
    options: Vec<ThemeOption>,
    columns: u16,
    locked: bool,
    write: Option<Write>,
}

impl ThemeGallery {
    pub fn new(id: impl Into<ElementId>, current: String, options: Vec<ThemeOption>) -> Self {
        Self {
            id: id.into(),
            current,
            options,
            columns: COLUMNS,
            locked: false,
            write: None,
        }
    }

    /// Examples per row: fewer in a narrow column, so each stays big enough to read.
    pub fn columns(mut self, columns: u16) -> Self {
        self.columns = columns.max(1);
        self
    }

    pub fn on_change(mut self, write: impl Fn(String, &mut App) + 'static) -> Self {
        self.write = Some(Rc::new(write));
        self
    }
}

impl Lockable for ThemeGallery {
    fn lock(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }
}

fn color(value: &str, fallback: Hsla) -> Hsla {
    parse_color(value).unwrap_or(fallback)
}

/// A sidebar, a title, a line of detail and a playing bar, so the theme shows as the app would.
fn example(colors: &theme::Colors, cx: &App) -> impl IntoElement {
    let fallback = cx.theme().foreground;
    let (bg, raised, surface) = (
        color(&colors.bg, cx.theme().background),
        color(&colors.raised, fallback),
        color(&colors.surface, fallback),
    );
    let (text, muted, accent) = (
        color(&colors.text, fallback),
        color(&colors.text_muted, fallback),
        color(&colors.accent, cx.theme().primary),
    );
    let bar = |width: f32, height: f32, fill: Hsla| {
        div()
            .w(relative(width))
            .h(px(height))
            .rounded_full()
            .bg(fill)
    };
    h_flex()
        .size_full()
        .bg(bg)
        .child(
            v_flex()
                .w(relative(0.3))
                .h_full()
                .gap_1()
                .p_1p5()
                .bg(raised)
                .child(bar(0.8, 3., muted))
                .child(bar(0.6, 3., muted))
                .child(bar(0.7, 3., accent)),
        )
        .child(
            v_flex()
                .flex_1()
                .h_full()
                .gap_1()
                .p_2()
                .child(bar(0.75, 5., text))
                .child(bar(0.45, 3., muted))
                .child(div().flex_1())
                .child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .flex_shrink_0()
                                .size(px(10.))
                                .rounded_full()
                                .bg(accent),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h(px(3.))
                                .rounded_full()
                                .bg(surface)
                                .child(bar(0.6, 3., accent)),
                        ),
                ),
        )
}

impl RenderOnce for ThemeGallery {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (locked, write) = (self.locked, self.write);
        div()
            .id(self.id)
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .when(locked, |gallery| gallery.opacity(LOCKED_OPACITY))
            // Rows of equal cells rather than a grid: a grid track here sizes to its content, not the width.
            .children(self.options.chunks(self.columns as usize).map(|row| {
                let blanks = self.columns as usize - row.len();
                h_flex()
                    .w_full()
                    .gap_3()
                    .children(row.iter().map(|option| {
                        card(option, &self.current, locked, write.clone(), cx).into_any_element()
                    }))
                    .children((0..blanks).map(|_| div().flex_1().min_w_0().into_any_element()))
            }))
    }
}

/// One example: the theme's window, ringed when it is the one in use, and its name under it.
fn card(
    option: &ThemeOption,
    current: &str,
    locked: bool,
    write: Option<Write>,
    cx: &App,
) -> impl IntoElement {
    let picked = option.id == current;
    let ring = match picked {
        true => cx.theme().primary,
        false => style::white(style::BORDER, cx),
    };
    let hover = style::white(style::TEXT_DIM, cx);
    let value = option.id.clone();
    v_flex()
        .id(SharedString::from(format!("theme-{}", option.id)))
        .flex_1()
        .min_w_0()
        .gap_1p5()
        .when(!locked, |card| {
            card.cursor_pointer().on_click(move |_, _, cx| {
                if let Some(write) = &write {
                    write(value.clone(), cx);
                }
            })
        })
        .child(
            div()
                .id(SharedString::from(format!("theme-window-{}", option.id)))
                .w_full()
                .h(rems(WINDOW))
                .overflow_hidden()
                .rounded(style::radius_lg())
                .border_2()
                .border_color(ring)
                .when(!picked && !locked, |window| {
                    window.hover(move |style| style.border_color(hover))
                })
                .child(example(&option.colors, cx)),
        )
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .when(picked, |label| label.font_weight(FontWeight::MEDIUM))
                        .text_color(match picked {
                            true => cx.theme().foreground,
                            false => style::white(style::TEXT_SECONDARY, cx),
                        })
                        .child(option.label.clone()),
                )
                .when(picked, |row| {
                    row.child(
                        icons::icon(icons::CHECK)
                            .size(px(12.))
                            .flex_shrink_0()
                            .text_color(cx.theme().primary),
                    )
                }),
        )
}
