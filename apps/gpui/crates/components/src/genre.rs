//! One genre as a card: its name over the cover of the first album filed under it.

use api::ArtworkRef;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div, px,
};
use store::{Navigator, Route};

use crate::{Cover, Scrim, motion, style};

/// One genre tile, wide enough for a name and a cover behind it.
pub const GENRE_TILE: Pixels = px(240.);
pub const GENRE_TILE_GAP: Pixels = px(16.);
// `aspect-video`, stated rather than derived: a virtualized band has to know its height up front.
const TILE_HEIGHT: Pixels = px(135.);

/// The card's channels, keyed off its name.
const HOVER: u64 = 1;
const WASH: u64 = 2;
const COVER: u64 = 3;

/// `aspect-video bg-white/5 border border-white/10 rounded-xl p-4`: one genre, as a card.
#[derive(IntoElement)]
pub struct GenreCard {
    name: SharedString,
    artwork: Option<ArtworkRef>,
}

impl GenreCard {
    pub fn new(name: impl Into<SharedString>, artwork: Option<ArtworkRef>) -> Self {
        Self {
            name: name.into(),
            artwork,
        }
    }
}

impl RenderOnce for GenreCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = SharedString::from(format!("genre-{}", self.name));
        let route = Route::Genre(self.name.clone());
        profile::elements::built();
        let (hovered, pointer) = motion::hovered(motion::channel(&key, HOVER), window, cx);
        let wash = motion::color(
            motion::channel(&key, WASH),
            style::white(if hovered { style::HOVER } else { style::RAISED }, cx),
            window,
            cx,
        );
        div()
            .id(key.clone())
            .relative()
            .overflow_hidden()
            .flex()
            .items_end()
            .w(GENRE_TILE)
            .h(TILE_HEIGHT)
            .flex_shrink_0()
            .p_4()
            .rounded(style::radius_xl())
            .border_1()
            .border_color(style::white(style::BORDER, cx))
            .bg(wash)
            .cursor_pointer()
            .on_hover(motion::track(pointer))
            .on_click(move |_, _, cx| Navigator::navigate(route.clone(), cx))
            .when_some(self.artwork, |tile, artwork| {
                tile.child(
                    Cover::new(Some(&artwork))
                        .fill()
                        .zoom(motion::channel(&key, COVER), hovered)
                        .zoom_factor(motion::ZOOM_LG)
                        .scrim(Scrim {
                            foot: 0.8,
                            top: 0.,
                            reach: 0.5,
                        }),
                )
            })
            .child(
                div()
                    .relative()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child(self.name),
            )
    }
}
