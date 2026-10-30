//! Plugins: the ones the daemon runs, and the community marketplace it installs more from.

use api::{PluginCatalog, PluginInfo, PluginOrigin, Table};
use components::format::{t, t_with};
use components::{Badge, BadgeTone, EmptyState, Heading, SectionHeading};
use components::{icons, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Task, Window, div,
};
use link::Link;
use store::{Invalidation, Notices, TablesChanged};

use crate::ui;

pub struct Plugins {
    catalog: Option<PluginCatalog>,
    /// The plugin an install or removal is running for; one at a time.
    busy: Option<String>,
    _fetch: Option<Task<()>>,
    _subscription: Subscription,
}

impl Plugins {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        // Plugins swapped in from the plugin directory arrive as a change to the plugins table.
        let subscription = cx.subscribe(
            &Invalidation::global(cx),
            |this, _, event: &TablesChanged, cx| {
                if event.touches(&[Table::Plugins, Table::Servers]) {
                    this.refetch(cx);
                }
            },
        );
        let mut this = Self {
            catalog: None,
            busy: None,
            _fetch: None,
            _subscription: subscription,
        };
        this.refetch(cx);
        this
    }

    fn refetch(&mut self, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.plugins().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let catalog = call
                .await
                .inspect_err(|error| tracing::warn!(%error, "reading the plugins failed"))
                .ok();
            this.update(cx, |this, cx| {
                this.catalog = catalog;
                cx.notify();
            })
            .ok();
        }));
    }

    /// `network` is the hosts shown beside the button; the daemon refuses a listing that changed since.
    fn install(&mut self, id: String, network: Vec<String>, cx: &mut Context<Self>) {
        self.busy = Some(id.clone());
        cx.notify();
        let call = Link::global(cx).call({
            let id = id.clone();
            move |api| async move { api.install_plugin(id, network).await }
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    tracing::warn!(%error, %id, "installing a plugin failed");
                    Notices::failed(&error, cx);
                }
                this.busy = None;
                this.refetch(cx);
            })
            .ok();
        })
        .detach();
    }

    fn uninstall(&mut self, id: String, cx: &mut Context<Self>) {
        self.busy = Some(id.clone());
        cx.notify();
        let call = Link::global(cx).call({
            let id = id.clone();
            move |api| async move { api.uninstall_plugin(id).await }
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    tracing::warn!(%error, %id, "removing a plugin failed");
                    Notices::failed(&error, cx);
                }
                this.busy = None;
                this.refetch(cx);
            })
            .ok();
        })
        .detach();
    }

    fn row(&self, plugin: &PluginInfo, cx: &mut Context<Self>) -> AnyElement {
        let community = plugin.origin == PluginOrigin::Community;
        let working = self.busy.as_deref() == Some(plugin.id.as_str());
        let idle = self.busy.is_none();
        let version = plugin
            .installed_version
            .clone()
            .or_else(|| plugin.available_version.clone())
            .unwrap_or_default();
        let origin = if community {
            t("plugin_community")
        } else {
            t("plugin_official")
        };
        let reaches = (!plugin.network.is_empty()).then(|| {
            t_with(
                "plugin_reaches",
                &[("hosts", hosts(&plugin.network).into())],
            )
        });
        let update_reaches = plugin
            .update
            .as_ref()
            .filter(|update| !update.added_hosts.is_empty())
            .map(|update| {
                t_with(
                    "plugin_update_reaches",
                    &[("hosts", hosts(&update.added_hosts).into())],
                )
            });
        let byline = (!plugin.author.is_empty())
            .then(|| t_with("plugin_by", &[("author", plugin.author.clone().into())]));

        let mut actions = h_flex().flex_shrink_0().gap_2();
        let id = plugin.id.clone();
        // An update waiting for its hosts can replace an official plugin too; only removal is community-only.
        if let Some(update) = &plugin.update {
            let id = id.clone();
            let network = update.network.clone();
            actions =
                actions.child(
                    self.button(
                        format!("plugin-update-{id}"),
                        t("plugin_update"),
                        working,
                        idle,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.install(id.clone(), network.clone(), cx)
                    })),
                );
        } else if community && plugin.installed_version.is_none() {
            let id = id.clone();
            let network = plugin.network.clone();
            actions =
                actions.child(
                    self.button(
                        format!("plugin-install-{id}"),
                        t("plugin_install"),
                        working,
                        idle,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.install(id.clone(), network.clone(), cx)
                    })),
                );
        }
        if community && on_disk(plugin) {
            actions = actions.child(
                Button::new(SharedString::from(format!("plugin-remove-{id}")))
                    .custom(style::tint_text_button(cx.theme().danger, cx))
                    .small()
                    .label(t("plugin_remove"))
                    .disabled(!idle)
                    .on_click(cx.listener(move |this, _, _, cx| this.uninstall(id.clone(), cx))),
            );
        }

        h_flex()
            .w_full()
            .items_start()
            .justify_between()
            .gap_5()
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(ui::label(SharedString::from(plugin.name.clone())))
                            .child(Badge::new(origin).tone(BadgeTone::Neutral))
                            .when(!plugin.sandboxed, |line| {
                                line.child(Badge::new(t("plugin_native")).tone(BadgeTone::Neutral))
                            })
                            .child(ui::hint(version.into(), cx)),
                    )
                    .children(byline.map(|byline| ui::hint(byline, cx)))
                    .when(!plugin.description.is_empty(), |text| {
                        text.child(ui::hint(plugin.description.clone().into(), cx))
                    })
                    .children(reaches.map(|reaches| ui::hint(reaches, cx)))
                    .children(update_reaches.map(|warning| {
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child(icons::icon(icons::WARNING).size_3())
                            .child(warning)
                    })),
            )
            .when(working, |row| {
                row.child(ui::hint(t("plugin_working"), cx).flex_shrink_0())
            })
            .when(!working, |row| row.child(actions))
            .into_any_element()
    }

    fn button(
        &self,
        id: String,
        label: SharedString,
        working: bool,
        idle: bool,
        cx: &App,
    ) -> Button {
        Button::new(SharedString::from(id))
            .custom(style::soft_button(cx.theme().foreground, cx))
            .small()
            .label(label)
            .loading(working)
            .disabled(!idle)
    }
}

