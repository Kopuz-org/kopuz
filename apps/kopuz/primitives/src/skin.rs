//! What a skin varies: the metrics, chrome proportions and arrangement every surface is drawn by.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gpui_kit::{Pixels, Rems, SharedString, px, rems};
use store::Route;
use store::layout::{Density, NavPosition};

/// The sizes a skin sets; resolved once when the skin changes, never per render.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// `rounded`, the tightest corner in the app.
    pub radius: Rems,
    /// `rounded-md`, the cover thumbs and tab pills.
    pub radius_md: Rems,
    /// `rounded-lg`, the row and small-control corner.
    pub radius_lg: Rems,
    /// `rounded-xl`, the card and panel corner.
    pub radius_xl: Rems,
    /// `rounded-2xl`, the largest corner a surface takes.
    pub radius_2xl: Rems,
    /// The corner of a small piece of art or a pill measured in pixels rather than type.
    pub thumb_radius: Pixels,
    /// The corner of a cover big enough to read as a picture.
    pub cover_radius: Pixels,
    /// The track row, and so the virtualized row height.
    pub row_height: Rems,
    /// A desktop row's cover thumb.
    pub row_cover: Rems,
    /// The square icon buttons in the chrome.
    pub icon_button: Rems,
    /// What `Density::Compact` measures.
    pub compact_row: Pixels,
    /// What `Density::Comfortable` measures.
    pub comfortable_row: Pixels,
    /// A row carrying a cover and two lines: the queue, an album's tracks, a shelf.
    pub media_row: Pixels,
    /// A bare one-line row: the playlists panel.
    pub list_row: Pixels,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            radius: rems(0.25),
            radius_md: rems(0.375),
            radius_lg: rems(0.5),
            radius_xl: rems(0.75),
            radius_2xl: rems(1.),
            thumb_radius: px(6.),
            cover_radius: px(8.),
            row_height: rems(3.5),
            row_cover: rems(2.5),
            icon_button: rems(2.25),
            compact_row: px(32.),
            comfortable_row: px(56.),
            media_row: px(60.),
            list_row: px(34.),
        }
    }
}

impl Metrics {
    /// How tall a row stands at `density`; the user picks the step, the skin says what it measures.
    pub fn density_row(&self, density: Density) -> Pixels {
        match density {
            Density::Compact => self.compact_row,
            Density::Comfortable => self.comfortable_row,
        }
    }
}

/// The chrome's own proportions: the bars, the rail, and the controls that ride on them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    /// The top bar.
    pub bar_height: Pixels,
    /// The left rail, when the navigation is down the side.
    pub rail_width: Pixels,
    /// The search field on the bar.
    pub search_height: Pixels,
    /// The player bar across the bottom.
    pub player_height: Pixels,
    /// A tab in the now-playing panel's strip.
    pub tab_height: Pixels,
    /// The small square buttons on the top bar.
    pub button: Pixels,
    /// The round buttons the player and its panels use.
    pub round_button: Pixels,
    /// The smaller round button a list's own toolbar uses.
    pub tool_button: Pixels,
    /// The corner the page, the side panel and the player bar round to as cards of their own; zero leaves them flush.
    pub pane_radius: Pixels,
    /// The space between those cards and around the window's edge.
    pub pane_gap: Pixels,
    /// The play button as a rounded rectangle, or `None` for the round one.
    pub play_shape: Option<PlayShape>,
}

/// A play button wider than it is tall, both measured against its height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayShape {
    pub width: f32,
    pub corner: f32,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            bar_height: px(44.),
            rail_width: px(200.),
            search_height: px(30.),
            player_height: px(96.),
            tab_height: px(28.),
            button: px(30.),
            round_button: px(36.),
            tool_button: px(28.),
            pane_radius: px(0.),
            pane_gap: px(0.),
            play_shape: None,
        }
    }
}

/// Where a skin arranges a surface differently, rather than only measuring it differently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrangement {
    pub nav: Nav,
    pub bar: BarLayout,
    pub rows: Rows,
    pub pages: Pages,
    pub mini: Mini,
}

