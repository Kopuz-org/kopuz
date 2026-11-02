//! Settings → Interface → Home: which shelves the page draws, and in what order.

use components::format::t;
use components::settings::{order_row, section};
use gpui_kit::component::v_flex;
use gpui_kit::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled, Window,
};
use store::layout::{CATALOG_SHELVES, SHELVES, Section as Group};
use store::{LayoutModel, SourcesModel};

/// Registered by the app at startup, beside the other Interface sections.
pub fn register(cx: &mut App) {
    settings_interface::register(
        "home",
        "customize_home",
        3,
        Some(Group::Home),
        |window, cx| cx.new(|cx| Shelves::new(window, cx)).into(),
        cx,
    );
}

struct Shelves;

impl Shelves {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&LayoutModel::global(cx), |_, _, cx| cx.notify())
            .detach();
        // The layout being arranged is the one Home draws for the active source.
        cx.observe(&SourcesModel::global(cx), |_, _, cx| cx.notify())
            .detach();
        Self
    }
}

impl Render for Shelves {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = store::layout(cx);
        let rows = match SourcesModel::global(cx).read(cx).has_catalog() {
            true => CATALOG_SHELVES
                .iter()
                .filter_map(|name| {
                    order_row(
                        "home",
                        name,
                        t(crate::shelf::title(name)),
                        &layout.catalog_home,
                        |layout| &mut layout.catalog_home,
                        cx,
                    )
                    .shown(cx)
                })
                .collect::<Vec<_>>(),
            false => SHELVES
                .iter()
                .filter_map(|name| {
                    order_row(
                        "home",
                        name,
                        t(crate::shelf::title(name)),
                        &layout.home,
                        |layout| &mut layout.home,
                        cx,
                    )
                    .shown(cx)
                })
                .collect::<Vec<_>>(),
        };
        v_flex()
            .w_full()
            .child(section(t("home")).children(rows))
            .into_any_element()
    }
}
