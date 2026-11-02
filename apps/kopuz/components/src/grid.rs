//! The virtualized grid every catalog page lays its tiles in: bands of tiles, or one row per entry.

use std::cell::Cell;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, RenderOnce, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window, div, px, uniform_list,
};

use crate::{motion, scroll, style};

/// Until the list has been laid out once there is no measured width to divide.
const COLUMNS_FALLBACK: usize = 4;
/// The jump rail's width, which the bands do not get to use.
const RAIL: Pixels = px(20.);
/// The letters the rail offers, with everything else gathered under the first.
const LETTERS: &str = "#ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Whether a grid draws bands of tiles or one row per entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GridMode {
    #[default]
    Tiles,
    Rows,
}

type Build<V> = Rc<dyn Fn(&mut V, usize, &mut Window, &mut Context<V>) -> AnyElement>;
type Visit<V> = Rc<dyn Fn(&mut V, Range<usize>, &mut Window, &mut Context<V>)>;
type Jump = Rc<dyn Fn(char, &mut Window, &mut App)>;

/// A page's virtualized grid: its scroller, and how many tiles its last layout fit across.
pub struct TileGrid {
    id: SharedString,
    scroll: UniformListScrollHandle,
    gap: Pixels,
    fallback: usize,
    columns: Cell<usize>,
}

impl TileGrid {
    pub fn new(id: &'static str) -> Self {
        Self {
            id: SharedString::new_static(id),
            scroll: UniformListScrollHandle::new(),
            gap: px(24.),
            fallback: COLUMNS_FALLBACK,
            columns: Cell::new(COLUMNS_FALLBACK),
        }
    }

    /// Scroll with a handle another list on the page drives too, so both keep one position.
    pub fn sharing(mut self, scroll: UniformListScrollHandle) -> Self {
        self.scroll = scroll;
        self
    }

    /// The space between tiles, and under each band; `gap-6` unless a page says otherwise.
    pub fn gap(mut self, gap: Pixels) -> Self {
        self.gap = gap;
        self
    }

    /// How many tiles a band holds before there is a measured width to divide.
    pub fn fallback(mut self, columns: usize) -> Self {
        self.fallback = columns;
        self.columns.set(columns);
        self
    }

    /// How many entries a band held at the last render: one in row mode.
    pub fn columns(&self) -> usize {
        self.columns.get().max(1)
    }

    /// Put entry `index` at the top of the view, in whichever band holds it.
    pub fn scroll_to(&self, index: usize) {
        self.scroll
            .scroll_to_item(index / self.columns(), ScrollStrategy::Top);
    }

    /// How many tiles fit across, from the width the list reported at its last layout.
    fn fit(&self, tile: Pixels) -> usize {
        let width: Pixels = self.scroll.0.borrow().base_handle.bounds().size.width;
        if width <= px(0.) {
            return self.fallback;
        }
        // The last tile in a band needs no gap after it.
        (((width + self.gap) / (tile + self.gap)) as usize).max(1)
    }

    /// `count` entries, each `tile` wide in tile mode, built by `cell` only once its band is on screen.
    pub fn bands<V: 'static>(
        &self,
        count: usize,
        tile: Pixels,
        mode: GridMode,
        cell: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) -> AnyElement + 'static,
    ) -> Bands<'_, V> {
        Bands {
            grid: self,
            count,
            tile,
            mode,
            cell: Rc::new(cell),
            visit: None,
            rail: None,
        }
    }
}

/// One render of a [`TileGrid`], with its optional hooks.
pub struct Bands<'a, V> {
    grid: &'a TileGrid,
    count: usize,
    tile: Pixels,
    mode: GridMode,
    cell: Build<V>,
    visit: Option<Visit<V>>,
    rail: Option<AnyElement>,
}

impl<V: 'static> Bands<'_, V> {
    /// Told which entries the bands on screen cover, before they are built: for paging or photos.
    pub fn visit(
        mut self,
        visit: impl Fn(&mut V, Range<usize>, &mut Window, &mut Context<V>) + 'static,
    ) -> Self {
        self.visit = Some(Rc::new(visit));
        self
    }

    /// A column beside the bands, such as a [`JumpRail`].
    pub fn rail(mut self, rail: Option<impl IntoElement>) -> Self {
        self.rail = rail.map(IntoElement::into_any_element);
        self
    }