impl Default for Arrangement {
    fn default() -> Self {
        Self {
            nav: Nav::Strip,
            bar: BarLayout::Split,
            rows: Rows::default(),
            pages: Pages::default(),
            mini: Mini::default(),
        }
    }
}

/// How the destinations are offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Nav {
    /// The strip the person orders and places, along the top or down the side.
    #[default]
    Strip,
    /// Fixed [`SECTIONS`] down a sidebar, then [`TOOLS`], in an order nobody changes.
    Sections,
}

/// A labelled group of sidebar entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavSection {
    /// The i18n key of the heading, which pages in it also wear above their title.
    pub label: &'static str,
    pub entries: &'static [&'static str],
}

/// The sidebar's sections, as the Dioxus Vaxry sidebar lists them; search is Discover's own field here.
pub const SECTIONS: &[NavSection] = &[
    NavSection {
        label: "discover",
        entries: &["home", "discover", "radio"],
    },
    NavSection {
        label: "library",
        entries: &[
            "library",
            "albums",
            "artists",
            "favorites",
            "activity",
            "playlists",
        ],
    },
];

/// The entries under the sections' divider.
pub const TOOLS: &[&str] = &["downloader", "settings"];

/// Where a sidebar entry goes; the library's listings are its facets, named in the route's payload.
pub fn route_of(entry: &str) -> Option<Route> {
    let facet = |name: &str| Route::LibraryView(SharedString::from(format!("f={name}")));
    Some(match entry {
        "library" => facet("tracks"),
        "albums" => facet("albums"),
        "artists" => facet("artists"),
        "playlists" => facet("playlists"),
        other => Route::from_name(other)?,
    })
}

/// The sidebar entry `route` lights up, detail pages included; `artists_remote` as the router reads it.
pub fn entry_of(route: &Route, artists_remote: bool) -> &'static str {
    match route {
        Route::Home => "home",
        Route::Search
        | Route::Discover
        | Route::DiscoverPlaylist { .. }
        | Route::DiscoverArtist(_)
        | Route::CatalogPage { .. }
        | Route::Genre(_) => "discover",
        Route::Radio => "radio",
        Route::Library => "library",
        Route::LibraryView(view) => {
            let facet = view
                .split('&')
                .find_map(|part| part.strip_prefix("f="))
                .unwrap_or_default();
            match facet {
                "albums" => "albums",
                "artists" => "artists",
                "playlists" => "playlists",
                _ => "library",
            }
        }
        Route::Albums | Route::Album(_) => "albums",
        Route::Artist(_) if artists_remote => "discover",
        Route::Artists | Route::Artist(_) => "artists",
        Route::Favorites => "favorites",
        Route::Activity => "activity",
        Route::Playlists | Route::PlaylistFolder(_) | Route::Playlist(_) => "playlists",
        Route::Downloader => "downloader",
        Route::Settings { .. } => "settings",
    }
}

/// The heading of the section `route` belongs to, if it belongs to one.
pub fn section_of(route: &Route, artists_remote: bool) -> Option<&'static str> {
    let entry = entry_of(route, artists_remote);
    SECTIONS
        .iter()
        .find(|section| section.entries.contains(&entry))
        .map(|section| section.label)
}

/// The order the player bar reads in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarLayout {
    /// The track on the left, the transport over the seek bar in the middle, the tools on the right.
    #[default]
    Split,
    /// The transport first, then the cover and a one-line title over the seek bar, then the tools.
    Inline,
}

/// How a track table's rows and header are drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rows {
    /// The index column's width.
    pub lead: Pixels,
    /// The space between columns; rems, so it follows the interface scale.
    pub gap: Rems,
    /// The space above and below a row's cells.
    pub pad_y: Rems,
    /// The space between a row's cover and its title.
    pub cover_gap: Rems,
    /// The white wash under the pointer.
    pub hover: f32,
    /// A hairline ring drawn with the hover wash, at this alpha.
    pub hover_ring: Option<f32>,
    /// The track number's white alpha.
    pub number: f32,
    /// The header's text size, bold when set; `None` keeps `text-sm` at medium weight.
    pub header_text: Option<Pixels>,
}

