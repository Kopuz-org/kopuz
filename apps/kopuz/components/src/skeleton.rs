//! What sits where the rows will be. A shape of the right size reads as "nearly there"; a
//! centred spinner reads as "nothing yet", and blanks a page that already had content on it.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, Hsla, IntoElement, ParentElement as _, Pixels, Styled as _, div, px};

use crate::style;

/// The wash a placeholder shape is painted in; the theme names it for exactly this.
fn ink(cx: &App) -> Hsla {
    cx.theme().skeleton
}

/// A bar of the given width, for a cell that will hold text.
fn bar(width: Pixels, height: Pixels, cx: &App) -> impl IntoElement {
    div()
        .w(width)
        .h(height)
        .rounded(style::radius(cx))
        .bg(ink(cx))
}

/// One placeholder row, laid out like the table row it stands in for.
pub fn row(
    slot: usize,
    height: Pixels,
    covers: bool,
    columns: usize,
    cx: &App,
) -> impl IntoElement {
    // A fixed ripple rather than an animation: a tween here re-renders the whole window per frame.
    let widths = [160., 120., 140., 100., 180.];
    let width = |column: usize| px(widths[(slot + column) % widths.len()]);
    h_flex()
        .w_full()
        .h(height)
        .items_center()
        .gap_6()
        .px_2()
        .when(covers, |row| {
            row.child(
                div()
                    .size(style::row_cover())
                    .flex_shrink_0()
                    .rounded(style::thumb_radius())
                    .bg(ink(cx)),
            )
        })
        .children((0..columns.max(1)).map(|column| bar(width(column), px(10.), cx)))
}

/// A screenful of placeholder rows, for a table that has asked but not been answered.
pub fn rows(
    count: usize,
    height: Pixels,
    covers: bool,
    columns: usize,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_1()
        .children((0..count).map(|slot| row(slot, height, covers, columns, cx)))
}

/// One placeholder tile, the size the real tiles will be.
pub fn tile(width: Pixels, round: bool, cx: &App) -> impl IntoElement {
    let cover = width - px(32.);
    v_flex()
        .w(width)
        .p_4()
        .child(div().size(cover).bg(ink(cx)).map(|shape| match round {
            true => shape.rounded(cover / 2.),
            false => shape.rounded(style::radius_xl()),
        }))
        .child(catalog_text(cover, cx))
}

fn catalog_text(width: Pixels, cx: &App) -> impl IntoElement {
    v_flex()
        .mt_3()
        .w_full()
        .child(
            div().relative().h_6().child(
                div().absolute().inset_0().flex().items_center().child(bar(
                    width * 0.7,
                    px(10.),
                    cx,
                )),
            ),
        )
        .child(
            div().relative().h_5().child(
                div().absolute().inset_0().flex().items_center().child(bar(
                    width * 0.45,
                    px(8.),
                    cx,
                )),
            ),
        )
}

pub fn catalog_row(covers: bool, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_3()
        .items_center()
        .px_3()
        .py_2()
        .mb_1()
        .when(covers, |row| row.child(div().size(px(48.)).bg(ink(cx))))
        .child(
            v_flex()
                .child(div().relative().w_40().h_6().child(
                    div().absolute().inset_0().flex().items_center().child(bar(
                        px(120.),
                        px(10.),
                        cx,
                    )),
                ))
                .child(div().relative().w_40().h_5().child(
                    div().absolute().inset_0().flex().items_center().child(bar(
                        px(80.),
                        px(8.),
                        cx,
                    )),
                )),
        )
}

/// A band of placeholder tiles, matching the grid the real ones will fill.
pub fn band(count: usize, width: Pixels, round: bool, cx: &App) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_start()
        .gap_6()
        .pb_6()
        .children((0..count.max(1)).map(|_| tile(width, round, cx)))
}

/// Content fading in `shown` of the way over the placeholder it replaces, which fades out beneath it.
pub fn reveal(
    content: impl IntoElement,
    placeholder: Option<gpui_kit::AnyElement>,
    shown: f32,
) -> gpui_kit::AnyElement {
    div()
        .w_full()
        .relative()
        .when_some(placeholder.filter(|_| shown < 1.), |frame, placeholder| {
            frame.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .opacity(1. - shown)
                    .child(placeholder),
            )
        })
        .child(div().w_full().opacity(shown).child(content))
        .into_any_element()
}

/// A placeholder laid out as an unframed `Tile` is, its lines holding real blank text, so the tile that replaces it is no taller.
pub fn shelf_tile(width: Pixels, cx: &App) -> impl IntoElement {
    let cover = width - px(32.);
    let line = |text_sm: bool, bar_width: Pixels, bar_height: Pixels| {
        div()
            .relative()
            .w_full()
            .when(text_sm, |line| line.text_sm())
            .text_color(gpui_kit::transparent_black())
            .child("\u{a0}")
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .child(bar(bar_width, bar_height, cx)),
            )
    };
    v_flex()
        .w(width)
        .gap_4()
        .child(
            div()
                .size(width)
                .p_4()
                .child(div().size(cover).bg(ink(cx)).rounded(style::radius_xl())),
        )
        .child(
            v_flex()
                .w_full()
                .px_1()
                .gap_1()
                .child(line(false, cover * 0.7, px(10.)))
                .child(line(true, cover * 0.45, px(8.))),
        )
}
