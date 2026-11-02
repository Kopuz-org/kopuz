//! The "add source" form: a name, a service, and whatever form the daemon publishes for it.

use std::time::Duration;

use api::schema::{FieldKind, FieldValue, Problem};
use api::{
    BrowserSession, DraftCheck, DraftProbe, ServerProbe, ServiceInfo, SignInKind, SourceDraft,
    Table,
};
use components::format::{t, t_with, text};
use components::{BrowserSessionList, Callout, CalloutTone, ErrorBanner, LabeledInput};
use components::{FieldChanged, SchemaForm};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled, Subscription, Window, div,
};
use link::Link;
use store::Query;

use crate::ui;

/// A source was saved; the owner makes it active and picks up whatever sign-in it needs.
#[derive(Debug, Clone)]
pub struct SourceAdded(pub String);

/// How long the form waits after an answer before trying the draft.
const PROBE_DELAY: Duration = Duration::from_millis(400);
/// The height of one probe line, kept free until the answer fills it.
const PROBE_SLOT: gpui_kit::Pixels = gpui_kit::px(42.);

/// What the last probe of the draft found, said in one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeNote {
    Found(u32),
    Empty,
    Unreachable(String),
}

impl ProbeNote {
    /// A note worth a second press before the draft is saved anyway.
    pub fn warns(&self) -> bool {
        !matches!(self, ProbeNote::Found(_))
    }

    fn line(&self) -> SharedString {
        match self {
            ProbeNote::Found(count) => {
                t_with("onboarding_probe_found", &[("count", (*count).into())])
            }
            ProbeNote::Empty => t("onboarding_probe_empty"),
            ProbeNote::Unreachable(url) => t_with(
                "onboarding_probe_unreachable",
                &[("url", url.clone().into())],
            ),
        }
    }
}

/// The one line a probe answer comes down to; `url` is the address the draft named.
pub fn note(probe: &DraftProbe, url: Option<&str>) -> Option<ProbeNote> {
    match probe {
        DraftProbe::Folders(folders) => {
            let found: u32 = folders
                .iter()
                .filter(|folder| folder.exists && folder.readable)
                .map(|folder| folder.audio_files)
                .sum();
            Some(match found {
                0 => ProbeNote::Empty,
                found => ProbeNote::Found(found),
            })
        }
        DraftProbe::Server(ServerProbe::Unreachable) => {
            Some(ProbeNote::Unreachable(url.unwrap_or_default().to_string()))
        }
        DraftProbe::Server(ServerProbe::Status(code)) if *code >= 500 => {
            Some(ProbeNote::Unreachable(url.unwrap_or_default().to_string()))
        }
        DraftProbe::Server(_) | DraftProbe::NotProbeable => None,
    }
}

