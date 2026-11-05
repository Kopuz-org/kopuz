//! Connectivity: everything configured per account rather than per source, as the daemon lists it.

use std::collections::{HashMap, HashSet};

use api::schema::FieldValue;
use api::{BrowserProfile, ConnectKind, IntegrationInfo, Table};
use components::format::{t, t_with, text};
use components::settings::Choice;
use components::{FieldChanged, SchemaForm, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, WeakEntity, Window, div,
};
use link::Link;
use store::{ConfigModel, Notice, Notices, Query};

use crate::ui;

/// Answer one integration's fields; a refusal puts the form back. The value is never logged.
fn write_settings(
    id: String,
    value: FieldValue,
    query: Entity<Query<Vec<IntegrationInfo>>>,
    form: WeakEntity<SchemaForm>,
    window: &mut Window,
    cx: &mut App,
) {
    let key = value.key.clone();
    let call = Link::global(cx)
        .call(move |api| async move { api.set_integration_settings(id, vec![value]).await });
    window
        .spawn(cx, async move |cx| match call.await {
            Ok(_) => {
                cx.update(|_, cx| query.update(cx, |query, cx| query.refresh(cx)))
                    .ok();
            }
            Err(error) => {
                form.update_in(cx, |form, window, cx| {
                    form.revert(&key, window, cx);
                    Notices::failed(&error, cx);
                })
                .ok();
            }
        })
        .detach();
}

struct IntegrationForm {
    form: Entity<SchemaForm>,
    _subscription: Subscription,
}

pub struct Integrations {
    query: Entity<Query<Vec<IntegrationInfo>>>,
    forms: HashMap<String, IntegrationForm>,
    /// Read only once an integration connects from a browser, since it walks the host's profiles.
    profiles: Option<(Entity<Query<Vec<BrowserProfile>>>, Subscription)>,
    /// The profile picked per integration, before Connect is pressed.
    picked: HashMap<String, String>,
    connecting: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl Integrations {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = ConfigModel::global(cx);
        let query = cx.new(|cx| {
            Query::new(
                &[Table::Servers],
                |api| async move { api.integrations().await },
                cx,
            )
        });
        let subscriptions = vec![
            cx.observe_in(&query, window, |this, _, window, cx| {
                this.sync_forms(window, cx)
            }),
            cx.observe_in(&config, window, |this, _, window, cx| {
                this.sync_forms(window, cx)
            }),
        ];
        // The listing may already be cached, in which case no observer would ever fire.
        cx.defer_in(window, |this, window, cx| this.sync_forms(window, cx));
        Self {
            query,
            forms: HashMap::new(),
            profiles: None,
            picked: HashMap::new(),
            connecting: HashSet::new(),
            _subscriptions: subscriptions,
        }
    }

    fn listed(&self, cx: &App) -> Vec<IntegrationInfo> {
        self.query.read(cx).value().cloned().unwrap_or_default()
    }

