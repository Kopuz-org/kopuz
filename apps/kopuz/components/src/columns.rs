//! The track table's columns, and the flex frame a row and its header share since GPUI has no grid.

use std::rc::Rc;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{App, Div, Pixels, Styled as _, div, px};
use store::layout::Column;

use crate::format::t;

/// The index column: the number, the play glyph, or the selection box.
pub const INDEX: Pixels = px(20.);
/// The row-menu column, which the header keeps clear for the rows' dots.
pub const MENU: Pixels = px(40.);

/// One column the table can draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub id: &'static str,
    pub label_key: &'static str,
    /// A column with no natural width shares the leftover space this many ways.
    pub grow: f32,
    /// What a flexible column never goes below, or what a fixed one is.
    pub base: f32,
    pub align_end: bool,
    /// The title column carries the cover and can never be hidden; without it there is no row.
    pub required: bool,
}

/// Every column, in the order the table ships with.
pub const SPECS: &[Spec] = &[
    Spec {
        id: "title",
        label_key: "title",
        grow: 2.,
        base: 200.,
        align_end: false,
        required: true,
    },
    Spec {
        id: "artist",
        label_key: "artist",
        grow: 1.,
        base: 150.,
        align_end: false,
        required: false,
    },
    Spec {
        id: "album",
        label_key: "album",
        grow: 1.,
        base: 150.,
        align_end: false,
        required: false,
    },
    Spec {
        id: "plays",
        label_key: "library_column_plays",
        grow: 0.,
        base: 64.,
        align_end: true,
        required: false,
    },
    Spec {
        id: "format",
        label_key: "library_column_format",
        grow: 0.,
        base: 72.,
        align_end: false,
        required: false,
    },
    Spec {
        id: "duration",
        label_key: "duration",
        grow: 0.,
        base: 64.,
        align_end: true,
        required: false,
    },
];

pub fn spec(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.id == id)
}

/// A column as drawn: what it is, and the width it was dragged to if it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    pub spec: &'static Spec,
    pub width: Option<f32>,
}

/// The columns a row and its header draw, built once a frame and shared by every row.
pub type Layout = Rc<[Shown]>;

/// The shown columns of a saved layout, in draw order.
pub fn visible(columns: &[Column]) -> Layout {
    columns
        .iter()
        .filter(|column| column.visible)
        .filter_map(|column| {
            spec(&column.id).map(|spec| Shown {
                spec,
                width: column.width,
            })
        })
        .collect()
}

/// The fixed table a page list draws: title, artist, the album unless the page is one, duration.
pub fn standard(album: bool) -> Layout {
    SPECS
        .iter()
        .filter(|spec| match spec.id {
            "title" | "artist" | "duration" => true,
            "album" => album,
            _ => false,
        })
        .map(|spec| Shown { spec, width: None })
        .collect()
}

/// The frame a row and a header share: `items-center p-2` with the `1.5rem` column gap.
pub fn frame() -> Div {
    div().flex().w_full().items_center().p_2().gap_6()
}

/// A fixed column: the index, the menu, or one whose width is set.
pub fn fixed(width: Pixels) -> Div {
    div().flex().items_center().w(width).flex_shrink_0()
}

/// A `minmax(min, Nfr)` column, sharing the free space `grow` ways.
pub fn flexible(min: Pixels, grow: f32) -> Div {
    div()
        .flex()
        .items_center()
        .flex_grow(grow)
        .flex_shrink(1.)
        .flex_basis(px(0.))
        .min_w(min)
}

/// A column's box: the width it was dragged to, or the one its kind ships with.
pub fn sized(shown: &Shown) -> Div {
    match (shown.width, shown.spec.grow > 0.) {
        (Some(width), _) => fixed(px(width)),
        (None, true) => flexible(px(shown.spec.base), shown.spec.grow),
        (None, false) => fixed(px(shown.spec.base)),
    }
}

/// What ticking a column in the hide menu does.
pub type OnToggle = Rc<dyn Fn(&str, &mut App)>;

/// The show-and-hide menu: a tick per column that can be hidden, and what a click toggles.
pub fn hide_menu(held: &[Column], toggle: OnToggle, mut menu: PopupMenu) -> PopupMenu {
    for column in held {
        let Some(spec) = spec(&column.id) else {
            continue;
        };
        if spec.required {
            continue;
        }
        let toggle = toggle.clone();
        menu = menu.item(
            PopupMenuItem::new(t(spec.label_key))
                .checked(column.visible)
                .on_click(move |_, _, cx| toggle(spec.id, cx)),
        );
    }
    menu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_list_draws_the_album_column_only_when_asked() {
        let ids = |layout: Layout| layout.iter().map(|shown| shown.spec.id).collect::<Vec<_>>();
        assert_eq!(
            ids(standard(true)),
            ["title", "artist", "album", "duration"]
        );
        assert_eq!(ids(standard(false)), ["title", "artist", "duration"]);
    }

    #[test]
    fn a_hidden_or_unknown_column_is_not_drawn() {
        let mut hidden = Column::new("artist");
        hidden.visible = false;
        let layout = visible(&[Column::new("title"), hidden, Column::new("gone")]);
        assert_eq!(layout.len(), 1);
        assert_eq!(layout[0].spec.id, "title");
    }
}