impl Default for Rows {
    fn default() -> Self {
        Self {
            lead: px(20.),
            gap: rems(1.5),
            pad_y: rems(0.5),
            cover_gap: rems(1.),
            hover: 0.05,
            hover_ring: None,
            number: 0.60,
            header_text: None,
        }
    }
}

/// How a page frames itself: its padding, its title and a detail page's opening block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pages {
    pub padding: Rems,
    /// The page's section heading over its title, which then takes the icon's place.
    pub eyebrow: bool,
    pub title: Rems,
    /// Whether the home page's greeting is drawn at all.
    pub greeting: bool,
    /// A detail page's cover.
    pub hero: Pixels,
    pub hero_title: Rems,
    /// The space between a detail page's cover and its words.
    pub hero_gap: Rems,
    /// A detail page's actions under its facts, rather than a column of their own at the right.
    pub hero_actions_below: bool,
}

impl Default for Pages {
    fn default() -> Self {
        Self {
            padding: rems(2.),
            eyebrow: false,
            title: rems(1.875),
            greeting: true,
            hero: px(256.),
            hero_title: rems(4.5),
            hero_gap: rems(2.),
            hero_actions_below: false,
        }
    }
}

/// How the mini player departs from its baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mini {
    /// A square cover rather than a rounded one.
    pub square_cover: bool,
    /// The title in capitals, in the monospace face.
    pub caps_title: bool,
    /// Play as a soft wash rather than the filled accent.
    pub soft_play: bool,
}

/// How controls are drawn where a shape is more than a number: a slider's handle, a segment's outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Controls {
    #[default]
    Kopuz,
    /// The Material 3 baseline: tracked sliders with a bar handle, outlined segments, pill buttons.
    Material,
}

/// One skin. Everything but the name has a default, so a skin states only what it changes.
pub trait Skin {
    /// The `ui_style` it answers to, as `UiStyle::as_str` spells it.
    fn id(&self) -> &'static str;

    /// The i18n key naming it in the settings row.
    fn title(&self) -> &'static str;

    fn metrics(&self) -> Metrics {
        Metrics::default()
    }

    fn chrome(&self) -> Chrome {
        Chrome::default()
    }

    fn controls(&self) -> Controls {
        Controls::Kopuz
    }

    fn arrangement(&self) -> Arrangement {
        Arrangement::default()
    }

    /// The face this skin is drawn in, used when the system has it and nothing else is chosen.
    fn font(&self) -> Option<&'static str> {
        None
    }

    /// Asset paths this skin swaps, as `(shipped path, replacement)`; read once on a skin change.
    fn assets(&self) -> Vec<(&'static str, SharedString)> {
        Vec::new()
    }
}

/// Kopuz as it is drawn: the baseline every other skin is a departure from.
pub struct Normal;

impl Skin for Normal {
    fn id(&self) -> &'static str {
        "normal"
    }

    fn title(&self) -> &'static str {
        "ui_normal"
    }
}

/// Material 3 as the Dioxus app draws it: the baseline spec's shape scale, tonal cards and a squared play button.
pub struct Material3;

impl Skin for Material3 {
    fn id(&self) -> &'static str {
        "material3"
    }

    fn title(&self) -> &'static str {
        "ui_material3"
    }

    // Extra small 4, small 8, medium 12, large 16 and extra large 28.
    fn metrics(&self) -> Metrics {
        Metrics {
            radius: rems(0.25),
            radius_md: rems(0.5),
            radius_lg: rems(0.75),
            radius_xl: rems(1.),
            radius_2xl: rems(1.75),
            thumb_radius: px(8.),
            cover_radius: px(12.),
            row_height: rems(3.5),
            row_cover: rems(2.5),
            icon_button: rems(2.5),
            compact_row: px(40.),
            comfortable_row: px(56.),
            media_row: px(64.),
            list_row: px(40.),
        }
    }

    fn chrome(&self) -> Chrome {
        Chrome {
            bar_height: px(64.),
            rail_width: px(220.),
            search_height: px(40.),
            player_height: px(104.),
            tab_height: px(32.),
            button: px(40.),
            round_button: px(40.),
            tool_button: px(32.),
            pane_radius: px(28.),
            pane_gap: px(8.),
            // The medium filled icon button: 56 by 48 with the large corner.
            play_shape: Some(PlayShape {
                width: 56. / 48.,
                corner: 16. / 48.,
            }),
        }
    }

    fn font(&self) -> Option<&'static str> {
        Some("Roboto")
    }

    fn controls(&self) -> Controls {
        Controls::Material
    }
}

