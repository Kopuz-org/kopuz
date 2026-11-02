//! The playlists inside one folder.

use components::format::t;
use components::{EmptyState, Loading, icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use store::{Navigator, PlaylistsModel, Route};

use crate::cards::playlist_card;
use components::tile_grid as card_grid;

pub struct PlaylistFolder {
    id: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl PlaylistFolder {
    pub fn new(id: SharedString, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&PlaylistsModel::global(cx), |_, _, cx| cx.notify())];
        Self {
            id,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for PlaylistFolder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = PlaylistsModel::global(cx);
        let loaded = model.read(cx).is_loaded();
        let folder = model
            .read(cx)
            .folders()
            .iter()
            .find(|folder| folder.id == self.id.as_ref())
            .cloned();
        let playlists = model.read(cx).playlists().to_vec();
        let Some(folder) = folder else {
            return components::page().child(if loaded {
                EmptyState::new(t("no_playlists_yet"))
                    .icon(icons::FOLDER)
                    .into_any_element()
            } else {
                Loading::new().into_any_element()
            });
        };
        let inside: Vec<_> = folder
            .playlist_ids
            .iter()
            .filter_map(|id| playlists.iter().find(|playlist| &playlist.id == id))
            .cloned()
            .collect();

        components::page()
            .child(breadcrumb(folder.name.clone(), cx))
            .child(
                v_flex()
                    .id("folder-body")
                    .flex_1()
                    .min_h_0()
                    .pb_20()
                    .overflow_y_scroll()
                    .child(if inside.is_empty() {
                        EmptyState::new(t("no_playlists_yet"))
                            .icon(icons::FOLDER)
                            .into_any_element()
                    } else {
                        card_grid()
                            .children(inside.iter().map(playlist_card))
                            .into_any_element()
                    }),
            )
    }
}

/// Back, a dim slash, and the folder's name: the trail the Dioxus page draws above the grid.
fn breadcrumb(name: String, cx: &App) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_center()
        .pb_8()
        .child(
            Button::new("folder-back")
                .custom(style::quiet_button(style::slate_400(cx), cx))
                .icon(icons::icon(icons::CHEVRON_LEFT))
                .on_click(|_, _, cx| Navigator::navigate(Route::Playlists, cx)),
        )
        .child(
            div()
                .text_color(style::white(style::TEXT_DIM, cx))
                .child("/"),
        )
        .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
}
