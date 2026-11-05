//! The navigation bar: history, the destination tabs, the palette field and the right-hand cluster.

mod bounds;
pub mod frame;
mod offline;
mod settings;
mod switcher;
mod tabs;

use components::format::t;
use components::marker::Along;
use components::skin::{self, Nav};
use components::{Bar, SearchTrigger, Tab, TabStrip, icons, style};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, Context, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, NavigationDirection, ParentElement as _, Pixels, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, Window, div, px,
};
use store::layout::{NavPosition, Ordered as _};
use store::{ConfigModel, LayoutModel, Navigator, Route, SourcesModel};

pub use bounds::{remember as remember_bounds, saved_bounds};
pub use settings::register as register_settings;
pub use switcher::SourceSwitcher;

/// Below this the tab labels go, leaving the glyphs.
pub const COMPACT_BELOW: Pixels = px(700.);
/// The width past which a table stops stretching and a grid starts centring.
pub const CONTENT_MAX: Pixels = px(1400.);

const CHROME_GLYPH: Pixels = px(15.);
/// Room above the rail's first row for macOS's window buttons, which sit over it.
const RAIL_TRAFFIC_LIGHTS: Pixels = px(28.);
// `bg-black/40` and `border-white/5`: the chrome's own scrim, as the rail wore it.
const SCRIM: f32 = 0.40;
/// `h-10`: the sectioned sidebar's own room for the window buttons.
const SIDEBAR_TRAFFIC_LIGHTS: Pixels = px(40.);
// A section heading's ink and the divider's, as fractions of the sidebar's text colour.
const SECTION_INK: f32 = 0.25;
const DIVIDER: f32 = 0.06;

/// The frame a page draws inside, so a table stops stretching and a grid centres.
pub fn content_frame() -> gpui_kit::Div {
    div().w_full().max_w(CONTENT_MAX).mx_auto()
}

/// Mouse buttons 4 and 5, for the root of whatever window wants history on them.
pub fn mouse_history(root: gpui_kit::Div) -> gpui_kit::Div {
    root.on_mouse_down(
        MouseButton::Navigate(NavigationDirection::Back),
        |_, _, cx| {
            Navigator::global(cx).update(cx, |navigator, cx| navigator.go_back(cx));
        },
    )
    .on_mouse_down(
        MouseButton::Navigate(NavigationDirection::Forward),
        |_: &MouseDownEvent, _, cx| {
            Navigator::global(cx).update(cx, |navigator, cx| navigator.go_forward(cx));
        },
    )
}

/// The bar. Same entity either way up: `layout.nav.position` picks the axis.
pub struct TopBar {
    destinations: Vec<&'static str>,
    position: NavPosition,
    activity: Option<AnyView>,
    compact: bool,
    _subscriptions: Vec<Subscription>,
}

