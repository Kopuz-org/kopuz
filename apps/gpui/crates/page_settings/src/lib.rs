//! Settings: the category rail and the panel it shows; each category lives in its own crate.

use components::format::{t, t_with};
use components::marker::Along;
use components::{Bar, Callout, EmptyState, PageHeader, SearchField, Tab, TabStrip, icons, style};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, rems,
};
use settings_appearance::AppearanceSettings;
use settings_interface::InterfaceSettings;
use settings_playback::PlaybackSettings;
use settings_sources::SourceSettings;
use store::ConfigModel;

/// `border-white/9` under the fan, over `bg-black/92`.
const FAN_BORDER: f32 = 0.09;
const FAN_BG: f32 = 0.92;
/// `text-white/70` on the line under an empty panel.
const NOTICE: f32 = 0.70;

/// Which panel the rail is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    General,
    Appearance,
    Interface,
    Library,
    Connectivity,
    Offline,
    Metadata,
    Player,
    Tools,
}

impl Category {
    pub const ALL: &'static [Category] = &[
        Category::General,
        Category::Appearance,
        Category::Interface,
        Category::Library,
        Category::Connectivity,
        Category::Offline,
        Category::Metadata,
        Category::Player,
        Category::Tools,
    ];

    /// The `Route::Settings { section }` name, which is also how an anchor asks for a category.
    pub fn slug(self) -> &'static str {
        match self {
            Category::General => "general",
            Category::Appearance => "appearance",
            Category::Interface => "interface",
            Category::Library => "library",
            Category::Connectivity => "connectivity",
            Category::Offline => "offline",
            Category::Metadata => "metadata",
            Category::Player => "player",
            Category::Tools => "tools",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|category| category.slug() == slug)
    }

    fn label(self) -> SharedString {
        t(match self {
            Category::General => "settings_general",
            Category::Appearance => "settings_customization",
            Category::Interface => "settings_interface",
            Category::Library => "settings_library",
            Category::Connectivity => "settings_connectivity",
            Category::Offline => "settings_offline",
            Category::Metadata => "settings_metadata",
            Category::Player => "settings_player",
            Category::Tools => "settings_tools",
        })
    }

    fn icon(self) -> &'static str {
        match self {
            Category::General => icons::SETTINGS,
            Category::Appearance => icons::GRID,
            Category::Interface => icons::ROWS,
            Category::Library => icons::LIBRARY,
            Category::Connectivity => icons::SPEAKER,
            Category::Offline => icons::DOWNLOAD,
            Category::Metadata => icons::EDIT,
            Category::Player => icons::PLAY,
            Category::Tools => icons::WARNING,
        }
    }
}

pub struct Settings {
    category: Category,
    scroll: gpui_kit::ScrollHandle,
    search: Entity<InputState>,
    searching: bool,
    appearance: Entity<AppearanceSettings>,
    interface: Entity<InterfaceSettings>,
    sources: Entity<SourceSettings>,
    playback: Entity<PlaybackSettings>,
    _subscriptions: Vec<Subscription>,
}

impl Settings {
    pub fn new(section: Option<SharedString>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let category = section
            .and_then(|section| Category::from_slug(&section))
            .unwrap_or(Category::General);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("settings_search")));
        // The box is new, so whatever a previous visit was filtering by is not.
        components::settings::set_query("", cx);
        let subscriptions = vec![
            cx.subscribe(&search, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let typed = state.read(cx).value().to_string();
                    this.filter(&typed, cx);
                }
            }),
            cx.observe(&ConfigModel::global(cx), |_, _, cx| cx.notify()),
        ];
        let mut settings = Self {
            category,
            scroll: gpui_kit::ScrollHandle::new(),
            search,
            searching: false,
            appearance: cx.new(|cx| AppearanceSettings::new(window, cx)),
            interface: cx.new(|cx| InterfaceSettings::new(window, cx)),
            sources: cx.new(|cx| SourceSettings::new(window, cx)),
            playback: cx.new(|cx| PlaybackSettings::new(window, cx)),
            _subscriptions: subscriptions,
        };
        settings.select(category, cx);
        settings
    }

    /// Filtering spans every category, so the panels stop showing one section each while it lasts.
    fn filter(&mut self, typed: &str, cx: &mut Context<Self>) {
        components::settings::set_query(typed, cx);
        let searching = components::settings::query(cx).is_some();
        if searching == self.searching {
            cx.notify();
            return;
        }
        self.searching = searching;
        if searching {
            self.appearance.update(cx, |panel, cx| panel.show(None, cx));
            self.sources.update(cx, |panel, cx| panel.show(None, cx));
            self.playback.update(cx, |panel, cx| panel.show(None, cx));
            cx.notify();
        } else {
            self.select(self.category, cx);
        }
    }

    /// Show one category, telling its crate which of its sections to draw.
    fn select(&mut self, category: Category, cx: &mut Context<Self>) {
        self.category = category;
        let section = Some(SharedString::from(category.slug()));
        match category {
            Category::General | Category::Appearance => self
                .appearance
                .update(cx, |panel, cx| panel.show(section, cx)),
            // The Interface panel draws whatever registered itself; it has no sections to pick.
            Category::Interface => {}
            Category::Library | Category::Connectivity => {
                self.sources.update(cx, |panel, cx| panel.show(section, cx))
            }
            _ => self
                .playback
                .update(cx, |panel, cx| panel.show(section, cx)),
        }
        cx.notify();
    }

    /// A category rail entry was clicked; a search that was on is what it replaces.
    fn choose(&mut self, category: Category, window: &mut Window, cx: &mut Context<Self>) {
        if self.searching {
            self.search
                .update(cx, |state, cx| state.set_value("", window, cx));
            self.searching = false;
            components::settings::set_query("", cx);
        }
        self.select(category, cx);
    }

    fn panel(&self) -> AnyElement {
        if self.searching {
            return v_flex()
                .w_full()
                .gap_4()
                .child(self.appearance.clone())
                .child(self.interface.clone())
                .child(self.sources.clone())
                .child(self.playback.clone())
                .into_any_element();
        }
        match self.category {
            Category::Appearance | Category::General => self.appearance.clone().into_any_element(),
            Category::Interface => self.interface.clone().into_any_element(),
            Category::Library | Category::Connectivity => self.sources.clone().into_any_element(),
            Category::Offline | Category::Metadata | Category::Player | Category::Tools => {
                self.playback.clone().into_any_element()
            }
        }
    }
}