    pub fn build(self, window: &mut Window, cx: &mut Context<V>) -> AnyElement {
        let grid = self.grid;
        scroll::smooth_list(grid.id.clone(), &grid.scroll, window, cx);
        let tiles = self.mode == GridMode::Tiles;
        let columns = match tiles {
            true => grid.fit(self.tile),
            false => 1,
        };
        grid.columns.set(columns);
        let count = self.count;
        let gap = grid.gap;
        let cell = self.cell;
        let visit = self.visit;
        let list = uniform_list(
            grid.id.clone(),
            count.div_ceil(columns),
            cx.processor(move |this, range: Range<usize>, window, cx| {
                let shown = (range.start * columns).min(count)..(range.end * columns).min(count);
                if let Some(visit) = visit.as_ref() {
                    visit(this, shown, window, cx);
                }
                range
                    .map(|band| {
                        let first = band * columns;
                        let cells: Vec<AnyElement> = (first..(first + columns).min(count))
                            .map(|index| cell(this, index, window, cx))
                            .collect();
                        match tiles {
                            true => h_flex()
                                .w_full()
                                .items_start()
                                .gap(gap)
                                .pb(gap)
                                .children(cells)
                                .into_any_element(),
                            false => v_flex().w_full().pb_1().children(cells).into_any_element(),
                        }
                    })
                    .collect()
            }),
        )
        .track_scroll(&grid.scroll)
        .size_full();
        match self.rail {
            Some(rail) => h_flex()
                .size_full()
                .items_stretch()
                .child(div().flex_1().min_w_0().child(list))
                .child(rail)
                .into_any_element(),
            None => list.into_any_element(),
        }
    }
}

/// Which letter of a rail a sort key falls under; everything non-alphabetic gathers under `#`.
pub fn rail_letter(sort_key: &str) -> char {
    match sort_key.chars().next() {
        Some(first) if first.is_ascii_alphabetic() => first.to_ascii_uppercase(),
        _ => '#',
    }
}

/// The letters down a grid's edge: a long grid is reachable without dragging a scrollbar.
#[derive(IntoElement)]
pub struct JumpRail {
    id: SharedString,
    present: HashSet<char>,
    jump: Jump,
}

impl JumpRail {
    /// `present` holds the letters some entry starts with; the rest show but do not jump.
    pub fn new(
        id: &'static str,
        present: HashSet<char>,
        jump: impl Fn(char, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: SharedString::new_static(id),
            present,
            jump: Rc::new(jump),
        }
    }
}

impl RenderOnce for JumpRail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hover = SharedString::from(format!("{}-hover", self.id));
        let tint = SharedString::from(format!("{}-ink", self.id));
        let letters: Vec<_> = LETTERS
            .chars()
            .map(|letter| {
                let known = self.present.contains(&letter);
                let (over, pointer) =
                    motion::hovered(motion::channel(&hover, letter as u64), window, cx);
                let ink = motion::color(
                    motion::channel(&tint, letter as u64),
                    match (known, over) {
                        (true, true) => cx.theme().foreground,
                        (true, false) => style::white(style::TEXT_SECONDARY, cx),
                        (false, _) => style::white(style::TEXT_FAINT, cx),
                    },
                    window,
                    cx,
                );
                let jump = self.jump.clone();
                div()
                    .id(ElementId::NamedInteger(self.id.clone(), letter as u64))
                    .w_full()
                    .text_center()
                    .text_size(style::text_10())
                    .text_color(ink)
                    .when(known, |entry| {
                        entry
                            .cursor_pointer()
                            .on_hover(motion::track(pointer))
                            .on_click(move |_, window, cx| jump(letter, window, cx))
                    })
                    .child(letter.to_string())
            })
            .collect();
        v_flex()
            .w(RAIL)
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .py_2()
            .children(letters)
    }
}

#[cfg(test)]
mod tests {
    use super::rail_letter;

    #[test]
    fn a_title_that_starts_with_anything_but_a_letter_gathers_under_the_hash() {
        assert_eq!(rail_letter("abbey road"), 'A');
        assert_eq!(rail_letter("1989"), '#');
        assert_eq!(rail_letter(""), '#');
    }
}