impl TopBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let layout = LayoutModel::global(cx);
        let subscriptions = vec![
            // Derived here rather than in render: the bar redraws on every notify in the app.
            cx.observe(&layout, |this: &mut Self, _, cx| {
                this.reread(cx);
                cx.notify();
            }),
            cx.observe(&Navigator::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&ConfigModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&SourcesModel::global(cx), |_, _, cx| cx.notify()),
            cx.observe_window_bounds(window, |this, window, cx| {
                bounds::remember(window, cx);
                let compact = window.viewport_size().width < COMPACT_BELOW;
                if compact != this.compact {
                    this.compact = compact;
                    cx.notify();
                }
            }),
        ];
        let mut this = Self {
            destinations: Vec::new(),
            position: NavPosition::Top,
            activity: None,
            compact: window.viewport_size().width < COMPACT_BELOW,
            _subscriptions: subscriptions,
        };
        this.reread(cx);
        this
    }

    /// Put a view in the slot left of the source switcher; `None` empties it again.
    pub fn set_activity(&mut self, view: Option<AnyView>, cx: &mut Context<Self>) {
        self.activity = view;
        cx.notify();
    }

    fn reread(&mut self, cx: &App) {
        let nav = &store::layout(cx).nav;
        self.destinations = nav
            .visible()
            .into_iter()
            .filter_map(|name| {
                store::layout::DESTINATIONS
                    .iter()
                    .copied()
                    .find(|d| *d == name)
            })
            .collect();
        self.position = nav.position;
    }

    /// Where the bar sits once the skin has had its say.
    fn position(&self) -> NavPosition {
        skin::nav_position(self.position)
    }

    fn open(&self, index: usize, cx: &mut App) {
        let Some(name) = self.destinations.get(index) else {
            return;
        };
        if let Some(route) = Route::destination(name) {
            Navigator::navigate(route, cx);
        }
    }

    fn strip(&self, current: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let vertical = self.position() == NavPosition::Left;
        let along = match vertical {
            true => Along::Column,
            false => Along::Row,
        };
        let compact = self.compact && !vertical;
        let tabs: Vec<Tab> = self
            .destinations
            .iter()
            .map(|name| {
                let label = t(tabs::label(name));
                let tooltip =
                    compact.then(|| commands::tooltip(tabs::command(name), label.clone(), cx));
                Tab::new(*name, tabs::icon(name))
                    .label((!compact).then_some(label))
                    .tooltip(tooltip)
            })
            .collect();
        let strip = TabStrip::new("nav-strip", along, Bar::Pill)
            .roving("NavBar")
            .current(current.map(|name| SharedString::from(name.to_string())))
            .tabs(tabs)
            .on_select(cx.listener(|this, index: &usize, _, cx| this.open(*index, cx)));
        strip.into_any_element()
    }

    /// The palette's doorway: it looks like a field, and clicking it opens the palette.
    fn search(&self, cx: &App) -> AnyElement {
        let shortcut = commands::keystroke_for("app.palette", cx)
            .and_then(|held| gpui_kit::Keystroke::parse(&held).ok());
        SearchTrigger::new("nav-search", t("search_placeholder"))
            .on_open(|_, cx| store::Chrome::open_quick_search(cx))
            .shortcut(shortcut)
            .into_any_element()
    }

    fn history(&self, cx: &mut Context<Self>) -> AnyElement {
        let navigator = Navigator::global(cx);
        let (back, forward) = {
            let navigator = navigator.read(cx);
            (navigator.can_go_back(), navigator.can_go_forward())
        };
        h_flex()
            .gap_0p5()
            .flex_shrink_0()
            .child(
                chrome_button("nav-back", icons::CHEVRON_LEFT, cx)
                    .disabled(!back)
                    .tooltip(commands::tooltip("nav.back", t("go_back"), cx))
                    .on_click(|_, _, cx| {
                        Navigator::global(cx).update(cx, |this, cx| this.go_back(cx));
                    }),
            )
            .child(
                chrome_button("nav-forward", icons::CHEVRON_RIGHT, cx)
                    .disabled(!forward)
                    .tooltip(commands::tooltip("nav.forward", t("go_forward"), cx))
                    .on_click(|_, _, cx| {
                        Navigator::global(cx).update(cx, |this, cx| this.go_forward(cx));
                    }),
            )
            .into_any_element()
    }

    fn cluster(&self, cx: &mut Context<Self>) -> AnyElement {
        let sources = SourcesModel::global(cx).read(cx).sources().to_vec();
        let status = SourcesModel::global(cx).read(cx).active_status();
        let compact = self.compact && self.position() == NavPosition::Top;
        let show_source_toggle = ConfigModel::global(cx)
            .read(cx)
            .config()
            .ui
            .show_source_toggle;
        let settings = chrome_button("nav-settings", icons::SETTINGS, cx)
            .tooltip(commands::tooltip("app.settings", t("settings"), cx))
            .on_click(|_, _, cx| {
                Navigator::navigate(Route::Settings { section: None }, cx);
            });
        let offline = offline::indicator(compact, cx);
        let source = div().when(!compact, |slot| slot.w(px(180.))).child(
            SourceSwitcher::new(sources, status)
                .collapsed(compact)
                .bare(true),
        );

        match self.position() {
            NavPosition::Top => h_flex()
                .gap_1()
                .flex_shrink_0()
                .items_center()
                .children(self.activity.clone())
                .children(offline)
                .when(show_source_toggle, |cluster| cluster.child(source))
                .child(settings)
                .into_any_element(),
            NavPosition::Left => v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .children(self.activity.clone())
                        .children(offline)
                        .child(settings),
                )
                .when(show_source_toggle, |cluster| {
                    cluster.child(div().w_full().child(source))
                })
                .into_any_element(),
        }
    }
}

