//! Radio registries: the station lists the radio page reads, enabled one by one.

use components::dialogs::{DialogSize, FormDialog};
use components::format::{t, t_with};
use components::{CheckRow, Disclosure, EmptyState, ErrorBanner};
use components::{icons, style};
use config::{IntegrationsEdit, RadioRegistryOp, RegistryEntry, SettingsEdit};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, Window,
};
use link::Link;
use store::ConfigModel;

use crate::ui;

/// Change the stored registries; the config model debounces the write.
fn edit_registry(cx: &mut App, op: RadioRegistryOp) {
    ConfigModel::edit(
        cx,
        SettingsEdit::Integrations(IntegrationsEdit::RadioRegistries(op)),
    );
}

fn set_enabled(cx: &mut App, url: &str, enabled: bool) {
    let held = ConfigModel::global(cx)
        .read(cx)
        .config()
        .integrations
        .radio_registries
        .iter()
        .find(|entry| entry.url == url)
        .cloned();
    if let Some(entry) = held {
        edit_registry(cx, RadioRegistryOp::Put(RegistryEntry { enabled, ..entry }));
    }
}

pub struct RadioRegistries {
    expanded: bool,
    error: Option<SharedString>,
    add: Entity<AddRegistry>,
    _subscription: Subscription,
}

