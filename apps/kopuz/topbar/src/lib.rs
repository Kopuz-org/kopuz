//! The navigation bar: history, the destination tabs, the palette field and the right-hand cluster.

mod bounds;
mod offline;
mod settings;
mod switcher;
mod tabs;

use components::format::t;
use components::marker::Along;
use components::{Bar, SearchTrigger, Tab, TabStrip, icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, Context, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, NavigationDirection, ParentElement as _, Pixels, Render, SharedString, Styled,
    Subscription, Window, div, px,
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
// `bg-black/40` and `border-white/5`: the chrome's own scrim, as the rail wore it.
const SCRIM: f32 = 0.40;

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

    fn open(&self, index: usize, cx: &mut App) {
        let Some(name) = self.destinations.get(index) else {
            return;
        };
        if let Some(route) = Route::destination(name) {
            Navigator::navigate(route, cx);
        }
    }

    fn strip(&self, current: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let vertical = self.position == NavPosition::Left;
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
        let compact = self.compact && self.position == NavPosition::Top;
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

        match self.position {
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let strip = self.strip(current, cx);
        let search = self.search(cx);
        let history = self.history(cx);
        let cluster = self.cluster(cx);

        match self.position {
            NavPosition::Top => h_flex()
                .w_full()
                .h(style::chrome().bar_height)
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .px_2()
                .bg(style::black(SCRIM, cx))
                .border_b_1()
                .border_color(style::white(style::BORDER_SOFT, cx))
                .child(
                    h_flex()
                        .h_full()
                        .items_center()
                        .gap_2()
                        .child(history)
                        .child(strip),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .justify_center()
                        .px_2()
                        .child(search),
                )
                .child(cluster)
                .into_any_element(),
            NavPosition::Left => v_flex()
                .h_full()
                .w(style::chrome().rail_width)
                .flex_shrink_0()
                .gap_2()
                .p_2()
                .bg(style::black(SCRIM, cx))
                .border_r_1()
                .border_color(style::white(style::BORDER_SOFT, cx))
                .child(history)
                .child(search)
                .child(strip)
                .child(div().flex_1().min_h_0())
                .child(v_flex().w_full().gap_1().child(cluster))
                .into_any_element(),
        }
    }
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