impl Render for TopBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("topbar");
        let artists_remote = store::SourcesModel::global(cx)
            .read(cx)
            .capabilities()
            .artists
            == api::ArtistPresentation::Remote;
        let current = Navigator::global(cx)
            .read(cx)
            .current()
            .destination_of(artists_remote);
        if skin::arrangement().nav == Nav::Sections {
            return self.sidebar(window, cx);
        }
        let strip = self.strip(current, cx);
        let search = self.search(cx);
        let history = self.history(cx);
        let cluster = self.cluster(cx);

        let traffic_lights = frame::traffic_lights(window, cx);
        let rtl = i18n::is_rtl();

        match self.position() {
            NavPosition::Top => h_flex()
                .w_full()
                .h(style::chrome().bar_height)
                .flex_shrink_0()
                .items_center()
                .bg(style::black(SCRIM, cx))
                .border_b_1()
                .border_color(style::white(style::BORDER_SOFT, cx))
                // The window buttons stay at the left whichever way the bar reads.
                .when_some(traffic_lights, |bar, inset| {
                    bar.child(
                        frame::drag_region("nav-traffic-lights", window, cx)
                            .h_full()
                            .w(inset)
                            .flex_shrink_0(),
                    )
                })
                .child(
                    h_flex()
                        .h_full()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .when(rtl, |bar| bar.flex_row_reverse())
                        .child(
                            h_flex()
                                .h_full()
                                .items_center()
                                .gap_2()
                                .when(rtl, |group| group.flex_row_reverse())
                                .child(history)
                                .child(strip),
                        )
                        .child(
                            h_flex()
                                .h_full()
                                .flex_1()
                                .min_w_0()
                                .items_center()
                                .child(
                                    frame::drag_region("nav-drag-start", window, cx)
                                        .h_full()
                                        .flex_1(),
                                )
                                .child(search)
                                .child(
                                    frame::drag_region("nav-drag-end", window, cx)
                                        .h_full()
                                        .flex_1(),
                                ),
                        )
                        .child(cluster),
                )
                .into_any_element(),
            NavPosition::Left => v_flex()
                .h_full()
                .w(style::chrome().rail_width)
                .flex_shrink_0()
                .gap_2()
                .p_2()
                .bg(style::black(SCRIM, cx))
                .map(|rail| match rtl {
                    true => rail.border_l_1(),
                    false => rail.border_r_1(),
                })
                .border_color(style::white(style::BORDER_SOFT, cx))
                .when(traffic_lights.is_some(), |rail| {
                    rail.child(
                        frame::drag_region("nav-rail-drag", window, cx)
                            .w_full()
                            .h(RAIL_TRAFFIC_LIGHTS)
                            .flex_shrink_0(),
                    )
                })
                .child(history)
                .child(search)
                .child(strip)
                .child(div().flex_1().min_h_0())
                .child(v_flex().w_full().gap_1().child(cluster))
                .into_any_element(),
        }
    }
}

