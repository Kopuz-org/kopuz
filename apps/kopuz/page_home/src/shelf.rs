//! A shelf: the shared carousel, titled by its name and filled with its cards.

use components::format::t;
use components::{Carousel, Heading, SectionHeading};
use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, ScrollHandle, Styled as _, div, px,
};

use crate::cards::{GAP, Now, TILE, card, row};
use crate::data::{Form, LISTEN_NOW_ROWS, Shelf};

pub(crate) fn shelf(shelf: &Shelf, handle: &ScrollHandle, now: &Now) -> AnyElement {
    carousel(shelf, handle, now).into_any_element()
}

/// Listen Now as `ListenNowStyle::List` draws it: a two-column grid of album rows.
pub(crate) fn rows(shelf: &Shelf, now: &Now, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .gap_6()
        .child(SectionHeading::new(t(title(shelf.name))).heading(Heading::Shelf))
        .child(
            div().grid().grid_cols(2).gap_4().children(
                shelf
                    .tiles
                    .iter()
                    .take(LISTEN_NOW_ROWS)
                    .map(|tile| row(tile, now, cx)),
            ),
        )
        .into_any_element()
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
        "hero" => "home_section_hero",
        "jump_back_in" => "jump_back_in",
        "listen_now" => "listen_now",
        "new_releases" => "new_releases",
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
