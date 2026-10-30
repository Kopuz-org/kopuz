//! First run: the language, the music (a source added, connected and filled), the look, playback, services, privacy, then the way in.

mod appearance;
mod flow;
mod pick;
mod playback;
mod privacy;
mod ready;
mod services;
mod sync;
mod ui;
mod welcome;

use api::{JobKind, JobState, Page, ServiceInfo, SourceInfo, SourceState, Table, TrackFilter};
use components::format::t;
use components::{StepFrame, Stepper};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollHandle, StatefulInteractiveElement as _, Styled,
    Subscription, Window,
};
use link::Link;
use settings_sources::add_source::{AddSource, SourceAdded};
use settings_sources::integrations::Integrations;
use store::{ConfigModel, JobEvent, JobsModel, Navigator, Query, Route, SetupModel, SourcesModel};

use flow::Step;

/// The source this run added, and what to put back if Back takes it away again.
#[derive(Debug, Clone)]
struct Added {
    id: String,
    /// The source that was active before, switched back to on undo.
    previous: Option<String>,
    /// The default source was filled rather than a new one made: the folders it held before.
    filled: Option<Vec<String>>,
}

/// How the first scan or sync ended, and what it left in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Outcome {
    ok: bool,
    tracks: u32,
    albums: u32,
}

/// The guide the shell draws instead of the page while setup runs.
pub struct FirstRun {
    step: Step,
    /// The setup run this state belongs to; a new one starts the guide over.
    run: u64,
    added: Option<Added>,
    /// The default source's folders when its form opened, for the undo.
    fill_previous: Option<Vec<String>>,
    /// The newest daemon job before the source was saved; only later ones can be its first fill.
    baseline: Option<u64>,
    /// Whether the jobs were last checked with the source ready to fill, so each change checks once.
    checked: Option<bool>,
    /// The saved source has been probed once since its sign-in landed.
    probed: bool,
    outcome: Option<Outcome>,
    language_offered: bool,
    backdrop_offered: bool,
    focus_first: bool,
    services: Entity<Query<Vec<ServiceInfo>>>,
    /// How many tracks the active source's library holds; a configured but empty folder is not music.
    tracks: Entity<Query<u32>>,
    add: Entity<AddSource>,
    integrations: Entity<Integrations>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl FirstRun {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sources = SourcesModel::global(cx);
        let jobs = JobsModel::global(cx);
        let setup = SetupModel::global(cx);
        let config = ConfigModel::global(cx);
        let services = cx.new(|cx| {
            Query::new(
                &[Table::Servers],
                |api| async move { api.services().await },
                cx,
            )
        });
        let tracks = cx.new(|cx| {
            Query::new(
                &[Table::Tracks, Table::Servers],
                |api| async move {
                    let page = Page {
                        offset: 0,
                        limit: 1,
                    };
                    api.tracks(TrackFilter::default(), page)
                        .await
                        .map(|page| page.total)
                },
                cx,
            )
        });
        let host_access = cx.new(|cx| {
            Query::new(
                &[Table::Servers],
                |api| async move { api.can_open_browser().await },
                cx,
            )
        });
        let add = cx.new(|cx| AddSource::new(window, cx).embedded());
        let subscriptions = vec![
            cx.observe(&sources, |this, _, cx| this.sources_moved(cx)),
            cx.observe(&services, |_, _, cx| cx.notify()),
            cx.observe(&tracks, |_, _, cx| cx.notify()),
            cx.observe(&add, |_, _, cx| cx.notify()),
            cx.observe(&config, |this, _, cx| {
                this.offer_language(cx);
                this.offer_backdrop(cx);
            }),
            cx.observe_in(&setup, window, |this, setup, window, cx| {
                let run = setup.read(cx).run();
                if flow::starts_over(this.run, run) {
                    this.run = run;
                    this.reset(window, cx);
                }
                cx.notify();
            }),
            cx.observe(&host_access, |this, query, cx| {
                let allowed = query.read(cx).value().copied().unwrap_or(true);
                this.add
                    .update(cx, |add, cx| add.set_host_access(allowed, cx));
                cx.notify();
            }),
            cx.subscribe_in(&add, window, |this, _, added: &SourceAdded, window, cx| {
                this.adopt(added.0.clone(), window, cx)
            }),
            cx.subscribe(&jobs, |this, _, event: &JobEvent, cx| {
                this.job_finished(event, cx)
            }),
        ];
        let mut this = Self {
            step: Step::Welcome,
            run: SetupModel::global(cx).read(cx).run(),
            added: None,
            fill_previous: None,
            baseline: None,
            checked: None,
            probed: false,
            outcome: None,
            language_offered: false,
            backdrop_offered: false,
            focus_first: false,
            services,
            tracks,
            add,
            integrations: cx.new(|cx| Integrations::new(window, cx)),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        // The config is usually loaded before the guide opens, so no change will arrive to prompt these.
        this.offer_language(cx);
        this.offer_backdrop(cx);
        this
    }

    /// Start over from the welcome, with nothing left of an earlier run.
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step = Step::Welcome;
        self.added = None;
        self.fill_previous = None;
        self.baseline = None;
        self.checked = None;
        self.probed = false;
        self.outcome = None;
        self.language_offered = false;
        self.backdrop_offered = false;
        self.focus_first = false;
        self.add.update(cx, |add, cx| add.reset(window, cx));
        self.offer_language(cx);
        self.offer_backdrop(cx);
        cx.notify();
    }

    fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        self.step = step;
        match step {
            Step::Pick => self.focus_first = true,
            Step::Connect => self.take_baseline(cx),
            _ => {}
        }
        self.scroll
            .set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)));
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A saved source still connecting is taken away, leaving its form to correct.
        if self.step == Step::Connect && self.added.is_some() {
            self.undo(window, cx);
            cx.notify();
            return;
        }
        let Some(to) = self.step.back(self.added.is_some()) else {
            return;
        };
        if self.step.undoes_source() {
            self.undo(window, cx);
        }
        self.go(to, cx);
    }

    /// The source the default folder source is, if `service` is the one it speaks.
    fn default_source(service: &ServiceInfo, cx: &App) -> Option<SourceInfo> {
        SourcesModel::global(cx)
            .read(cx)
            .sources()
            .iter()
            .find(|source| source.permanent && source.service.id == service.id)
            .cloned()
    }

    /// Open the service's form; the default folder source is filled in rather than doubled.
    fn open_service(&mut self, service: ServiceInfo, window: &mut Window, cx: &mut Context<Self>) {
        let default = Self::default_source(&service, cx);
        self.fill_previous = default.as_ref().map(settings_sources::folders::of);
        self.add.update(cx, |form, cx| {
            form.reset(window, cx);
            match default {
                Some(source) => {
                    let held: Vec<api::schema::FieldValue> = source
                        .settings
                        .iter()
                        .filter_map(|field| {
                            field
                                .value
                                .clone()
                                .map(|value| api::schema::FieldValue::new(field.key.clone(), value))
                        })
                        .collect();
                    form.fill_into(Some(source.id.clone()), cx);
                    form.select_with(service, &held, window, cx);
                }
                None => form.select(service, window, cx),
            }
        });
        self.go(Step::Connect, cx);
        self.add.update(cx, |form, cx| form.focus(window, cx));
    }

    fn take_baseline(&mut self, cx: &mut Context<Self>) {
        self.baseline = None;
        let call = Link::global(cx).call(|api| async move { api.jobs().await });
        cx.spawn(async move |this, cx| {
            let newest = call
                .await
                .map(|jobs| {
                    jobs.iter()
                        .filter_map(|job| flow::job_number(&job.id))
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            this.update(cx, |this, _| this.baseline = Some(newest)).ok();
        })
        .detach();
    }

    fn ours(&self, id: &str, kind: JobKind) -> bool {
        let filling = flow::FILL_KINDS.contains(&kind);
        let after = flow::job_number(id).unwrap_or(0) > self.baseline.unwrap_or(0);
        filling && after && self.added.is_some()
    }

    /// A source was saved: making it active is also what starts whichever sign-in it needs.
    fn adopt(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let active = active_source(cx).map(|source| source.id);
        self.added = Some(Added {
            id: id.clone(),
            previous: active.clone().filter(|active| *active != id),
            filled: self.fill_previous.clone(),
        });
        self.outcome = None;
        self.checked = None;
        self.probed = false;
        cx.notify();
        match active.as_deref() == Some(id.as_str()) {
            true => SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx)),
            false => {
                SourcesModel::global(cx).update(cx, |sources, cx| sources.switch(id, window, cx))
            }
        }
    }

    /// Connect ends only once the saved source is active, signed in and answering.
    fn connecting(&mut self, cx: &mut Context<Self>) {
        let Some(added) = self.added.as_ref() else {
            return;
        };
        let Some(source) = active_source(cx).filter(|source| source.id == added.id) else {
            return;
        };
        match connect_next(&source, self.probed) {
            ConnectNext::Advance => {
                self.go(Step::Scan, cx);
                self.sources_moved(cx);
            }
            ConnectNext::Probe => {
                self.probed = true;
                self.retry(cx);
            }
            ConnectNext::Wait => {}
        }
    }

    /// Sources moved: once ours is active, look for the fill it may already have finished.
    fn sources_moved(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        if self.step == Step::Connect {
            self.connecting(cx);
            return;
        }
        if !self.step.past_connect() || self.outcome.is_some() {
            return;
        }
        let Some(added) = self.added.as_ref() else {
            return;
        };
        let Some(source) = active_source(cx).filter(|source| source.id == added.id) else {
            return;
        };
        let ready = ready_to_sync(&source);
        if self.checked == Some(ready) {
            return;
        }
        self.checked = Some(ready);
        self.check_jobs(source, cx);
    }

    /// A fill that ended before the guide was listening still counts; a source nobody filled is filled now.
    fn check_jobs(&mut self, source: SourceInfo, cx: &mut Context<Self>) {
        let call = Link::global(cx).call(|api| async move { api.jobs().await });
        cx.spawn(async move |this, cx| {
            let Ok(jobs) = call.await else { return };
            this.update(cx, |this, cx| {
                let ours: Vec<JobState> = jobs
                    .iter()
                    .filter(|job| this.ours(&job.id, job.kind))
                    .map(|job| job.state)
                    .collect();
                match flow::fill(&ours) {
                    flow::Fill::Running => return,
                    flow::Fill::Ended { ok } => return this.settle(ok, cx),
                    flow::Fill::NotStarted => {}
                }
                if !ready_to_sync(&source) {
                    return;
                }
                let kinds =
                    flow::fill_kinds(source.capabilities.scan_folders, source.capabilities.sync);
                if kinds.is_empty() {
                    return this.settle(true, cx);
                }
                JobsModel::global(cx).update(cx, |jobs, cx| {
                    for kind in kinds {
                        jobs.start(*kind, cx);
                    }
                });
            })
            .ok();
        })
        .detach();
    }

    /// One of the fill's jobs ended; the fill is over only once none of the others still runs.
    fn job_finished(&mut self, event: &JobEvent, cx: &mut Context<Self>) {
        if !self.step.past_connect() || self.outcome.is_some() || !self.ours(&event.id, event.kind)
        {
            return;
        }
        if let Some(source) = self.added_source(cx) {
            self.check_jobs(source, cx);
        }
    }

    /// The fill ended: count what it left, and a good end is setup done.
    fn settle(&mut self, ok: bool, cx: &mut Context<Self>) {
        if ok {
            SetupModel::global(cx).update(cx, |setup, cx| setup.complete(cx));
        }
        let link = Link::global(cx).clone();
        let tracks = link.call(|api| async move {
            api.tracks(
                TrackFilter::default(),
                Page {
                    offset: 0,
                    limit: 1,
                },
            )
            .await
        });
        let albums = link.call(|api| async move {
            api.albums(
                Default::default(),
                Page {
                    offset: 0,
                    limit: 1,
                },
            )
            .await
        });
        cx.spawn(async move |this, cx| {
            let tracks = tracks.await.map(|page| page.total).unwrap_or(0);
            let albums = albums.await.map(|page| page.total).unwrap_or(0);
            this.update(cx, |this, cx| {
                this.outcome = Some(Outcome { ok, tracks, albums });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Back off the scan: the source goes, the one before it comes back, and the form is as it was.
    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(added) = self.added.take() else {
            return;
        };
        self.outcome = None;
        self.checked = None;
        self.probed = false;
        let link = Link::global(cx).clone();
        let undone = added.clone();
        let call = link.call(move |api| async move {
            if let Some(previous) = undone.previous.clone() {
                api.switch_source(previous).await?;
            }
            match undone.filled {
                Some(folders) => {
                    let value = api::schema::FieldValue::new(
                        settings_sources::folders::KEY,
                        api::schema::encode_directories(&folders),
                    );
                    api.set_source_settings(undone.id, vec![value]).await?;
                }
                None => api.delete_source(undone.id).await?,
            }
            Ok(())
        });
        cx.spawn(async move |_, cx| {
            if let Err(error) = call.await {
                tracing::warn!(%error, "could not take the guide's source away again");
            }
            cx.update(|cx| {
                SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx));
                ConfigModel::global(cx).update(cx, |config, cx| config.reload(cx));
            });
        })
        .detach();
        let filled = added.filled.is_some().then(|| added.id.clone());
        self.add.update(cx, |form, cx| {
            form.restore_last(window, cx);
            form.fill_into(filled, cx);
        });
    }

    fn start_listening(&mut self, cx: &mut Context<Self>) {
        SetupModel::global(cx).update(cx, |setup, cx| {
            setup.complete(cx);
            setup.close(cx);
        });
        Navigator::navigate(Route::Library, cx);
    }

    fn skip(&mut self, cx: &mut Context<Self>) {
        SetupModel::global(cx).update(cx, |setup, cx| setup.skip(cx));
    }

    /// The added source, as the daemon now reports it.
    fn added_source(&self, cx: &App) -> Option<SourceInfo> {
        let id = &self.added.as_ref()?.id;
        SourcesModel::global(cx)
            .read(cx)
            .sources()
            .iter()
            .find(|source| &source.id == id)
            .cloned()
    }
}

impl Render for FirstRun {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _timing = profile::render("first_run");
        components::scroll::smooth("first-run-scroll", &self.scroll, window, cx);
        let step = self.step;
        let body = match step {
            Step::Welcome => self.welcome_step(cx),
            Step::Pick => self.pick_step(window, cx),
            Step::Connect => self.connect_step(cx),
            Step::Scan => self.sync_step(window, cx),
            Step::Appearance => self.appearance_step(cx),
            Step::Playback => self.playback_step(cx),
            Step::Integrations => self.services_step(cx),
            Step::Privacy => self.privacy_step(cx),
            Step::Ready => self.ready_step(cx),
        };
        let subtitle = match step {
            Step::Welcome => t("onboarding_welcome_blurb"),
            _ => t("first_run_subtitle"),
        };
        let key = format!("{}-{}", step.key(), self.run);
        let added = self.added.is_some();
        let mut frame = StepFrame::new(key, t("first_run_title"), subtitle)
            .when_some(step.index(), |frame, index| {
                frame.stepper(Stepper::new(flow::RAIL.map(t)).current(index))
            })
            .when(step.back(added).is_some(), |frame| {
                frame.on_back(cx.listener(|this: &mut Self, _, window, cx| this.back(window, cx)))
            })
            .when(step != Step::Ready, |frame| {
                frame.on_skip(cx.listener(|this: &mut Self, _, _, cx| this.skip(cx)))
            });
        frame = match step {
            Step::Welcome => frame.next(
                t("continue"),
                false,
                cx.listener(|this: &mut Self, _, _, cx| this.go(Step::Pick, cx)),
            ),
            // Picking a card is the way on; only music already in the library, as on a rerun, may be passed by.
            Step::Pick => match active_source(cx).is_some_and(|source| has_music(&source))
                && self.tracks.read(cx).value().is_some_and(|total| *total > 0)
            {
                true => frame.next(
                    t("continue"),
                    false,
                    cx.listener(|this: &mut Self, _, _, cx| this.go(Step::Appearance, cx)),
                ),
                false => frame,
            },
            Step::Connect if self.added.is_some() => frame.next(
                t("continue"),
                self.connection_blocked(cx).is_none(),
                cx.listener(|this: &mut Self, _, _, cx| this.retry(cx)),
            ),
            Step::Connect => frame.next(
                t("continue"),
                self.add.read(cx).is_saving(),
                cx.listener(|this: &mut Self, _, window, cx| {
                    this.add.update(cx, |form, cx| form.save(window, cx))
                }),
            ),
            // The fill carries on in the background, so the rest of the guide need not wait for it.
            Step::Scan => frame.next(
                t("continue"),
                false,
                cx.listener(|this: &mut Self, _, _, cx| this.go(Step::Appearance, cx)),
            ),
            Step::Appearance => frame.next(
                t("continue"),
                false,
                cx.listener(|this: &mut Self, _, _, cx| this.go(Step::Playback, cx)),
            ),
            Step::Playback => frame.next(
                t("continue"),
                false,
                cx.listener(|this: &mut Self, _, _, cx| this.go(Step::Integrations, cx)),
            ),
            Step::Integrations => frame.next(
                t("continue"),
                false,
                cx.listener(|this: &mut Self, _, _, cx| this.go(Step::Privacy, cx)),
            ),
            Step::Privacy => frame.next(
                t("continue"),
                false,
                cx.listener(|this: &mut Self, _, _, cx| this.go(Step::Ready, cx)),
            ),
            Step::Ready => frame.next(
                t("onboarding_start_listening"),
                false,
                cx.listener(|this: &mut Self, _, _, cx| this.start_listening(cx)),
            ),
        };
        v_flex()
            .id("first-run")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .bg(cx.theme().background)
            .child(frame.child(body))
    }
}