impl TopBar {
    /// The sectioned sidebar: the switcher, each section under its heading, then the tools below a rule.
    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let history = self.history(cx);
        let artists_remote = store::SourcesModel::global(cx)
            .read(cx)
            .capabilities()
            .artists
            == api::ArtistPresentation::Remote;
        let lit = skin::entry_of(Navigator::global(cx).read(cx).current(), artists_remote);
        let over_art = theme::ambient(cx).is_some() || {
            let config = ConfigModel::global(cx);
            let background = &config.read(cx).config().background;
            background.cover_art_background || !background.custom_background_path.is_empty()
        };
        let show_source_toggle = ConfigModel::global(cx)
            .read(cx)
            .config()
            .ui
            .show_source_toggle;
        let sources = SourcesModel::global(cx).read(cx).sources().to_vec();
        let status = SourcesModel::global(cx).read(cx).active_status();
        // The sections are fixed, as upstream: what the strip hides is the strip's own choice.
        let sections = skin::SECTIONS.iter().map(|section| {
            v_flex()
                .w_full()
                .mb_2()
                .child(
                    div()
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .text_size(style::text_10())
                        .font_weight(FontWeight::BOLD)
                        .text_color(style::white(SECTION_INK, cx))
                        .child(t(section.label)),
                )
                .child(entry_strip(section.label, section.entries.to_vec(), lit))
        });
        let traffic_lights = frame::traffic_lights(window, cx);

        v_flex()
            .h_full()
            .w(style::chrome().rail_width)
            .flex_shrink_0()
            .map(|sidebar| match over_art {
                true => sidebar.bg(style::black(SCRIM, cx)),
                false => sidebar.bg(cx.theme().popover),
            })
            .map(|sidebar| match i18n::is_rtl() {
                true => sidebar.border_l_1(),
                false => sidebar.border_r_1(),
            })
            .border_color(style::white(style::BORDER_SOFT, cx))
            .when(traffic_lights.is_some(), |sidebar| {
                sidebar.child(
                    frame::drag_region("nav-sidebar-drag", window, cx)
                        .w_full()
                        .h(SIDEBAR_TRAFFIC_LIGHTS)
                        .flex_shrink_0(),
                )
            })
            .when(show_source_toggle, |sidebar| {
                sidebar.child(
                    div().w_full().px_2().pb_1().child(
                        SourceSwitcher::new(sources, status)
                            .collapsed(false)
                            .bare(true),
                    ),
                )
            })
            .child(
                v_flex()
                    .id("nav-sidebar-entries")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_1()
                    .py_2()
                    .children(sections)
                    .child(
                        div()
                            .mx_2()
                            .my_2()
                            .h(px(1.))
                            .flex_shrink_0()
                            .bg(style::white(DIVIDER, cx)),
                    )
                    .child(entry_strip("tools", skin::TOOLS.to_vec(), lit)),
            )
            .child(
                h_flex()
                    .w_full()
                    .px_2()
                    .pb_2()
                    .gap_1()
                    .items_center()
                    .justify_between()
                    .child(history)
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .children(self.activity.clone())
                            .children(offline::indicator(false, cx)),
                    ),
            )
            .into_any_element()
    }
}

/// One section's entries as a roving strip, lit on `lit` when it is one of them.
fn entry_strip(id: &'static str, entries: Vec<&'static str>, lit: &'static str) -> TabStrip {
    let current = entries.contains(&lit).then_some(lit);
    let tabs = entries
        .iter()
        .map(|entry| Tab::new(*entry, tabs::icon(entry)).label(Some(t(tabs::label(entry)))));
    TabStrip::new(id, Along::Column, Bar::Sidebar)
        .roving("NavBar")
        .current(current)
        .tabs(tabs)
        .on_select(move |index: &usize, _, cx| {
            if let Some(route) = entries.get(*index).and_then(|entry| skin::route_of(entry)) {
                Navigator::navigate(route, cx);
            }
        })
}

/// The bar's own icon buttons: no fill of their own, a wash under the pointer.
fn chrome_button(id: &'static str, glyph: &'static str, cx: &App) -> Button {
    Button::new(id)
        .tab_index(0)
        .custom(style::quiet_button(style::slate_400(cx), cx))
        .with_size(CHROME_GLYPH * (4. / 3.))
        .size(style::chrome().button)
        .rounded(style::icon_corner(style::chrome().button))
        .icon(icons::icon(glyph))
}
