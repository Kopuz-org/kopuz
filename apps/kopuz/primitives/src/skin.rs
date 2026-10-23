//! What a skin varies: the metrics and chrome proportions every surface measures itself by.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gpui_kit::{Pixels, Rems, SharedString, px, rems};
use store::layout::Density;

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
    /// The id `layout.skin` stores.
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

thread_local! {
    static SKINS: RefCell<Vec<Box<dyn Skin>>> = RefCell::new(vec![Box::new(Normal), Box::new(Material3)]);
    static FONT: Cell<Option<&'static str>> = const { Cell::new(None) };
    static CONTROLS: Cell<Controls> = const { Cell::new(Controls::Kopuz) };
    static ACTIVE: Cell<&'static str> = const { Cell::new("normal") };
    static METRICS: Cell<Metrics> = Cell::new(Metrics::default());
    static CHROME: Cell<Chrome> = Cell::new(Chrome::default());
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
        let (chosen, metrics, chrome, assets, font, controls) = match skin {
            Some(skin) => (
                skin.id(),
                skin.metrics(),
                skin.chrome(),
                skin.assets(),
                skin.font(),
                skin.controls(),
            ),
            None => (
                "normal",
                Metrics::default(),
                Chrome::default(),
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
}