fn active_source(cx: &App) -> Option<SourceInfo> {
    SourcesModel::global(cx).read(cx).active().cloned()
}

/// Nothing can be read from a source still waiting on a sign-in.
fn ready_to_sync(source: &SourceInfo) -> bool {
    source.authenticated || source.anonymous
}

/// A source that can already fill a library: signed in, and pointed at folders if it reads them.
fn has_music(source: &SourceInfo) -> bool {
    ready_to_sync(source)
        && (!source.capabilities.scan_folders || !settings_sources::folders::of(source).is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectNext {
    Wait,
    Probe,
    Advance,
}

/// Only a signed-in source the daemon has seen answer moves the guide on to its scan.
fn connect_next(source: &SourceInfo, probed: bool) -> ConnectNext {
    if !ready_to_sync(source) {
        return ConnectNext::Wait;
    }
    match source.state {
        Some(SourceState::Online) => ConnectNext::Advance,
        _ if !probed => ConnectNext::Probe,
        _ => ConnectNext::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in(authenticated: bool) -> SourceInfo {
        SourceInfo {
            authenticated,
            ..Default::default()
        }
    }

    #[test]
    fn a_source_waiting_on_a_sign_in_is_not_ready() {
        assert!(!ready_to_sync(&signed_in(false)));
        assert!(ready_to_sync(&signed_in(true)));
    }

    #[test]
    fn music_is_passed_by_only_when_a_source_can_already_fill_the_library() {
        assert!(!has_music(&signed_in(false)));
        assert!(has_music(&signed_in(true)));
        let mut folders = signed_in(true);
        folders.capabilities.scan_folders = true;
        assert!(!has_music(&folders));
        folders.settings = vec![api::schema::FieldSpec {
            key: settings_sources::folders::KEY.into(),
            value: Some(api::schema::encode_directories(&["/music".into()])),
            ..Default::default()
        }];
        assert!(has_music(&folders));
    }

    fn in_state(state: Option<SourceState>) -> SourceInfo {
        SourceInfo {
            state,
            ..signed_in(true)
        }
    }

    #[test]
    fn connect_holds_until_the_source_answers() {
        let mut waiting = in_state(Some(SourceState::Online));
        waiting.authenticated = false;
        assert_eq!(connect_next(&waiting, false), ConnectNext::Wait);
        assert_eq!(connect_next(&in_state(None), false), ConnectNext::Probe);
        assert_eq!(connect_next(&in_state(None), true), ConnectNext::Wait);
        assert_eq!(
            connect_next(&in_state(Some(SourceState::Checking)), true),
            ConnectNext::Wait
        );
        assert_eq!(
            connect_next(&in_state(Some(SourceState::Offline)), true),
            ConnectNext::Wait
        );
        assert_eq!(
            connect_next(&in_state(Some(SourceState::AuthExpired)), true),
            ConnectNext::Wait
        );
        assert_eq!(
            connect_next(&in_state(Some(SourceState::Online)), true),
            ConnectNext::Advance
        );
    }
}
