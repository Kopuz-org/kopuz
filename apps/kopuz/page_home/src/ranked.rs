//! A ranking reads as a list, not a row of equals: it is numbered, and it says what it counted.

use components::format::t;
use components::{RankedList, TileRow};
use gpui_kit::{AnyElement, App, IntoElement};

use crate::data::Shelf;

pub(crate) fn ranked(shelf: &Shelf, cx: &App) -> AnyElement {
    let thumb = store::ConfigModel::global(cx).read(cx).row_images();
    RankedList::new(t(crate::shelf::title(shelf.name)))
        .rows(shelf.tiles.iter().map(|tile| row(tile, thumb)))
        .into_any_element()
}

fn row(tile: &crate::data::Tile, thumb: bool) -> TileRow {
    let row = TileRow::new(tile.key.clone(), tile.title.clone())
        .thumb(thumb)
        .subtitle(tile.subtitle.clone())
        .artwork(tile.artwork.as_ref())
        .round(true)
        .placeholder(tile.placeholder)
        .pictured(tile.pictured.clone())
        .menu(Some(tile.target.clone()));
    match tile.route.clone() {
        Some(route) => row.route(route),
        None => row,
    }
}