/// An update is only ever a build of a plugin already on disk, even one an update waiting across a restart left unloaded.
fn on_disk(plugin: &PluginInfo) -> bool {
    plugin.installed_version.is_some() || plugin.update.is_some()
}

/// The hosts as the user reads them; `$server` is the address the source was added with.
fn hosts(network: &[String]) -> String {
    network
        .iter()
        .map(|host| match host.as_str() {
            "$server" => t("plugin_your_server").to_string(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl Render for Plugins {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalog = self.catalog.clone().unwrap_or_default();
        let unreachable = catalog.marketplace_error.clone().map(|error| {
            t_with(
                "plugins_marketplace_unreachable",
                &[("error", error.into())],
            )
        });
        let (installed, available): (Vec<_>, Vec<_>) =
            catalog.plugins.iter().partition(|plugin| on_disk(plugin));

        let mut card = ui::card(cx)
            .child(ui::hint(t("plugins_description"), cx))
            .child(SectionHeading::new(t("plugins_installed")).heading(Heading::Group));
        for plugin in &installed {
            card = card.child(self.row(plugin, cx));
        }
        card = card.child(SectionHeading::new(t("plugins_marketplace")).heading(Heading::Group));
        card = match unreachable {
            Some(unreachable) => card.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().warning)
                    .child(unreachable),
            ),
            None if available.is_empty() && self.catalog.is_some() => {
                card.child(EmptyState::new(t("plugins_marketplace_empty")).compact())
            }
            None => card,
        };
        for plugin in &available {
            card = card.child(self.row(plugin, cx));
        }
        ui::section(t("plugins")).opaque().child(card)
    }
}
