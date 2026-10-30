//! The custom theme editor: the saved list, thirteen color rows, a preview card, save and delete.

use std::collections::HashMap;

use components::format::t;
use components::{ColorChanged, ColorField, Panel, ThemePreview, icons, style};
use config::{CustomTheme, CustomThemeOp, SettingsEdit, UiEdit};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Selectable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, Window, rems,
};
use store::ConfigModel;

/// The thirteen palette tokens, in the order the editor lists them.
pub const VAR_KEYS: &[&str] = &[
    "bg",
    "raised",
    "surface",
    "text",
    "text-muted",
    "accent",
    "accent-soft",
    "accent-alt",
    "accent-deep",
    "highlight",
    "highlight-dark",
    "progress",
    "danger",
];

const DEFAULT_VARS: &[(&str, &str)] = &[
    ("bg", "#0f0f17"),
    ("raised", "#1a1a2a"),
    ("surface", "#282838"),
    ("text", "#e2e2f0"),
    ("text-muted", "#7878a0"),
    ("accent", "#5f8aff"),
    ("accent-soft", "#8faeff"),
    ("accent-alt", "#3a5fd9"),
    ("accent-deep", "#0a0a1a"),
    ("highlight", "#c77dff"),
    ("highlight-dark", "#9d4edd"),
    ("progress", "#5f8aff"),
    ("danger", "#ff6b6b"),
];

fn default_vars() -> HashMap<String, String> {
    DEFAULT_VARS
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}
/// `custom-<slug>`, with `-n` appended until the id is free.
fn theme_id(name: &str, taken: &HashMap<String, CustomTheme>) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = format!("custom-{}", slug.trim_matches('-'));
    if !taken.contains_key(&base) {
        return base;
    }
    (1u32..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.contains_key(candidate))
        .unwrap_or(base)
}

pub struct ThemeEditor {
    selected: Option<String>,
    name: Entity<InputState>,
    rows: Vec<(&'static str, Entity<ColorField>)>,
    vars: HashMap<String, String>,
    // What was last loaded or saved; anything else on screen is an edit nobody has kept yet.
    baseline: (String, HashMap<String, String>),
    _subscriptions: Vec<Subscription>,
}

impl ThemeEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let vars = default_vars();
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("my_custom_theme")));
        let mut subscriptions = vec![cx.observe(&ConfigModel::global(cx), |_, _, cx| cx.notify())];
        let mut rows = Vec::new();
        for key in VAR_KEYS {
            let value = vars.get(*key).cloned().unwrap_or_default();
            let field = cx.new(|cx| ColorField::new(t(key), &value, window, cx));
            subscriptions.push(
                cx.subscribe(&field, move |this, _, event: &ColorChanged, cx| {
                    this.set_var(key, event.0.clone(), cx)
                }),
            );
            rows.push((*key, field));
        }
        Self {
            selected: None,
            name,
            rows,
            baseline: (String::new(), vars.clone()),
            vars,
            _subscriptions: subscriptions,
        }
    }

    /// Whether the editor holds edits the saved theme does not; the dot and the prompt ask this.
    pub(crate) fn dirty(&self, cx: &Context<Self>) -> bool {
        self.name.read(cx).value().trim() != self.baseline.0 || self.vars != self.baseline.1
    }

    /// Leaving a theme mid-edit asks first; saying yes is what actually swaps it.
    fn choose(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dirty(cx) {
            self.load(id, window, cx);
            return;
        }
        let editor = cx.entity().downgrade();
        components::dialogs::confirm(
            components::dialogs::ConfirmKind::Discard,
            t("theme_unsaved_title"),
            t("theme_unsaved_body"),
            move |window, cx| {
                let id = id.clone();
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |this, cx| this.load(id, window, cx));
                }
            },
            window,
            cx,
        );
    }

    fn set_var(&mut self, key: &str, value: String, cx: &mut Context<Self>) {
        if self.vars.get(key).is_some_and(|held| held == &value) {
            return;
        }
        self.vars.insert(key.to_string(), value);
        cx.notify();
    }

    fn load(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let (name, vars) = match id.as_ref() {
            Some(id) => ConfigModel::global(cx)
                .read(cx)
                .config()
                .ui
                .custom_themes
                .get(id)
                .map(|theme| (theme.name.clone(), theme.vars.clone()))
                .unwrap_or_else(|| (String::new(), default_vars())),
            None => (String::new(), default_vars()),
        };
        self.selected = id;
        let input = self.name.clone();
        input.update(cx, |state, cx| state.set_value(name, window, cx));
        for (key, field) in &self.rows {
            let value = vars
                .get(*key)
                .cloned()
                .or_else(|| default_vars().get(*key).cloned())
                .unwrap_or_default();
            self.vars.insert(key.to_string(), value.clone());
            field.update(cx, |field, cx| field.set(&value, window, cx));
        }
        self.baseline = (
            self.name.read(cx).value().trim().to_string(),
            self.vars.clone(),
        );
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        let config = ConfigModel::global(cx);
        // Saving before the daemon's config arrived would be dropped, taking the edits with it.
        if name.is_empty() || !config.read(cx).is_loaded() {
            return;
        }
        let vars = self.vars.clone();
        let id = self
            .selected
            .clone()
            .unwrap_or_else(|| theme_id(&name, &config.read(cx).config().ui.custom_themes));
        let saved = id.clone();
        ConfigModel::edit(
            cx,
            SettingsEdit::Ui(UiEdit::CustomThemes(CustomThemeOp::Put {
                name: saved,
                theme: CustomTheme { name, vars },
            })),
        );
        self.load(Some(id), window, cx);
    }

    /// What the selected theme is called, for the confirmation that names what goes.
    fn editing_name(&self, cx: &Context<Self>) -> SharedString {
        let Some(id) = self.selected.as_ref() else {
            return t("delete");
        };
        ConfigModel::global(cx)
            .read(cx)
            .config()
            .ui
            .custom_themes
            .get(id)
            .map(|theme| SharedString::from(theme.name.clone()))
            .unwrap_or_else(|| SharedString::from(id.clone()))
    }

    /// Reached only from the confirmation; deleting the theme in use puts the app back on the default.
    fn delete_confirmed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let in_use = ConfigModel::global(cx).read(cx).config().ui.theme == id;
        ConfigModel::edit(
            cx,
            SettingsEdit::Ui(UiEdit::CustomThemes(CustomThemeOp::Delete {
                name: id.clone(),
            })),
        );
        if in_use {
            ConfigModel::edit(cx, SettingsEdit::Ui(UiEdit::Theme("default".to_string())));
        }
        self.load(None, window, cx);
    }

    fn saved_themes(&self, cx: &Context<Self>) -> Vec<(String, String)> {
        let mut themes: Vec<(String, String)> = ConfigModel::global(cx)
            .read(cx)
            .config()
            .ui
            .custom_themes
            .iter()
            .map(|(id, theme)| (id.clone(), theme.name.clone()))
            .collect();
        themes.sort_by(|left, right| left.1.cmp(&right.1));
        themes
    }
}