impl Settings {
    /// The category fan, as its top-bar branch: the CD disc is rotated clip-paths GPUI cannot draw.
    fn rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = Category::ALL.iter().map(|category| {
            Tab::new(category.slug(), category.icon()).label(Some(category.label()))
        });
        let strip = TabStrip::new("settings-fan", Along::Row, Bar::Rule)
            .roving("SettingsRail")
            .current(Some(self.category.slug()))
            .tabs(tabs)
            .on_select(cx.listener(|this, index: &usize, window, cx| {
                if let Some(category) = Category::ALL.get(*index).copied() {
                    this.choose(category, window, cx);
                }
            }));
        h_flex()
            .id("settings-fan-scroll")
            .w_full()
            .flex_none()
            .mb_4()
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .border_b_1()
            .border_color(style::white(FAN_BORDER, cx))
            .bg(components::settings::panel_background(FAN_BG, cx))
            .child(strip)
    }

    /// The box that filters every category at once.
    fn search_box(&self) -> impl IntoElement {
        div()
            .w_full()
            .mb_4()
            .child(SearchField::new(&self.search).cleanable(true))
    }

    /// Nothing to edit until the daemon has answered; dead controls would save nothing.
    fn unavailable(&self, cx: &App) -> impl IntoElement {
        v_flex()
            .w_full()
            .py_8()
            .child(EmptyState::new(t("settings_unavailable")).icon(icons::WARNING))
            .child(
                div()
                    .w_full()
                    .text_center()
                    .text_xs()
                    .text_color(style::white(NOTICE, cx))
                    .child(t("settings_unavailable_hint")),
            )
    }

    /// One notice per kind of layer that pins a setting, above the fan.
    fn managed_notice(&self, locks: &[api::SettingLock]) -> impl IntoElement {
        v_flex().w_full().mb_4().gap_2().children(
            lock_notices(locks)
                .into_iter()
                .map(|notice| Callout::new(t(notice)).icon(icons::LOCK)),
        )
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        components::scroll::smooth("settings-panel", &self.scroll, window, cx);
        let config = ConfigModel::global(cx);
        let loaded = config.read(cx).is_loaded();
        let locks = config.read(cx).locks().to_vec();
        let empty = self.searching && components::settings::nothing_matched(cx);
        let typed = SharedString::from(self.search.read(cx).value().to_string());
        // The page root is the scroller: a nested `flex_1 min_h_0` child measured no height here.
        v_flex()
            .id("settings-panel")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                v_flex()
                    .w_full()
                    .max_w(rems(80.))
                    .mx_auto()
                    .px_6()
                    .py_7()
                    .child(PageHeader::new(t("settings")))
                    .map(|page| {
                        if !loaded {
                            return page.child(self.unavailable(cx));
                        }
                        page.child(self.search_box())
                            .when(!locks.is_empty(), |page| {
                                page.child(self.managed_notice(&locks))
                            })
                            .child(self.rail(cx))
                            .when(empty, |page| {
                                page.child(EmptyState::new(t_with(
                                    "no_results_found",
                                    &[("query", typed.to_string().into())],
                                )))
                            })
                            .child(self.panel())
                    }),
            )
    }
}

/// The notice keys for each kind of layer that pins a setting, so the banner names only what does.
fn lock_notices(locks: &[api::SettingLock]) -> Vec<&'static str> {
    let pinned_by = |kind: fn(&api::LockSource) -> bool| locks.iter().any(|lock| kind(&lock.by));
    [
        (
            pinned_by(|by| matches!(by, api::LockSource::Managed(_))),
            "settings_managed_notice",
        ),
        (
            pinned_by(|by| matches!(by, api::LockSource::DropIn(_))),
            "settings_dropin_notice",
        ),
        (
            pinned_by(|by| matches!(by, api::LockSource::Env(_))),
            "settings_env_notice",
        ),
    ]
    .into_iter()
    .filter_map(|(pinned, notice)| pinned.then_some(notice))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::lock_notices;

    fn lock(by: api::LockSource) -> api::SettingLock {
        api::SettingLock {
            key: config::SettingKey::Ui(config::UiKey::Theme),
            by,
        }
    }

    #[test]
    fn the_banner_names_only_the_layers_that_pin() {
        let dropin = lock(api::LockSource::DropIn("/x/settings.d/30-c.toml".into()));
        let env = lock(api::LockSource::Env("KOPUZ_CONFIG_THEME".into()));
        assert!(lock_notices(&[]).is_empty());
        assert_eq!(
            lock_notices(std::slice::from_ref(&dropin)),
            vec!["settings_dropin_notice"]
        );
        assert_eq!(
            lock_notices(&[env.clone(), dropin, env]),
            vec!["settings_dropin_notice", "settings_env_notice"]
        );
    }
}
