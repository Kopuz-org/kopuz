//! Connectivity: everything configured per account rather than per source, as the daemon lists it.

use std::collections::HashMap;

use api::schema::FieldValue;
use api::{ConnectKind, IntegrationInfo, Table};
use components::format::{t, t_with, text};
use components::{FieldChanged, SchemaForm, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, WeakEntity, Window,
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
