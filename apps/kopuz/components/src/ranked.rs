//! A ranking reads as a list, not a row of equals: it is numbered, and it fills its columns top to bottom.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled, Window,
};

use crate::{SectionHeading, TileRow};

/// How many rows go in each of `columns`, the first filling before the next.
fn column_heights(rows: usize, columns: usize) -> Vec<usize> {
    let per = rows.div_ceil(columns.max(1)).max(1);
    let mut heights = Vec::new();
    let mut rest = rows;
    while rest > 0 {
        let take = per.min(rest);
        heights.push(take);
        rest -= take;
    }
    heights
}

/// Titled rows numbered from one, set down each column rather than across.
#[derive(IntoElement)]
pub struct RankedList {
    title: SharedString,
    rows: Vec<TileRow>,
    columns: usize,
}

impl RankedList {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
            columns: 2,
        }
    }

    /// Eight names read as a list at a glance in two columns, sixteen as a wall of text in one.
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns;
        self
    }

    /// The rows in rank order; the number on each is added here.
    pub fn rows(mut self, rows: impl IntoIterator<Item = TileRow>) -> Self {
        self.rows.extend(rows);
        self
    }
}

impl RenderOnce for RankedList {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let heights = column_heights(self.rows.len(), self.columns);
        let mut rows = self.rows.into_iter().enumerate();
        let columns: Vec<Vec<AnyElement>> = heights
            .into_iter()
            .map(|height| {
                rows.by_ref()
                    .take(height)
                    .map(|(index, row)| row.lead((index + 1).to_string()).into_any_element())
                    .collect()
            })
            .collect();
        v_flex()
            .w_full()
            .gap_4()
            .child(SectionHeading::new(self.title))
            .child(
                h_flex().w_full().gap_8().items_start().children(
                    columns
                        .into_iter()
                        .map(|column| v_flex().flex_1().min_w_0().gap_1().children(column)),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::column_heights;

    #[test]
    fn a_ranking_fills_the_first_column_before_the_second() {
        assert_eq!(column_heights(8, 2), [4, 4]);
        assert_eq!(column_heights(7, 2), [4, 3]);
        assert_eq!(column_heights(1, 2), [1]);
        assert!(column_heights(0, 2).is_empty());
    }
}
