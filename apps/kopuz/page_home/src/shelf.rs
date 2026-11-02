//! A shelf: the shared carousel, titled by its name and filled with its cards.

use components::format::t;
use components::{Carousel, Heading};
use gpui_kit::{AnyElement, IntoElement, ScrollHandle, px};

use crate::cards::{GAP, Now, TILE, card};
use crate::data::{Form, Shelf};

pub(crate) fn shelf(shelf: &Shelf, handle: &ScrollHandle, now: &Now) -> AnyElement {
    carousel(shelf, handle, now).into_any_element()
}

pub(crate) fn carousel(shelf: &Shelf, handle: &ScrollHandle, now: &Now) -> Carousel {
    let tiles = shelf.tiles.clone();
    let now = now.clone();
    Carousel::new(
        shelf.name,
        t(title(shelf.name)),
        tiles.len(),
        TILE,
        move |index, _, _| card(&tiles[index], &now),
    )
    .gap(px(GAP))
    .heading(match shelf.form {
        Form::Mosaic => Heading::Lead,
        _ => Heading::Shelf,
    })
    .handle(handle.clone())
}

/// The i18n key a shelf is titled by.
pub(crate) fn title(name: &str) -> &'static str {
    match name {
        "jump_back_in" => "jump_back_in",
        "for_you" => "for_you",
        "recently_added" => "recently_added",
        "made_for_you" => "made_for_you",
        "top_artists" => "top_artists",
        "your_playlists" => "playlists",
        "catalog" => "home_section_catalog",
        _ => "recently_played",
    }
}

#[cfg(test)]
mod tests {
    use components::format::t;

    #[test]
    fn the_for_you_row_has_its_own_title() {
        assert_ne!(t(super::title("for_you")), "for_you");
        assert_ne!(super::title("for_you"), super::title("made_for_you"));
    }
}
