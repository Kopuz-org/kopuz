//! Settings → Interface → Library: the defaults every facet starts from.

use components::format::t;
use components::settings::{SettingRow, choice, section};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
};
use store::LayoutModel;
use store::layout::{Density, Section as Group, TileSize, ViewMode};

use crate::facet::Facet;

/// Contributed from here, so no other package ever writes the library's own preferences.
pub fn register(cx: &mut App) {
    settings_interface::register(
        "library",
        "interface_library",
        3,
        Some(Group::Library),
        |window, cx| cx.new(|cx| LibrarySettings::new(window, cx)).into(),
        cx,
    );
}

/// The knobs, drawn as one section; each writes straight into the facet it belongs to.
pub struct LibrarySettings;

impl LibrarySettings {
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Self
    }
}

impl Render for LibrarySettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tracks = store::layout(cx).library_facet(Facet::Tracks.name());
        let albums = store::layout(cx).library_facet(Facet::Albums.name());
        v_flex().w_full().child(
            section(t("interface_library"))
                .children(
                    SettingRow::new(t("library_density"))
                        .description(t("library_density_description"))
                        .control(choice(
                            "library-density-setting",
                            &tracks.density,
                            vec![
                                (Density::Comfortable, t("library_density_comfortable")),
                                (Density::Compact, t("library_density_compact")),
                            ],
                            |value, cx| {
                                write(Facet::Tracks, cx, move |facet| facet.density = value)
                            },
                        ))
                        .shown(cx),
                )
                .children(
                    SettingRow::new(t("library_default_view"))
                        .description(t("library_default_view_description"))
                        .control(choice(
                            "library-view-setting",
                            &albums.mode,
                            vec![
                                (ViewMode::Grid, t("view_grid")),
                                (ViewMode::Table, t("view_list")),
                            ],
                            |value, cx| {
                                for facet in [Facet::Albums, Facet::Artists, Facet::Playlists] {
                                    write(facet, cx, move |held| held.mode = value);
                                }
                            },
                        ))
                        .shown(cx),
                )
                .children(
                    SettingRow::new(t("library_tile_size"))
                        .description(t("library_tile_size_description"))
                        .control(choice(
                            "library-tile-setting",
                            &albums.tile,
                            vec![
                                (TileSize::Small, t("library_tile_small")),
                                (TileSize::Medium, t("library_tile_medium")),
                                (TileSize::Large, t("library_tile_large")),
                            ],
                            |value, cx| {
                                for facet in [Facet::Albums, Facet::Artists, Facet::Playlists] {
                                    write(facet, cx, move |held| held.tile = value);
                                }
                            },
                        ))
                        .shown(cx),
                ),
        )
    }
}

fn write(facet: Facet, cx: &mut App, edit: impl FnOnce(&mut store::layout::Facet)) {
    let name = facet.name().to_string();
    LayoutModel::edit(cx, move |layout| layout.edit_library_facet(&name, edit));
}