/// Vaxry as the Dioxus app draws it: a sectioned sidebar, a one-line player bar and denser rows.
pub struct Vaxry;

impl Skin for Vaxry {
    fn id(&self) -> &'static str {
        "vaxry"
    }

    fn title(&self) -> &'static str {
        "ui_vaxry"
    }

    // A row is `py-1.5` around a `w-8 h-8` cover: 44px, against Normal's 56, in the library's table too.
    fn metrics(&self) -> Metrics {
        Metrics {
            row_height: rems(2.75),
            row_cover: rems(2.),
            comfortable_row: px(44.),
            ..Metrics::default()
        }
    }

    // `h-16`: the bar is one line of controls rather than two.
    fn chrome(&self) -> Chrome {
        Chrome {
            player_height: px(64.),
            ..Chrome::default()
        }
    }

    fn arrangement(&self) -> Arrangement {
        Arrangement {
            nav: Nav::Sections,
            bar: BarLayout::Inline,
            rows: Rows {
                lead: px(40.),
                gap: rems(0.75),
                pad_y: rems(0.375),
                cover_gap: rems(0.5),
                hover: 0.10,
                hover_ring: Some(0.08),
                number: 0.25,
                header_text: Some(px(10.)),
            },
            pages: Pages {
                padding: rems(1.5),
                eyebrow: true,
                title: rems(1.5),
                greeting: false,
                hero: px(176.),
                hero_title: rems(2.25),
                hero_gap: rems(1.5),
                hero_actions_below: true,
            },
            mini: Mini {
                square_cover: true,
                caps_title: true,
                soft_play: true,
            },
        }
    }
}

thread_local! {
    static SKINS: RefCell<Vec<Box<dyn Skin>>> = RefCell::new(vec![Box::new(Normal), Box::new(Vaxry), Box::new(Material3)]);
    static FONT: Cell<Option<&'static str>> = const { Cell::new(None) };
    static CONTROLS: Cell<Controls> = const { Cell::new(Controls::Kopuz) };
    static ACTIVE: Cell<&'static str> = const { Cell::new("normal") };
    static METRICS: Cell<Metrics> = Cell::new(Metrics::default());
    static CHROME: Cell<Chrome> = Cell::new(Chrome::default());
    static ARRANGEMENT: Cell<Arrangement> = Cell::new(Arrangement::default());
    static ASSETS: RefCell<HashMap<&'static str, SharedString>> = RefCell::new(HashMap::new());
}

/// Add a skin to the registry; a second one with the same id replaces the first.
pub fn register(skin: Box<dyn Skin>) {
    SKINS.with_borrow_mut(|skins| {
        let id = skin.id();
        skins.retain(|held| held.id() != id);
        skins.push(skin);
    });
}

/// Every registered skin as `(id, title key)`, in the order they were added.
pub fn registered() -> Vec<(&'static str, &'static str)> {
    SKINS.with_borrow(|skins| skins.iter().map(|skin| (skin.id(), skin.title())).collect())
}

/// The id now drawing, which is `normal` until a registered skin is chosen.
pub fn active() -> &'static str {
    ACTIVE.get()
}

