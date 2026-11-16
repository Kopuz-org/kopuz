//! The phone's chrome, drawn only under the Android host: a slim header over the page and a tab
//! bar under the player bar, each reaching into the safe area it borders.

use std::rc::Rc;

use api::SourceInfo;
use components::format::t;
use components::{icons, style};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, AnyView, App, Context, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled, div, px,
};
use store::layout::Ordered as _;
use store::{ConfigModel, Navigator, Route, SourcesModel};
use theme::mobile::{HEADER_HEIGHT, TAB_BAR_HEIGHT, TOUCH_TARGET};

use crate::{SCRIM, tabs};

/// How many of the nav's destinations take a tab; Settings always takes the one after them.
const DESTINATION_TABS: usize = 4;
const TAB_GLYPH: Pixels = px(22.);
const TAB_LABEL: Pixels = px(11.);
/// Material's active indicator: a pill behind the lit tab's glyph.
const PILL_WIDTH: Pixels = px(56.);
const PILL_HEIGHT: Pixels = px(28.);
const PILL_TINT: f32 = 0.18;
const HEADER_GLYPH: Pixels = px(20.);
const MENU_GLYPH: Pixels = px(16.);
const MENU_WIDTH: Pixels = px(220.);

/// What a phone header's overflow menu takes from elsewhere: the status cluster's lists, which have no
/// buttons of their own on a phone.
pub type MenuEntries = Rc<dyn Fn(PopupMenu, &App) -> PopupMenu>;

/// The nav's visible destinations in its order: those that take a tab, then those the overflow menu holds.
fn split(cx: &App) -> (Vec<&'static str>, Vec<&'static str>) {
    let mut tabs: Vec<&'static str> = store::layout(cx)
        .nav
        .visible()
        .into_iter()
        .filter_map(|name| {
            store::layout::DESTINATIONS
                .iter()
                .copied()
                .find(|d| *d == name)
        })
        .collect();
    let rest = tabs.split_off(DESTINATION_TABS.min(tabs.len()));
    (tabs, rest)
}

/// The destination the current route lights, and whether it is Settings, which has none.
fn current(cx: &App) -> (Option<&'static str>, bool) {
    let artists_remote =
        SourcesModel::global(cx).read(cx).capabilities().artists == api::ArtistPresentation::Remote;
    let route = Navigator::global(cx).read(cx).current();
    (
        route.destination_of(artists_remote),
        matches!(route, Route::Settings { .. }),
    )
}