impl RadioRegistries {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = ConfigModel::global(cx);
        let add = cx.new(|cx| AddRegistry::new(window, cx));
        Self {
            expanded: false,
            error: None,
            add,
            _subscription: cx.observe(&config, |_, _, cx| cx.notify()),
        }
    }

    fn entries(cx: &App) -> Vec<RegistryEntry> {
        ConfigModel::global(cx)
            .read(cx)
            .config()
            .integrations
            .radio_registries
            .clone()
    }

    fn locked(cx: &App) -> bool {
        ConfigModel::global(cx)
            .read(cx)
            .is_locked("radio_registries")
    }

    /// Turning one on means proving it answers first; turning one off is just a write.
    fn toggle(&mut self, url: String, enable: bool, cx: &mut Context<Self>) {
        self.error = None;
        if !enable {
            set_enabled(cx, &url, false);
            cx.notify();
            return;
        }
        let call = Link::global(cx).call({
            let url = url.clone();
            move |api| async move { api.validate_radio_registry(url).await }
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => set_enabled(cx, &url, true),
                    Err(error) => {
                        this.error = Some(t_with(
                            "radio_registry_enable_failed",
                            &[("error", store::human(&error).to_string().into())],
                        ));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn delete(&mut self, url: String, cx: &mut Context<Self>) {
        edit_registry(cx, RadioRegistryOp::Remove { url });
        cx.notify();
    }

    fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add.update(cx, |form, cx| form.reset(window, cx));
        let form = self.add.clone();
        FormDialog::new(t("add_radio_registry"))
            .size(DialogSize::Wide)
            .on_submit(move |window, cx| form.update(cx, |form, cx| form.save(window, cx)))
            .open(self.add.clone(), window, cx);
        self.add.update(cx, |form, cx| form.focus(window, cx));
    }

    fn entry(&self, entry: &RegistryEntry, locked: bool, cx: &mut Context<Self>) -> CheckRow {
        let url = entry.url.clone();
        let shown = if entry.is_default {
            t("radio_default_registry")
        } else {
            SharedString::from(url.clone())
        };
        CheckRow::new(format!("registry-{url}"), shown)
            .checked(entry.enabled)
            .locked(locked)
            .on_toggle(cx.listener({
                let url = url.clone();
                move |this: &mut Self, checked: &bool, _, cx| this.toggle(url.clone(), *checked, cx)
            }))
            .when(!entry.is_default, |row| {
                row.trailing(
                    Button::new(SharedString::from(format!("delete-registry-{url}")))
                        .custom(style::tint_text_button(cx.theme().danger, cx))
                        .xsmall()
                        .label(t("delete"))
                        .disabled(locked)
                        .on_click(cx.listener({
                            let url = url.clone();
                            move |this: &mut Self, _, _, cx| this.delete(url.clone(), cx)
                        })),
                )
            })
    }
}

impl Render for RadioRegistries {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = Self::entries(cx);
        let locked = Self::locked(cx);
        let active = entries.iter().filter(|entry| entry.enabled).count();
        let summary = t_with(
            "radio_registries_active",
            &[
                ("enabled_count", active.into()),
                ("total", entries.len().into()),
            ],
        );
        let expanded = self.expanded;

        ui::section(t("radio")).opaque().child(
            ui::card(cx).child(
                Disclosure::new("registries-toggle", summary)
                    .open(expanded)
                    .on_toggle(cx.listener(|this, _, _, cx| {
                        this.expanded = !this.expanded;
                        cx.notify();
                    }))
                    .children(
                        entries
                            .is_empty()
                            .then(|| EmptyState::new(t("radio_registries_empty")).compact()),
                    )
                    .children(self.error.clone().map(ErrorBanner::new))
                    .children(
                        entries
                            .iter()
                            .map(|entry| self.entry(entry, locked, cx))
                            .collect::<Vec<_>>(),
                    )
                    .child(
                        Button::new("add-registry")
                            .custom(style::soft_button(cx.theme().foreground, cx))
                            .small()
                            .icon(icons::icon(icons::PLUS))
                            .label(t("add"))
                            .disabled(locked)
                            .on_click(cx.listener(|this, _, window, cx| this.open_add(window, cx)))
                            .self_start(),
                    ),
            ),
        )
    }
}

/// The "add radio registry" dialog: one URL, proved before it is kept.
pub struct AddRegistry {
    url: Entity<InputState>,
    error: Option<SharedString>,
    saving: bool,
}

impl AddRegistry {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            url: cx.new(|cx| {
                InputState::new(window, cx).placeholder(t("radio_registry_url_placeholder"))
            }),
            error: None,
            saving: false,
        }
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.url
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.error = None;
        self.saving = false;
        cx.notify();
    }

    /// Opening a dialog focuses the dialog itself, so this runs after it is open.
    fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.url.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Closes the dialog itself on success and keeps it open with the reason otherwise.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let url = self.url.read(cx).value().trim().to_string();
        if url.is_empty() {
            self.error = Some(t("radio_registry_empty_path"));
            cx.notify();
            return;
        }
        let taken = ConfigModel::global(cx)
            .read(cx)
            .config()
            .integrations
            .radio_registries
            .iter()
            .any(|entry| entry.url == url);
        if taken {
            self.error = Some(t("radio_registry_exists"));
            cx.notify();
            return;
        }
        self.error = None;
        self.saving = true;
        cx.notify();
        let call = Link::global(cx).call({
            let url = url.clone();
            move |api| async move { api.validate_radio_registry(url).await }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                cx.notify();
            })
            .ok();
            match result {
                Ok(_) => {
                    cx.update(|window, cx| {
                        let listed = ConfigModel::global(cx)
                            .read(cx)
                            .config()
                            .integrations
                            .radio_registries
                            .iter()
                            .any(|entry| entry.url == url);
                        if !listed {
                            edit_registry(
                                cx,
                                RadioRegistryOp::Put(RegistryEntry {
                                    url,
                                    enabled: true,
                                    is_default: false,
                                }),
                            );
                        }
                        window.close_dialog(cx);
                    })
                    .ok();
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.error = Some(t_with(
                            "radio_registry_import_failed",
                            &[("error", store::human(&error).to_string().into())],
                        ));
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }
}

impl Render for AddRegistry {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let saving = self.saving;
        v_flex()
            .w_full()
            .gap_3()
            .child(Input::new(&self.url).disabled(saving))
            .when_some(self.error.clone(), |form, message| {
                form.child(ErrorBanner::new(message))
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("cancel-add-registry")
                            .outline()
                            .label(t("cancel"))
                            .disabled(saving)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("save-add-registry")
                            .primary()
                            .label(if saving { t("saving") } else { t("save") })
                            .loading(saving)
                            .disabled(saving)
                            .on_click(
                                cx.listener(|this: &mut Self, _, window, cx| this.save(window, cx)),
                            ),
                    ),
            )
    }
}