/// Draw with `id` from here on, resolving its metrics once; an unknown id is Normal.
pub fn select(id: &str) {
    SKINS.with_borrow(|skins| {
        let skin = skins
            .iter()
            .find(|skin| skin.id() == id)
            .or_else(|| skins.iter().find(|skin| skin.id() == "normal"));
        let (chosen, metrics, chrome, arrangement, assets, font, controls) = match skin {
            Some(skin) => (
                skin.id(),
                skin.metrics(),
                skin.chrome(),
                skin.arrangement(),
                skin.assets(),
                skin.font(),
                skin.controls(),
            ),
            None => (
                "normal",
                Metrics::default(),
                Chrome::default(),
                Arrangement::default(),
                Vec::new(),
                None,
                Controls::Kopuz,
            ),
        };
        ACTIVE.set(chosen);
        FONT.set(font);
        CONTROLS.set(controls);
        METRICS.set(metrics);
        CHROME.set(chrome);
        ARRANGEMENT.set(arrangement);
        ASSETS.with_borrow_mut(|held| *held = assets.into_iter().collect());
    });
}

/// The active skin's metrics; a copy out of a cell, cheap enough for a render to ask every time.
pub fn metrics() -> Metrics {
    METRICS.get()
}

/// The active skin's chrome proportions.
pub fn chrome() -> Chrome {
    CHROME.get()
}

/// How the active skin arranges its surfaces.
pub fn arrangement() -> Arrangement {
    ARRANGEMENT.get()
}

/// Where the navigation sits: a skin that sections it keeps it down the side whatever was chosen.
pub fn nav_position(chosen: NavPosition) -> NavPosition {
    match arrangement().nav {
        Nav::Sections => NavPosition::Left,
        Nav::Strip => chosen,
    }
}

/// How the active skin draws its controls.
pub fn controls() -> Controls {
    CONTROLS.get()
}

/// Whether controls are drawn to the Material 3 baseline.
pub fn material() -> bool {
    controls() == Controls::Material
}

/// The active skin's own face, if it has one.
pub fn font() -> Option<&'static str> {
    FONT.get()
}