/// Whether `route` is the top of a destination the tab bar or its overflow opens, rather than a page
/// below one: Settings in any section, and the library under any facet or filter, count as its top.
fn at_root(route: &Route, roots: &[&'static str]) -> bool {
    match route {
        Route::Settings { .. } => true,
        Route::LibraryView(_) => roots.contains(&"library"),
        route => roots
            .iter()
            .any(|name| Route::destination(name).map(Route::redirect).as_ref() == Some(route)),
    }
}

/// The slim header: back below a tab's top, the page's name, search and the overflow menu, which
/// also holds the status cluster's lists and wears its badge.
pub(crate) fn header(
    activity: Option<AnyView>,
    entries: Option<MenuEntries>,
    cx: &App,
) -> AnyElement {
    let insets = theme::mobile::safe_insets(cx);
    let (tab_names, rest) = split(cx);
    let (lit, settings) = current(cx);
    let title: SharedString = match (settings, lit) {
        (true, _) => t("settings"),
        (false, Some(name)) => t(tabs::label(name)),
        (false, None) => config::flavor::Flavor::current().display_name().into(),
    };
    let back = {
        let navigator = Navigator::global(cx).read(cx);
        let roots: Vec<&'static str> = tab_names.iter().chain(&rest).copied().collect();
        navigator.can_go_back() && !at_root(navigator.current(), &roots)
    };
    h_flex()
        .w_full()
        .h(HEADER_HEIGHT + insets.top)
        .flex_shrink_0()
        .pt(insets.top)
        .pl(insets.left)
        .pr(insets.right)
        .bg(style::black(SCRIM, cx))
        .border_b_1()
        .border_color(style::white(style::BORDER_SOFT, cx))
        .child(
            h_flex()
                .size_full()
                .min_w_0()
                .items_center()
                .gap_0p5()
                .px_1()
                .when(back, |header| {
                    header.child(
                        header_button("phone-back", icons::CHEVRON_LEFT, cx).on_click(
                            |_, _, cx| {
                                Navigator::global(cx).update(cx, |this, cx| this.go_back(cx));
                            },
                        ),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .when(!back, |title| title.pl_3())
                        .truncate()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(title),
                )
                .child(
                    header_button("phone-search", icons::SEARCH, cx)
                        .on_click(|_, _, cx| store::Chrome::open_quick_search(cx)),
                )
                .child(
                    div()
                        .relative()
                        .flex_shrink_0()
                        .size(TOUCH_TARGET)
                        .child(
                            header_button("phone-more", icons::MORE, cx)
                                .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, cx| {
                                    overflow(menu, &rest, entries.as_ref(), cx)
                                }),
                        )
                        .children(
                            activity.map(|cluster| div().absolute().inset_0().child(cluster)),
                        ),
                ),
        )
        .into_any_element()
}

/// What the tabs leave out: the other destinations, the status cluster's lists, the sources and
/// offline mode, whose sign has no room in the header either.
fn overflow(
    menu: PopupMenu,
    rest: &[&'static str],
    entries: Option<&MenuEntries>,
    cx: &Context<PopupMenu>,
) -> PopupMenu {
    let mut menu = menu.min_w(MENU_WIDTH);
    for name in rest {
        let Some(route) = Route::destination(name) else {
            continue;
        };
        menu = menu.item(
            components::menus::entry(
                t(tabs::label(name)),
                Some(Icon::default().path(tabs::icon(name)).size(MENU_GLYPH)),
                false,
                cx,
            )
            .on_click(move |_, _, cx| Navigator::navigate(route.clone(), cx)),
        );
    }
    if let Some(entries) = entries {
        if !rest.is_empty() {
            menu = menu.separator();
        }
        menu = entries(menu, cx);
    }
    let show_source_toggle = ConfigModel::global(cx)
        .read(cx)
        .config()
        .ui
        .show_source_toggle;
    if show_source_toggle {
        let sources: Vec<SourceInfo> = SourcesModel::global(cx).read(cx).sources().to_vec();
        if !rest.is_empty() || entries.is_some() {
            menu = menu.separator();
        }
        menu = menu.label(t("sources"));
        for source in sources {
            let id = source.id.clone();
            menu = menu.item(
                components::menus::entry(SharedString::from(source.name.clone()), None, false, cx)
                    .checked(source.active)
                    .on_click(move |_, window, cx| {
                        SourcesModel::global(cx)
                            .update(cx, |sources, cx| sources.switch(id.clone(), window, cx));
                    }),
            );
        }
    }
    crate::offline::item(menu.separator(), cx).item(
        components::menus::entry(
            t("manage_sources"),
            Some(Icon::default().path(icons::SLIDERS).size(MENU_GLYPH)),
            false,
            cx,
        )
        .on_click(|_, _, cx| {
            Navigator::navigate(
                Route::Settings {
                    section: Some("library".into()),
                },
                cx,
            );
        }),
    )
}

/// The bottom tab bar: the nav's first destinations, then Settings, each a full-height touch target.
pub fn tab_bar(cx: &App) -> AnyElement {
    let insets = theme::mobile::safe_insets(cx);
    let (destinations, _) = split(cx);
    let (lit, settings) = current(cx);
    let tabs = destinations.into_iter().filter_map(|name| {
        let route = Route::destination(name)?;
        Some(tab(
            name,
            tabs::icon(name),
            t(tabs::label(name)),
            lit == Some(name),
            route,
            cx,
        ))
    });
    h_flex()
        .w_full()
        .h(TAB_BAR_HEIGHT + insets.bottom)
        .flex_shrink_0()
        .pb(insets.bottom)
        .pl(insets.left)
        .pr(insets.right)
        .bg(style::black(SCRIM, cx))
        .border_t_1()
        .border_color(style::white(style::BORDER_SOFT, cx))
        .children(tabs)
        .child(tab(
            "settings",
            icons::SETTINGS,
            t("settings"),
            settings,
            Route::Settings { section: None },
            cx,
        ))
        .into_any_element()
}

fn tab(
    name: &'static str,
    glyph: &'static str,
    label: SharedString,
    lit: bool,
    route: Route,
    cx: &App,
) -> impl IntoElement {
    let ink = match lit {
        true => cx.theme().foreground,
        false => style::white(style::TEXT_SUBTLE, cx),
    };
    let glyph_ink = match lit {
        true => cx.theme().primary,
        false => ink,
    };
    v_flex()
        .id(SharedString::from(format!("phone-tab-{name}")))
        .flex_1()
        .min_w(TOUCH_TARGET)
        .h_full()
        .items_center()
        .justify_center()
        .gap_1()
        .cursor_pointer()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(PILL_WIDTH)
                .h(PILL_HEIGHT)
                .rounded_full()
                .when(lit, |pill| pill.bg(cx.theme().primary.opacity(PILL_TINT)))
                .child(icons::icon(glyph).size(TAB_GLYPH).text_color(glyph_ink)),
        )
        .child(
            div()
                .text_size(TAB_LABEL)
                .text_color(ink)
                .when(lit, |label| label.font_weight(FontWeight::SEMIBOLD))
                .child(label),
        )
        .on_click(move |_, _, cx| Navigator::navigate(route.clone(), cx))
}

/// The header's icon buttons: a full touch target, no fill, a wash under the finger.
fn header_button(id: &'static str, glyph: &'static str, cx: &App) -> Button {
    Button::new(id)
        .custom(style::quiet_button(cx.theme().foreground, cx))
        .with_size(HEADER_GLYPH)
        .size(TOUCH_TARGET)
        .rounded(style::icon_corner(TOUCH_TARGET))
        .icon(icons::icon(glyph))
}