    /// One form per integration, rebuilt only when the daemon's fields change under it.
    fn sync_forms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let locked = ConfigModel::global(cx).read(cx).locked_names();
        let listed = self.listed(cx);
        let from_browser = listed
            .iter()
            .any(|integration| integration.connect == ConnectKind::BrowserProfile);
        if from_browser && self.profiles.is_none() {
            let profiles =
                cx.new(|cx| Query::new(&[], |api| async move { api.browser_profiles().await }, cx));
            let subscription = cx.observe(&profiles, |_, _, cx| cx.notify());
            self.profiles = Some((profiles, subscription));
        }
        self.forms
            .retain(|id, _| listed.iter().any(|integration| &integration.id == id));
        for integration in listed {
            let specs = integration.fields.clone();
            let held = self.forms.entry(integration.id.clone()).or_insert_with(|| {
                let form = cx.new(|cx| SchemaForm::new(specs.clone(), window, cx));
                let id = integration.id.clone();
                let query = self.query.clone();
                let subscription = cx.subscribe_in(
                    &form,
                    window,
                    move |_, form, event: &FieldChanged, window, cx| {
                        write_settings(
                            id.clone(),
                            event.0.clone(),
                            query.clone(),
                            form.downgrade(),
                            window,
                            cx,
                        );
                    },
                );
                IntegrationForm {
                    form,
                    _subscription: subscription,
                }
            });
            let (locked, configured) = (locked.clone(), integration.configured);
            held.form.update(cx, |form, cx| {
                form.sync(specs, locked, configured, window, cx)
            });
        }
        cx.notify();
    }

    fn connect(&mut self, id: String, cx: &mut Context<Self>) {
        let call =
            Link::global(cx).call(move |api| async move { api.authenticate_integration(id).await });
        let query = self.query.clone();
        cx.spawn(async move |_, cx| match call.await {
            Ok(_) => {
                cx.update(|cx| query.update(cx, |query, cx| query.refresh(cx)));
            }
            Err(error) => {
                cx.update(|cx| {
                    let reason = t_with(
                        "signin_failed",
                        &[("error", store::human(&error).to_string().into())],
                    );
                    Notices::push(Notice::failure(&error, reason), cx);
                });
            }
        })
        .detach();
    }

    fn connect_from_browser(&mut self, id: String, profile: String, cx: &mut Context<Self>) {
        self.connecting.insert(id.clone());
        cx.notify();
        let query = self.query.clone();
        let call = Link::global(cx).call({
            let id = id.clone();
            move |api| async move { api.connect_integration_from_browser(id, profile).await }
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.connecting.remove(&id);
                match result {
                    Ok(_) => query.update(cx, |query, cx| query.refresh(cx)),
                    Err(error) => {
                        tracing::warn!(%error, %id, "connecting an integration from a browser failed");
                        Notices::failed(&error, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Connect from a browser profile already signed in to the service, beside the fields that take a pasted credential.
    fn browser_row(
        &self,
        id: &str,
        name: &SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let profiles = self
            .profiles
            .as_ref()
            .and_then(|(query, _)| query.read(cx).value().cloned())
            .unwrap_or_default();
        let selected = profiles
            .iter()
            .find(|profile| self.picked.get(id) == Some(&profile.id))
            .or(profiles.first())
            .map(|profile| profile.id.clone())
            .unwrap_or_default();
        let busy = self.connecting.contains(id);
        let controls = if profiles.is_empty() {
            ui::hint(t("integration_browser_none"), cx).into_any_element()
        } else {
            let options = profiles
                .iter()
                .map(|profile| {
                    let label = t_with(
                        "integration_browser_profile",
                        &[
                            ("name", profile.name.clone().into()),
                            ("browser", profile.browser.clone().into()),
                        ],
                    );
                    (profile.id.clone(), label)
                })
                .collect();
            let this = cx.entity().downgrade();
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Choice::new(
                        SharedString::from(format!("browser-profile-{id}")),
                        selected.clone(),
                        options,
                    )
                    .on_change({
                        let id = id.to_string();
                        move |profile: String, cx| {
                            this.update(cx, |this, cx| {
                                this.picked.insert(id.clone(), profile);
                                cx.notify();
                            })
                            .ok();
                        }
                    }),
                )
                .child(
                    Button::new(SharedString::from(format!("connect-browser-{id}")))
                        .custom(style::soft_button(cx.theme().foreground, cx))
                        .small()
                        .label(match busy {
                            true => t("integration_connecting"),
                            false => {
                                t_with("integration_connect", &[("name", name.to_string().into())])
                            }
                        })
                        .disabled(busy)
                        .on_click(cx.listener({
                            let id = id.to_string();
                            move |this: &mut Self, _, _, cx| {
                                this.connect_from_browser(id.clone(), selected.clone(), cx)
                            }
                        })),
                )
                .into_any_element()
        };
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_3()
            .justify_between()
            .items_center()
            .child(div().text_sm().child(t("integration_browser_title")))
            .child(controls)
    }

    fn integration(
        &self,
        integration: &IntegrationInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = integration.id.clone();
        let name = text(&integration.name);
        let configured = integration.configured;
        let success = cx.theme().success;
        v_flex()
            .w_full()
            .gap_2()
            .child(ui::label(name.clone()))
            .children(self.forms.get(&id).map(|held| held.form.clone()))
            .when(integration.connect == ConnectKind::WebSignIn, |row| {
                row.child(
                    h_flex().child(
                        Button::new(SharedString::from(format!("connect-{id}")))
                            .small()
                            .map(|button| {
                                if configured {
                                    button
                                        .custom(style::tint_button(success, cx))
                                        .bg(success.opacity(style::ACTIVE))
                                } else {
                                    button.custom(style::soft_button(cx.theme().foreground, cx))
                                }
                            })
                            .label(if configured {
                                t_with(
                                    "integration_connected",
                                    &[("name", name.to_string().into())],
                                )
                            } else {
                                t_with("integration_connect", &[("name", name.to_string().into())])
                            })
                            .disabled(configured)
                            .on_click(cx.listener({
                                let id = id.clone();
                                move |this: &mut Self, _, _, cx| this.connect(id.clone(), cx)
                            })),
                    ),
                )
            })
            .when(integration.connect == ConnectKind::BrowserProfile, |row| {
                row.child(self.browser_row(&id, &name, cx))
            })
    }
}

impl Render for Integrations {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let listed = self.listed(cx);
        ui::section(t("connectivity")).opaque().child(
            ui::card(cx).children(
                listed
                    .iter()
                    .map(|integration| self.integration(integration, cx).into_any_element()),
            ),
        )
    }
}