impl Render for ThemeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let saved = self.saved_themes(cx);
        let selected = self.selected.clone();
        let danger = cx.theme().danger;
        let dirty = self.dirty(cx);
        // The original laid the thirteen colors out in a two-column grid; GPUI has no grid.
        let columns = |parity: usize| {
            self.rows
                .iter()
                .enumerate()
                .filter(move |(index, _)| index % 2 == parity)
                .map(|(_, (_, field))| field.clone())
        };
        h_flex()
            .w_full()
            .gap_6()
            .items_start()
            .child(
                v_flex()
                    .w(rems(13.))
                    .flex_none()
                    .gap_2()
                    .child(
                        Button::new("new-theme")
                            .custom(style::soft_button(cx.theme().foreground, cx))
                            .w_full()
                            .justify_start()
                            .icon(icons::icon(icons::PLUS))
                            .label(t("new_theme"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.choose(None, window, cx)),
                            ),
                    )
                    .child(v_flex().w_full().gap_1().children(saved.into_iter().map(
                        |(id, name)| {
                            let chosen = selected.as_deref() == Some(id.as_str());
                            let target = id.clone();
                            Button::new(SharedString::from(format!("theme-{id}")))
                                .custom(style::pick_button(chosen, cx))
                                .w_full()
                                .justify_start()
                                .label(SharedString::from(name))
                                .selected(chosen)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.choose(Some(target.clone()), window, cx)
                                }))
                        },
                    ))),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_5()
                    .child(
                        Panel::new(t("theme_name"))
                            .dirty(dirty)
                            .child(Input::new(&self.name)),
                    )
                    .child(
                        Panel::new(t("colors")).dirty(dirty).child(
                            h_flex()
                                .w_full()
                                .gap_10()
                                .items_start()
                                .child(v_flex().flex_1().min_w_0().gap_3().children(columns(0)))
                                .child(v_flex().flex_1().min_w_0().gap_3().children(columns(1))),
                        ),
                    )
                    .child(
                        Panel::new(t("preview"))
                            .dirty(dirty)
                            .child(ThemePreview::new(self.vars.clone())),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Button::new("save-theme")
                                    .primary()
                                    .label(t("save_theme"))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            )
                            .when(self.selected.is_some(), |actions| {
                                actions.child(
                                    Button::new("delete-theme")
                                        .custom(style::tint_button(danger, cx))
                                        .icon(icons::icon(icons::TRASH))
                                        .label(t("delete"))
                                        // Deleting the theme in use also resets you to default.
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let name = this.editing_name(cx);
                                            let entity = cx.entity();
                                            components::dialogs::confirm(
                                                components::dialogs::ConfirmKind::Delete,
                                                t("delete"),
                                                name,
                                                move |window, cx| {
                                                    entity.update(cx, |this, cx| {
                                                        this.delete_confirmed(window, cx)
                                                    });
                                                },
                                                window,
                                                cx,
                                            );
                                        })),
                                )
                            }),
                    ),
            )
    }
}