/// What the active skin draws instead of `path`, if it swaps it at all.
pub fn asset(path: &str) -> Option<SharedString> {
    ASSETS.with_borrow(|held| {
        if held.is_empty() {
            return None;
        }
        held.get(path).cloned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tall;

    impl Skin for Tall {
        fn id(&self) -> &'static str {
            "tall"
        }

        fn title(&self) -> &'static str {
            "skin_tall"
        }

        fn metrics(&self) -> Metrics {
            Metrics {
                row_height: rems(5.),
                ..Metrics::default()
            }
        }

        fn assets(&self) -> Vec<(&'static str, SharedString)> {
            vec![("icons/play.svg", "icons/pause.svg".into())]
        }
    }

    // Pinned to the literals that sat in the crates before the registry gathered them.
    #[test]
    fn normal_measures_exactly_what_the_call_sites_used_to_hardcode() {
        let metrics = Metrics::default();
        assert_eq!(metrics.radius, rems(0.25));
        assert_eq!(metrics.radius_md, rems(0.375));
        assert_eq!(metrics.radius_lg, rems(0.5));
        assert_eq!(metrics.radius_xl, rems(0.75));
        assert_eq!(metrics.radius_2xl, rems(1.));
        assert_eq!(metrics.thumb_radius, px(6.));
        assert_eq!(metrics.cover_radius, px(8.));
        assert_eq!(metrics.row_height, rems(3.5));
        assert_eq!(metrics.row_cover, rems(2.5));
        assert_eq!(metrics.icon_button, rems(2.25));
        assert_eq!(metrics.media_row, px(60.));
        assert_eq!(metrics.list_row, px(34.));
        let chrome = Chrome::default();
        assert_eq!(chrome.bar_height, px(44.));
        assert_eq!(chrome.rail_width, px(200.));
        assert_eq!(chrome.search_height, px(30.));
        assert_eq!(chrome.player_height, px(96.));
        assert_eq!(chrome.tab_height, px(28.));
        assert_eq!(chrome.button, px(30.));
        assert_eq!(chrome.round_button, px(36.));
        assert_eq!(chrome.tool_button, px(28.));
    }

    #[test]
    fn normal_is_registered_before_anyone_asks() {
        assert_eq!(active(), "normal");
        assert_eq!(metrics(), Metrics::default());
        assert!(registered().iter().any(|(id, _)| *id == "normal"));
    }

    #[test]
    fn an_unknown_skin_falls_back_to_normal() {
        select("no-such-skin");
        assert_eq!(active(), "normal");
        assert_eq!(metrics().row_height, Metrics::default().row_height);
    }

    #[test]
    fn a_registered_skin_replaces_the_metrics_and_the_assets() {
        register(Box::new(Tall));
        select("tall");
        assert_eq!(active(), "tall");
        assert_eq!(metrics().row_height, rems(5.));
        assert_eq!(asset("icons/play.svg").as_deref(), Some("icons/pause.svg"));
        assert_eq!(asset("icons/heart.svg"), None);
        select("normal");
        assert_eq!(asset("icons/play.svg"), None);
    }

    #[test]
    fn the_user_picks_the_density_step_and_the_skin_says_what_it_measures() {
        let metrics = Metrics::default();
        assert_eq!(metrics.density_row(Density::Compact), px(32.));
        assert_eq!(metrics.density_row(Density::Comfortable), px(56.));
    }

    #[test]
    fn normal_arranges_exactly_what_the_call_sites_used_to_hardcode() {
        let arrangement = Arrangement::default();
        assert_eq!(arrangement.nav, Nav::Strip);
        assert_eq!(arrangement.bar, BarLayout::Split);
        assert_eq!(arrangement.rows.lead, px(20.));
        // `gap-6`, `p-2` and `mr-4`: rems, so Normal keeps its spacing at every interface scale.
        assert_eq!(arrangement.rows.gap, rems(1.5));
        assert_eq!(arrangement.rows.pad_y, rems(0.5));
        assert_eq!(arrangement.rows.cover_gap, rems(1.));
        assert_eq!(arrangement.rows.hover, 0.05);
        assert_eq!(arrangement.rows.number, 0.60);
        assert_eq!(arrangement.pages.padding, rems(2.));
        assert_eq!(arrangement.pages.title, rems(1.875));
        assert_eq!(arrangement.pages.hero, px(256.));
        assert_eq!(arrangement.pages.hero_title, rems(4.5));
        assert_eq!(arrangement.mini, Mini::default());
        assert_eq!(Normal.arrangement(), arrangement);
        assert_eq!(Material3.arrangement(), arrangement);
    }

    #[test]
    fn vaxry_answers_to_its_ui_style_and_keeps_the_sidebar_down_the_side() {
        select("vaxry");
        assert_eq!(active(), "vaxry");
        assert_eq!(metrics().row_height, rems(2.75));
        assert_eq!(metrics().density_row(Density::Comfortable), px(44.));
        assert_eq!(chrome().player_height, px(64.));
        assert_eq!(arrangement().bar, BarLayout::Inline);
        assert_eq!(nav_position(NavPosition::Top), NavPosition::Left);
        select("normal");
        assert_eq!(nav_position(NavPosition::Top), NavPosition::Top);
        assert_eq!(metrics().density_row(Density::Comfortable), px(56.));
    }

    #[test]
    fn a_page_wears_the_heading_of_the_section_it_sits_in() {
        assert_eq!(
            section_of(&Route::Album("a".into()), false),
            Some("library")
        );
        assert_eq!(section_of(&Route::Radio, false), Some("discover"));
        assert_eq!(
            section_of(&Route::Artist("a".into()), true),
            Some("discover")
        );
        assert_eq!(section_of(&Route::Downloader, false), None);
        assert_eq!(section_of(&Route::Settings { section: None }, false), None);
    }

    #[test]
    fn every_sidebar_entry_goes_somewhere_and_lights_itself_there() {
        for entry in SECTIONS
            .iter()
            .flat_map(|section| section.entries)
            .chain(TOOLS)
        {
            let route = route_of(entry).expect(entry);
            assert_eq!(entry_of(&route, false), *entry);
        }
    }
}
