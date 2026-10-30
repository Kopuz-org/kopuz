//! Every configured source, which one is live, and the options each publishes.

use std::collections::HashMap;

use api::schema::{FieldKind, FieldSpec, FieldValue};
use api::{SourceFolderEntry, SourceInfo, Table};
use components::dialogs::{DialogSize, FormDialog};
use components::format::{t, t_with, text};
use components::{
    EmptyState, FieldChanged, FolderBrowser, FolderList, SchemaForm, SourceCard, SourceMark,
    StatusLine, icons, parent_dir, style,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled, Subscription, WeakEntity, Window,
};
use link::Link;
use store::{ConfigModel, Notices, Query, SourceStatus, SourcesModel};

use crate::add_source::{AddSource, SourceAdded};
use crate::folders;
use crate::ui;

/// Answer one of a source's own options; a refusal puts the form back. The value is never logged.
fn write_setting(
    id: String,
    value: FieldValue,
    form: WeakEntity<SchemaForm>,
    window: &mut Window,
    cx: &mut App,
) {
    let key = value.key.clone();
    let call = Link::global(cx)
        .call(move |api| async move { api.set_source_settings(id, vec![value]).await });
    window
        .spawn(cx, async move |cx| match call.await {
            Ok(_) => {
                cx.update(|_, cx| {
                    SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx));
                })
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

/// The fields a source's form shows: a folder tree the source browses itself is picked with the browser below it.
fn form_fields(source: &SourceInfo) -> Vec<FieldSpec> {
    source
        .settings
        .iter()
        .filter(|field| {
            !(source.capabilities.browse_folders && matches!(field.kind, FieldKind::Directories))
        })
        .cloned()
        .collect()
}

/// One source's options, kept in step with the specs the daemon last answered with.
struct SourceOptions {
    form: Entity<SchemaForm>,
    _subscription: Subscription,
}

/// The in-place folder picker for a source whose library is a remote directory tree.
#[derive(Default)]
struct Browsing {
    open: bool,
    path: String,
    entries: Vec<SourceFolderEntry>,
    error: Option<SharedString>,
    loading: bool,
    generation: u64,
}

pub struct Sources {
    options: HashMap<String, SourceOptions>,
    browser: Browsing,
    switching: Option<String>,
    host_access: Entity<Query<bool>>,
    add: Entity<AddSource>,
    _subscriptions: Vec<Subscription>,
}

impl Sources {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sources = SourcesModel::global(cx);
        let config = ConfigModel::global(cx);
        let host_access = cx.new(|cx| {
            Query::new(
                &[Table::Servers],
                |api| async move { api.can_open_browser().await },
                cx,
            )
        });
        let add = cx.new(|cx| AddSource::new(window, cx));
        let subscriptions = vec![
            cx.observe_in(&sources, window, |this, _, window, cx| {
                this.sync_options(window, cx)
            }),
            cx.observe_in(&config, window, |this, _, window, cx| {
                this.sync_options(window, cx)
            }),
            cx.observe(&host_access, |this, query, cx| {
                let allowed = query.read(cx).value().copied().unwrap_or(true);
                this.add
                    .update(cx, |add, cx| add.set_host_access(allowed, cx));
                cx.notify();
            }),
            cx.subscribe_in(&add, window, |this, _, added: &SourceAdded, window, cx| {
                this.activate(added.0.clone(), window, cx)
            }),
        ];
        // The sources may already be loaded, in which case no observer would ever fire.
        cx.defer_in(window, |this, window, cx| this.sync_options(window, cx));
        Self {
            options: HashMap::new(),
            browser: Browsing::default(),
            switching: None,
            host_access,
            add,
            _subscriptions: subscriptions,
        }
    }

    fn sources(cx: &App) -> Vec<SourceInfo> {
        SourcesModel::global(cx).read(cx).sources().to_vec()
    }

    fn host_access(&self, cx: &App) -> bool {
        self.host_access.read(cx).value().copied().unwrap_or(true)
    }

    /// Keep one form per source over its options, rebuilt only when the daemon's fields change.
    fn sync_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let locked = ConfigModel::global(cx).read(cx).locked_names();
        let sources = Self::sources(cx);
        self.options
            .retain(|id, _| sources.iter().any(|source| &source.id == id));
        for source in sources {
            let specs = form_fields(&source);
            if specs.is_empty() {
                self.options.remove(&source.id);
                continue;
            }
            let options = self.options.entry(source.id.clone()).or_insert_with(|| {
                let form = cx.new(|cx| SchemaForm::new(specs.clone(), window, cx));
                let id = source.id.clone();
                let subscription = cx.subscribe_in(
                    &form,
                    window,
                    move |_, form, event: &FieldChanged, window, cx| {
                        write_setting(id.clone(), event.0.clone(), form.downgrade(), window, cx);
                    },
                );
                SourceOptions {
                    form,
                    _subscription: subscription,
                }
            });
            let locked = locked.clone();
            let held = source.authenticated;
            options
                .form
                .update(cx, |form, cx| form.sync(specs, locked, held, window, cx));
        }
        cx.notify();
    }

    /// The daemon probes the active source and reports on the row; a switch in flight reads as connecting.
    fn status(&self, source: &SourceInfo) -> SourceStatus {
        if self.switching.as_deref() == Some(source.id.as_str()) {
            return SourceStatus::Connecting;
        }
        SourceStatus::of(source.state)
    }

    /// Make a source active, then pick up whichever sign-in the daemon says it still needs.
    fn activate(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.switching = Some(id.clone());
        cx.notify();
        let call = Link::global(cx).call(move |api| async move { api.switch_source(id).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.switching = None;
                cx.notify();
            })
            .ok();
            match result {
                Ok(source) => {
                    cx.update(|_, cx| {
                        SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx));
                        ConfigModel::global(cx).update(cx, |config, cx| config.reload(cx));
                    })
                    .ok();
                    cx.update(|window, cx| store::start_sign_in(&source, window, cx))
                        .ok();
                }
                Err(error) => {
                    cx.update(|_, cx| Notices::failed(&error, cx)).ok();
                }
            }
        })
        .detach();
    }

    /// Pressed, not automatic: the store offers a browser profile already signed in before a new window.
    fn sign_in_again(&mut self, source: &SourceInfo, window: &mut Window, cx: &mut Context<Self>) {
        store::sign_in_again(source, window, cx);
    }

    fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add.update(cx, |form, cx| form.reset(window, cx));
        let form = self.add.clone();
        FormDialog::new(t("add_source"))
            .size(DialogSize::Wide)
            .on_submit(move |window, cx| form.update(cx, |form, cx| form.save(window, cx)))
            .open(self.add.clone(), window, cx);
        self.add.update(cx, |form, cx| form.focus(window, cx));
    }

    fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(move |api| async move { api.delete_source(id).await });
        cx.spawn(async move |_, cx| match call.await {
            Ok(()) => {
                cx.update(|cx| {
                    SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx));
                });
            }
            Err(error) => {
                cx.update(|cx| Notices::failed(&error, cx));
            }
        })
        .detach();
    }

    fn browse(&mut self, id: String, path: String, cx: &mut Context<Self>) {
        self.browser.open = true;
        self.browser.path = path.clone();
        self.browser.loading = true;
        self.browser.error = None;
        self.browser.generation += 1;
        let generation = self.browser.generation;
        cx.notify();
        let call =
            Link::global(cx).call(move |api| async move { api.browse_source(id, path).await });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if this.browser.generation != generation {
                    return;
                }
                this.browser.loading = false;
                match result {
                    Ok(entries) => {
                        this.browser.entries = entries;
                        this.browser.error = None;
                    }
                    Err(error) => {
                        this.browser.entries.clear();
                        this.browser.error = Some(t_with(
                            "folder_browse_failed",
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

    fn status_line(&self, source: &SourceInfo, cx: &mut Context<Self>) -> StatusLine {
        let source = source.clone();
        StatusLine::new(self.status(&source))
            .waiting(store::sign_in_wait(&source.id, cx))
            .on_sign_in(cx.listener(move |this: &mut Self, _, window, cx| {
                this.sign_in_again(&source, window, cx)
            }))
    }

    /// A remote folder tree's chosen folders, and the browser that adds to them.
    fn remote_folders(&self, source: &SourceInfo, cx: &mut Context<Self>) -> AnyElement {
        let id = source.id.clone();
        let chosen = folders::of(source);
        let path = if self.browser.path.is_empty() {
            "/".to_string()
        } else {
            self.browser.path.clone()
        };
        let list = FolderList::new(SharedString::from(format!("remote-{id}")), chosen.clone())
            .empty(t("no_music_folders"))
            .on_remove({
                let id = id.clone();
                let chosen = chosen.clone();
                move |folder, _, cx| {
                    let kept = chosen
                        .iter()
                        .filter(|held| held.as_str() != folder)
                        .cloned()
                        .collect();
                    folders::write(id.clone(), kept, cx);
                }
            })
            .when(!self.browser.open, |list| {
                let id = id.clone();
                list.on_add(cx.listener(move |this: &mut Self, _, _, cx| {
                    this.browse(id.clone(), "/".to_string(), cx)
                }))
            });

        v_flex()
            .w_full()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(ui::hint(t("remote_music_folders"), cx))
            .child(list)
            .when(self.browser.open, |column| {
                column.child(self.browser_panel(&id, &path, &chosen, cx))
            })
            .into_any_element()
    }

    fn browser_panel(
        &self,
        id: &str,
        path: &str,
        chosen: &[String],
        cx: &mut Context<Self>,
    ) -> FolderBrowser {
        let this = cx.entity().downgrade();
        let (id, chosen) = (id.to_string(), chosen.to_vec());
        FolderBrowser::new(path, self.browser.entries.clone())
            .loading(self.browser.loading)
            .error(self.browser.error.clone())
            .on_open({
                let (this, id) = (this.clone(), id.clone());
                move |target, _, cx| {
                    this.update(cx, |this, cx| {
                        this.browse(id.clone(), target.to_string(), cx)
                    })
                    .ok();
                }
            })
            .on_parent(cx.listener({
                let (id, path) = (id.clone(), path.to_string());
                move |this: &mut Self, _, _, cx| this.browse(id.clone(), parent_dir(&path), cx)
            }))
            .on_use(move |path, _, cx| {
                let mut kept = chosen.clone();
                if !kept.iter().any(|held| held == path) {
                    kept.push(path.to_string());
                }
                folders::write(id.clone(), kept, cx);
                this.update(cx, |this, cx| {
                    this.browser.open = false;
                    cx.notify();
                })
                .ok();
            })
            .on_cancel(cx.listener(|this: &mut Self, _, _, cx| {
                this.browser.open = false;
                cx.notify();
            }))
    }

    fn source(&self, source: &SourceInfo, cx: &mut Context<Self>) -> SourceCard {
        let id = source.id.clone();
        let service = text(&source.service.name);
        let host_access = self.host_access(cx);
        let picker = source.active && source.capabilities.browse_folders && source.authenticated;
        let needs_host = source.capabilities.browser_playback && !host_access;

        SourceCard::new(id.clone(), source.name.clone())
            .mark(SourceMark::new(&source.service))
            .service(t_with("service", &[("name", service.to_string().into())]))
            .detail(source.detail.clone().unwrap_or_default())
            .active(source.active)
            .when(source.active && source.needs_network, |card| {
                card.status(self.status_line(source, cx))
            })
            .when(!source.active, |card| {
                card.action(
                    Button::new(SharedString::from(format!("switch-{id}")))
                        .custom(style::soft_button(cx.theme().foreground, cx))
                        .xsmall()
                        .label(t("switch_to_server"))
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this: &mut Self, _, window, cx| {
                                this.activate(id.clone(), window, cx)
                            }
                        })),
                )
            })
            .when(!source.permanent, |card| {
                card.action(
                    Button::new(SharedString::from(format!("delete-{id}")))
                        .custom(style::tint_text_button(cx.theme().danger, cx))
                        .small()
                        .label(t("delete"))
                        .on_click(cx.listener({
                            let id = id.clone();
                            let name = SharedString::from(source.name.clone());
                            // This takes the source's stored sign-in and library with it.
                            move |_: &mut Self, _, window, cx| {
                                let id = id.clone();
                                let entity = cx.entity();
                                components::dialogs::confirm(
                                    components::dialogs::ConfirmKind::Delete,
                                    t("delete"),
                                    name.clone(),
                                    move |_, cx| {
                                        let id = id.clone();
                                        entity.update(cx, |this, cx| this.delete(id, cx));
                                    },
                                    window,
                                    cx,
                                );
                            }
                        })),
                )
            })
            .when(picker, |card| card.child(self.remote_folders(source, cx)))
            .when(needs_host, |card| {
                card.child(ui::host_access_warning(
                    SharedString::from(format!("host-access-{id}")),
                    t("browser_playback_needs_host"),
                ))
            })
            .children(self.options.get(&source.id).map(|options| {
                v_flex()
                    .w_full()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(options.form.clone())
            }))
    }
}

impl Render for Sources {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sources = Self::sources(cx);
        let rows: Vec<_> = sources
            .iter()
            .map(|source| self.source(source, cx))
            .collect();
        ui::section(t("sources")).opaque().child(
            ui::card(cx)
                .when(sources.is_empty(), |card| {
                    card.child(EmptyState::new(t("no_saved_servers")).compact())
                })
                .children(rows)
                .child(
                    gpui_kit::component::h_flex().child(
                        Button::new("add-source")
                            .custom(style::soft_button(cx.theme().foreground, cx))
                            .small()
                            .icon(icons::icon(icons::PLUS))
                            .label(t("add_source"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_add(window, cx))),
                    ),
                ),
        )
    }
}