/// The name a source gets when none was typed: its first folder's name, else its service's.
pub fn default_name(service: &ServiceInfo, values: &[FieldValue]) -> String {
    let folder = service
        .fields
        .iter()
        .find(|field| matches!(field.kind, FieldKind::Directories))
        .and_then(|field| values.iter().find(|value| value.key == field.key))
        .and_then(|value| {
            api::schema::decode_directories(&value.value)
                .into_iter()
                .next()
        })
        .and_then(|path| {
            std::path::Path::new(path.trim_end_matches('/'))
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
    folder.unwrap_or_else(|| text(&service.name).to_string())
}

/// The draft's answers with nothing secret, which is what a probe or a warning is about.
fn shape(draft: &SourceDraft) -> SourceDraft {
    SourceDraft {
        secrets: Vec::new(),
        ..draft.clone()
    }
}

pub struct AddSource {
    services: Entity<Query<Vec<ServiceInfo>>>,
    service: Option<ServiceInfo>,
    name: Entity<InputState>,
    name_problem: Option<SharedString>,
    form: Option<Entity<SchemaForm>>,
    check: Option<DraftCheck>,
    error: Option<SharedString>,
    saving: bool,
    host_access: bool,
    /// Drawn inside a page that owns the buttons, rather than as a dialog with its own.
    embedded: bool,
    /// Try the draft before saving it, and warn about what it found.
    probing: bool,
    /// No problem is shown until the person has answered something or pressed save.
    touched: bool,
    probe: Option<(SourceDraft, ProbeNote)>,
    /// A warned draft saves on the next press if it has not changed since.
    warned: Option<SourceDraft>,
    /// Save into this source's settings instead of creating one.
    fill: Option<String>,
    /// The last saved answers, so a form can be put back the way it was.
    last: Option<(ServiceInfo, Vec<FieldValue>, String)>,
    /// The browser profiles signed in to the chosen service, by service id; `None` while the daemon looks.
    sessions: Option<(String, Option<Vec<BrowserSession>>)>,
    /// The browser session being taken into a new source.
    importing: Option<String>,
    // Bumped per check, so a slower answer never overwrites a newer one.
    generation: u64,
    probe_generation: u64,
    _form_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SourceAdded> for AddSource {}

impl AddSource {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let services = cx.new(|cx| {
            Query::new(
                &[Table::Servers],
                |api| async move { api.services().await },
                cx,
            )
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("source_name")));
        let subscriptions = vec![
            cx.observe_in(&services, window, |this, _, window, cx| {
                this.ensure_service(window, cx)
            }),
            cx.subscribe(&name, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.answered(cx);
                }
            }),
        ];
        Self {
            services,
            service: None,
            name,
            name_problem: None,
            form: None,
            check: None,
            error: None,
            saving: false,
            host_access: true,
            embedded: false,
            probing: false,
            touched: false,
            probe: None,
            warned: None,
            fill: None,
            last: None,
            sessions: None,
            importing: None,
            generation: 0,
            probe_generation: 0,
            _form_subscription: None,
            _subscriptions: subscriptions,
        }
    }

    /// Drawn inline by a guide that owns the buttons, trying each draft before it is saved.
    pub fn embedded(mut self) -> Self {
        self.embedded = true;
        self.probing = true;
        self
    }

    pub fn set_host_access(&mut self, host_access: bool, cx: &mut Context<Self>) {
        if self.host_access != host_access {
            self.host_access = host_access;
            cx.notify();
        }
    }

    /// Clear the form for a fresh source, keeping the service list already read.
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.name
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.check = None;
        self.error = None;
        self.name_problem = None;
        self.saving = false;
        self.touched = false;
        self.probe = None;
        self.warned = None;
        self.fill = None;
        self.importing = None;
        self.service = None;
        self.form = None;
        self._form_subscription = None;
        self.ensure_service(window, cx);
        cx.notify();
    }

    /// The name is asked first; opening a dialog focuses the dialog, so this runs once it is open.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |state, cx| state.focus(window, cx));
    }

    fn offered(&self, cx: &App) -> Vec<ServiceInfo> {
        self.services.read(cx).value().cloned().unwrap_or_default()
    }

    /// Nothing is chosen until the list arrives, and then the first is.
    fn ensure_service(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.service.is_some() || self.embedded {
            cx.notify();
            return;
        }
        let Some(first) = self.offered(cx).into_iter().next() else {
            cx.notify();
            return;
        };
        self.select(first, window, cx);
    }

    pub fn select(&mut self, service: ServiceInfo, window: &mut Window, cx: &mut Context<Self>) {
        let form = cx.new(|cx| SchemaForm::new(service.fields.clone(), window, cx));
        self._form_subscription =
            Some(cx.subscribe(&form, |this, _, _: &FieldChanged, cx| this.answered(cx)));
        self.form = Some(form);
        self.service = Some(service);
        self.check = None;
        self.error = None;
        self.probe = None;
        self.warned = None;
        self.recheck(cx);
        self.look_for_sessions(cx);
        cx.notify();
    }

    /// Ask which browser profiles are already signed in to the chosen service; most services have none.
    fn look_for_sessions(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.service.as_ref().map(|service| service.id.clone()) else {
            return;
        };
        if self
            .sessions
            .as_ref()
            .is_some_and(|(held, _)| *held == service)
        {
            return;
        }
        self.sessions = Some((service.clone(), None));
        let call = Link::global(cx).call({
            let service = service.clone();
            move |api| async move { api.browser_sessions(service).await }
        });
        cx.spawn(async move |this, cx| {
            let found = call.await.unwrap_or_else(|error| {
                tracing::debug!(%error, "could not list the browser sessions");
                Vec::new()
            });
            this.update(cx, |this, cx| {
                if this
                    .sessions
                    .as_ref()
                    .is_some_and(|(held, _)| *held == service)
                {
                    this.sessions = Some((service, Some(found)));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The profiles to offer, once the daemon has said the service signs in through a browser.
    fn offered_sessions(&self) -> Option<Option<Vec<BrowserSession>>> {
        let browser = self
            .check
            .as_ref()
            .is_some_and(|check| check.sign_in == SignInKind::Browser);
        let service = self.service.as_ref()?;
        let (held, sessions) = self.sessions.as_ref()?;
        (browser && *held == service.id).then(|| sessions.clone())
    }

    /// Save the draft as a new source signed in with `session`'s cookies, so no sign-in window opens.
    pub fn use_session(
        &mut self,
        session: BrowserSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.saving || self.importing.is_some() || self.service.is_none() {
            return;
        }
        if let Some(form) = self.form.as_ref() {
            form.update(cx, |form, cx| form.commit_typed(cx));
        }
        let draft = self.draft(cx);
        let remembered = self.service.clone().map(|service| {
            (
                service,
                draft.values.clone(),
                self.name.read(cx).value().to_string(),
            )
        });
        self.importing = Some(session.id.clone());
        self.error = None;
        cx.notify();
        let link = Link::global(cx).clone();
        let embedded = self.embedded;
        cx.spawn_in(window, async move |this, cx| {
            let fail = |message: SharedString,
                        this: &gpui_kit::WeakEntity<Self>,
                        cx: &mut gpui_kit::AsyncWindowContext| {
                this.update(cx, |this, cx| {
                    this.importing = None;
                    this.error = Some(message);
                    cx.notify();
                })
                .ok();
            };
            let checked = link
                .call({
                    let draft = draft.clone();
                    move |api| async move { api.check_source_draft(draft).await }
                })
                .await;
            let problem = match checked {
                Ok(check) => check.problems.first().map(|problem| text(&problem.label)),
                Err(error) => Some(store::human(&error)),
            };
            if let Some(problem) = problem {
                return fail(problem, &this, cx);
            }
            let saved = match link
                .call(move |api| async move { api.upsert_source(draft).await })
                .await
            {
                Ok(saved) => saved,
                Err(error) => return fail(store::human(&error), &this, cx),
            };
            let id = saved.id.clone();
            let imported = link
                .call({
                    let id = id.clone();
                    move |api| async move { api.import_browser_session(id, session.id).await }
                })
                .await;
            if let Err(error) = imported {
                // A source that never signed in would only sit in the list asking to.
                let removed = link
                    .call({
                        let id = id.clone();
                        move |api| async move { api.delete_source(id).await }
                    })
                    .await;
                if let Err(cleanup) = removed {
                    tracing::warn!(%cleanup, "could not remove a source whose sign-in failed");
                }
                let message = t_with(
                    "signin_failed",
                    &[("error", store::human(&error).to_string().into())],
                );
                return fail(message, &this, cx);
            }
            if !embedded {
                cx.update(|window, cx| window.close_dialog(cx)).ok();
            }
            this.update_in(cx, |this, window, cx| {
                this.reset(window, cx);
                this.last = remembered;
                cx.emit(SourceAdded(id));
            })
            .ok();
        })
        .detach();
    }

    /// The service's form with `values` already answered, as when editing what was saved.
    pub fn select_with(
        &mut self,
        mut service: ServiceInfo,
        values: &[FieldValue],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for field in &mut service.fields {
            if matches!(field.kind, FieldKind::Secret) {
                continue;
            }
            if let Some(value) = values.iter().find(|value| value.key == field.key) {
                field.value = Some(value.value.clone());
            }
        }
        self.select(service, window, cx);
        self.schedule_probe(cx);
    }

    /// Save into an existing source's settings rather than creating another.
    pub fn fill_into(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.fill = id;
        cx.notify();
    }

    /// Put back the answers last saved, for a guide that took the source away again.
    pub fn restore_last(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((service, values, name)) = self.last.clone() else {
            return false;
        };
        self.name
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.select_with(service, &values, window, cx);
        true
    }

    pub fn service(&self) -> Option<&ServiceInfo> {
        self.service.as_ref()
    }

    pub fn is_saving(&self) -> bool {
        self.saving
    }

    /// The probe line for the current draft, if it has been tried.
    pub fn probe_note(&self, cx: &App) -> Option<ProbeNote> {
        let draft = shape(&self.draft(cx));
        self.probe
            .as_ref()
            .filter(|(tried, _)| *tried == draft)
            .map(|(_, note)| note.clone())
    }

    /// Something was answered: problems may now show, and an old failure no longer applies.
    fn answered(&mut self, cx: &mut Context<Self>) {
        self.touched = true;
        self.error = None;
        self.recheck(cx);
        self.schedule_probe(cx);
    }

    fn url(&self, cx: &App) -> Option<String> {
        let service = self.service.as_ref()?;
        let key = service
            .fields
            .iter()
            .find(|field| matches!(field.kind, FieldKind::Url))?
            .key
            .clone();
        self.form
            .as_ref()
            .and_then(|form| form.read(cx).value(&key).map(str::to_string))
    }

    fn draft(&self, cx: &App) -> SourceDraft {
        let Some(service) = self.service.as_ref() else {
            return SourceDraft::default();
        };
        let secret_keys: Vec<String> = service
            .fields
            .iter()
            .filter(|field| matches!(field.kind, FieldKind::Secret))
            .map(|field| field.key.clone())
            .collect();
        let mut answers = self
            .form
            .as_ref()
            .map(|form| form.read(cx).values())
            .unwrap_or_default();
        answers.sort_by(|left, right| left.key.cmp(&right.key));
        let (secrets, values): (Vec<FieldValue>, Vec<FieldValue>) = answers
            .into_iter()
            .partition(|answer| secret_keys.contains(&answer.key));
        let typed = self.name.read(cx).value().trim().to_string();
        let name = match typed.is_empty() {
            true => default_name(service, &values),
            false => typed,
        };
        SourceDraft {
            id: None,
            name,
            service: service.id.clone(),
            values,
            secrets,
        }
    }

    /// Ask the daemon what the draft would do; cheap enough to run on every edit.
    fn recheck(&mut self, cx: &mut Context<Self>) {
        if self.service.is_none() {
            return;
        }
        let draft = self.draft(cx);
        self.generation += 1;
        let generation = self.generation;
        let call =
            Link::global(cx).call(move |api| async move { api.check_source_draft(draft).await });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.check = result.ok();
                this.show_problems(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn show_problems(&mut self, cx: &mut Context<Self>) {
        let problems: Vec<Problem> = match self.touched {
            true => self
                .check
                .as_ref()
                .map(|check| check.problems.clone())
                .unwrap_or_default(),
            false => Vec::new(),
        };
        // The name is this form's own field, so its problem has nowhere else to land.
        self.name_problem = problems
            .iter()
            .find(|problem| problem.field.as_deref() == Some("name"))
            .map(|problem| text(&problem.label));
        if let Some(form) = self.form.as_ref() {
            form.update(cx, |form, cx| form.set_problems(problems, cx));
        }
    }

    fn schedule_probe(&mut self, cx: &mut Context<Self>) {
        if !self.probing || self.service.is_none() {
            return;
        }
        self.probe_generation += 1;
        let generation = self.probe_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PROBE_DELAY).await;
            let Ok(call) = this.update(cx, |this, cx| {
                (this.probe_generation == generation).then(|| this.start_probe(cx))
            }) else {
                return;
            };
            let Some(call) = call else { return };
            call.await;
        })
        .detach();
    }

    /// Try the draft as it stands; the answer is kept against the draft it was for.
    fn start_probe(&mut self, cx: &mut Context<Self>) -> gpui_kit::Task<()> {
        let draft = self.draft(cx);
        let url = self.url(cx);
        let tried = shape(&draft);
        let call =
            Link::global(cx).call(move |api| async move { api.probe_source_draft(draft).await });
        cx.spawn(async move |this, cx| {
            let probe = match call.await {
                Ok(probe) => probe,
                Err(error) => {
                    tracing::debug!(%error, "draft probe failed");
                    return;
                }
            };
            this.update(cx, |this, cx| {
                match note(&probe, url.as_deref()) {
                    Some(found) => this.probe = Some((tried, found)),
                    None => this.probe = None,
                }
                cx.notify();
            })
            .ok();
        })
    }

    /// A sandboxed daemon cannot open a browser, so a service whose sign-in is one cannot be saved.
    fn blocked(&self) -> bool {
        !self.host_access
            && self
                .check
                .as_ref()
                .is_some_and(|check| check.sign_in == SignInKind::Browser)
    }

    /// Save the draft; a dialog closes itself once accepted, so a caller must not close it too.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.blocked() || self.service.is_none() {
            return;
        }
        self.touched = true;
        if let Some(form) = self.form.as_ref() {
            form.update(cx, |form, cx| form.commit_typed(cx));
        }
        // The last check saw the boxes before they were committed, so its problems are stale.
        self.recheck(cx);
        let draft = self.draft(cx);
        let shaped = shape(&draft);
        let probe = self
            .probing
            .then(|| match &self.probe {
                Some((tried, _)) if *tried == shaped => None,
                _ => Some(()),
            })
            .flatten();
        let warned = self.warned.as_ref() == Some(&shaped);
        self.error = None;
        self.saving = true;
        cx.notify();
        let link = Link::global(cx).clone();
        let checked = link.call({
            let draft = draft.clone();
            move |api| async move { api.check_source_draft(draft).await }
        });
        cx.spawn_in(window, async move |this, cx| {
            let problem = match checked.await {
                Ok(check) => check.problems.first().map(|problem| text(&problem.label)),
                Err(error) => Some(store::human(&error)),
            };
            if let Some(problem) = problem {
                this.update(cx, |this, cx| {
                    this.saving = false;
                    this.error = Some(problem);
                    cx.notify();
                })
                .ok();
                return;
            }
            if probe.is_some() {
                let Ok(tried) = this.update(cx, |this, cx| this.start_probe(cx)) else {
                    return;
                };
                tried.await;
            }
            let hold = this
                .update(cx, |this, cx| {
                    let note = this.probe_note(cx);
                    // A warning already on screen when this was pressed has been read; only a fresh one waits.
                    let fresh = probe.is_some() && !warned;
                    let hold = fresh && note.is_some_and(|note| note.warns());
                    if hold {
                        this.warned = Some(shaped.clone());
                        this.saving = false;
                        cx.notify();
                    }
                    hold
                })
                .unwrap_or(true);
            if hold {
                return;
            }
            this.update_in(cx, |this, window, cx| this.commit(draft, window, cx))
                .ok();
        })
        .detach();
    }

    fn commit(&mut self, draft: SourceDraft, window: &mut Window, cx: &mut Context<Self>) {
        let link = Link::global(cx).clone();
        let fill = self.fill.clone();
        let remembered = self.service.clone().map(|service| {
            (
                service,
                draft.values.clone(),
                self.name.read(cx).value().to_string(),
            )
        });
        let saved = match fill {
            Some(id) => {
                let values = draft.values.clone();
                link.call(move |api| async move { api.set_source_settings(id, values).await })
            }
            None => link.call(move |api| async move { api.upsert_source(draft).await }),
        };
        let embedded = self.embedded;
        cx.spawn_in(window, async move |this, cx| {
            let result = saved.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                cx.notify();
            })
            .ok();
            match result {
                Ok(source) => {
                    if !embedded {
                        cx.update(|window, cx| window.close_dialog(cx)).ok();
                    }
                    // A secret left in a live form is one the next source could pick up.
                    this.update_in(cx, |this, window, cx| {
                        this.reset(window, cx);
                        this.last = remembered;
                    })
                    .ok();
                    this.update(cx, |_, cx| cx.emit(SourceAdded(source.id.clone())))
                        .ok();
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.error = Some(store::human(&error));
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn service_label(service: &ServiceInfo) -> SharedString {
        let name = text(&service.name);
        if service.experimental {
            SharedString::from(format!("{name} ({})", t("experimental")))
        } else {
            name
        }
    }

    fn probe_line(&self, cx: &App) -> Option<Callout> {
        let note = self.probe_note(cx)?;
        let tone = match note.warns() {
            true => CalloutTone::Warning,
            false => CalloutTone::Info,
        };
        Some(Callout::new(note.line()).id("add-source-probe").tone(tone))
    }
}

impl Render for AddSource {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let services = self.offered(cx);
        let chosen = self
            .service
            .as_ref()
            .map(Self::service_label)
            .unwrap_or_else(|| t("choose"));
        let saving = self.saving;
        let blocked = self.blocked();
        let embedded = self.embedded;
        let this = cx.entity().downgrade();
        // A folder source is named after its folder, so asking for a name only gets in the way.
        let named = self.fill.is_none();

        v_flex()
            .w_full()
            .gap_3()
            .when(blocked, |form| {
                form.child(ui::host_access_warning(
                    "add-source-host-access",
                    t("browser_sign_in_needs_host"),
                ))
            })
            .when(named, |form| {
                form.child(
                    v_flex().w_full().gap_1().child(
                        LabeledInput::new("", &self.name)
                            .framed(false)
                            .disabled(saving)
                            .problem(self.name_problem.clone()),
                    ),
                )
            })
            .when(!embedded, |form| {
                form.child(
                    Button::new("add-source-service")
                        .outline()
                        .w_full()
                        .justify_between()
                        .label(chosen)
                        .disabled(saving)
                        .dropdown_menu(move |mut menu, _, _| {
                            for service in &services {
                                let service = service.clone();
                                let this = this.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(Self::service_label(&service)).on_click(
                                        move |_, window, cx| {
                                            this.update(cx, |this, cx| {
                                                this.select(service.clone(), window, cx)
                                            })
                                            .ok();
                                        },
                                    ),
                                );
                            }
                            menu
                        }),
                )
            })
            .children(self.offered_sessions().map(|sessions| {
                let this = cx.entity().downgrade();
                BrowserSessionList::new("add-source-sessions", sessions)
                    .importing(self.importing.clone())
                    .disabled(saving)
                    .on_use(move |session, window, cx| {
                        this.update(cx, |this, cx| this.use_session(session, window, cx))
                            .ok();
                    })
            }))
            .children(self.form.clone())
            .when(self.probing, |form| {
                // Held open, so the answer arriving never moves the buttons under the pointer.
                form.child(
                    div()
                        .w_full()
                        .min_h(PROBE_SLOT)
                        .children(self.probe_line(cx)),
                )
            })
            .when_some(self.error.clone(), |form, message| {
                form.child(ErrorBanner::new(message))
            })
            .when(!embedded, |form| {
                form.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cancel-add-source")
                                .outline()
                                .label(t("cancel"))
                                .disabled(saving)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("save-add-source")
                                .primary()
                                .label(t("save"))
                                .loading(saving)
                                .disabled(saving || blocked)
                                .on_click(cx.listener(|this: &mut Self, _, window, cx| {
                                    this.save(window, cx)
                                })),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders;
    use api::FolderProbe;
    use api::schema::{FieldSpec, Text};

    fn folders_service() -> ServiceInfo {
        ServiceInfo {
            id: "folders".into(),
            name: Text::Literal("Music Folders".into()),
            fields: vec![FieldSpec {
                key: folders::KEY.into(),
                kind: FieldKind::Directories,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn folder(audio_files: u32, exists: bool) -> FolderProbe {
        FolderProbe {
            exists,
            readable: exists,
            audio_files,
            ..Default::default()
        }
    }

    #[test]
    fn a_folder_source_is_named_after_its_first_folder() {
        let values = vec![FieldValue::new(
            folders::KEY,
            api::schema::encode_directories(&["/home/me/Music/".into()]),
        )];
        assert_eq!(default_name(&folders_service(), &values), "Music");
    }

    #[test]
    fn a_source_without_folders_is_named_after_its_service() {
        assert_eq!(default_name(&folders_service(), &[]), "Music Folders");
    }

    #[test]
    fn folders_count_only_what_can_be_read() {
        let probe = DraftProbe::Folders(vec![folder(3, true), folder(9, false)]);
        assert_eq!(note(&probe, None), Some(ProbeNote::Found(3)));
        let empty = DraftProbe::Folders(vec![folder(0, true)]);
        assert_eq!(note(&empty, None), Some(ProbeNote::Empty));
        assert!(ProbeNote::Empty.warns());
        assert!(!ProbeNote::Found(1).warns());
    }

    #[test]
    fn a_server_that_answers_at_all_is_not_a_warning() {
        let url = Some("http://nas:4533");
        assert_eq!(note(&DraftProbe::Server(ServerProbe::Reachable), url), None);
        assert_eq!(
            note(&DraftProbe::Server(ServerProbe::Status(401)), url),
            None
        );
        assert_eq!(
            note(&DraftProbe::Server(ServerProbe::Unreachable), url),
            Some(ProbeNote::Unreachable("http://nas:4533".into()))
        );
        assert_eq!(note(&DraftProbe::NotProbeable, url), None);
    }
}
