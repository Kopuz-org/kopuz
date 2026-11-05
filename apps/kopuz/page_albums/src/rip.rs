//! Ripping an audio CD to FLAC: where to, how far along, and how it ended.

use api::JobKind;
use components::dialogs::FormDialog;
use components::format::t;
use components::{ErrorBanner, PathField, ProgressBar, style};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled, Subscription, Task, Window, div,
};
use store::{JobEvent, JobsModel, Link};

/// Open the rip dialog for these tracks of the optical source.
pub(crate) fn open(keys: Vec<String>, window: &mut Window, cx: &mut App) {
    let body = cx.new(|cx| Rip::new(keys, window, cx));
    FormDialog::new(t("cd_rip_flac")).open(body, window, cx);
}

/// The folder the daemon writes into, and what the job it started has said.
struct Rip {
    keys: Vec<String>,
    output: Entity<InputState>,
    // A folder the picker chose, applied on the next frame since the input needs the window.
    picked: Option<String>,
    starting: bool,
    job: Option<String>,
    outcome: Option<Outcome>,
    _start: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

enum Outcome {
    Said(SharedString),
    Failed(SharedString),
}

impl Rip {
    fn new(keys: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let output = cx.new(|cx| InputState::new(window, cx));
        let jobs = JobsModel::global(cx);
        let subscriptions = vec![
            cx.subscribe(&output, |_, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    cx.notify();
                }
            }),
            cx.observe(&jobs, |_, _, cx| cx.notify()),
            cx.subscribe(&jobs, |this, _, event: &JobEvent, cx| {
                this.finished(event, cx)
            }),
        ];
        Self {
            keys,
            output,
            picked: None,
            starting: false,
            job: None,
            outcome: None,
            _start: None,
            _subscriptions: subscriptions,
        }
    }

    fn finished(&mut self, event: &JobEvent, cx: &mut Context<Self>) {
        // The job can end before the call that started it has answered with its id.
        let ours = match &self.job {
            Some(id) => *id == event.id,
            None => self.starting && event.kind == JobKind::AudioRip,
        };
        if !ours {
            return;
        }
        self.job = None;
        self.outcome = Some(match (&event.error, event.ok) {
            (Some(error), _) => Outcome::Failed(error.message.clone().into()),
            (None, true) => Outcome::Said(t("cd_rip_finished")),
            (None, false) => Outcome::Said(t("cd_rip_cancelled")),
        });
        cx.notify();
    }

    fn running<'a>(&self, cx: &'a App) -> Option<&'a api::JobProgress> {
        JobsModel::global(cx).read(cx).progress(JobKind::AudioRip)
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self.output.read(cx).value().trim().to_string();
        if directory.is_empty() || self.keys.is_empty() || self.starting {
            return;
        }
        self.starting = true;
        self.outcome = None;
        cx.notify();
        let keys = self.keys.clone();
        let call =
            Link::global(cx).call(move |api| async move { api.rip_audio(keys, directory).await });
        self._start = Some(cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                match result {
                    // Already over if its outcome arrived first.
                    Ok(job) if this.outcome.is_none() => this.job = Some(job.job_id),
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(%error, "the rip was refused");
                        this.outcome = Some(Outcome::Failed(store::human(&error)));
                    }
                }
                this.starting = false;
                cx.notify();
            })
            .ok();
        }));
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let id = self
            .job
            .clone()
            .or_else(|| self.running(cx).map(|job| job.id.clone()));
        if let Some(id) = id {
            JobsModel::global(cx).update(cx, |jobs, cx| jobs.cancel(id, cx));
        }
    }
}

impl Render for Rip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(path) = self.picked.take() {
            self.output
                .update(cx, |output, cx| output.set_value(path, window, cx));
        }
        let progress = self.running(cx).cloned();
        let busy = self.starting || progress.is_some();
        let empty = self.output.read(cx).value().trim().is_empty();
        let muted = style::white(style::TEXT_MUTED, cx);
        let page = cx.entity().downgrade();
        v_flex()
            .w_full()
            .gap_4()
            .child(div().text_sm().text_color(muted).child(t("cd_rip_help")))
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child(t("cd_output_directory")))
                    .child(
                        PathField::new("rip-output", self.output.read(cx).value())
                            .input(&self.output)
                            .locked(busy)
                            .on_change(move |path, cx| {
                                page.update(cx, |this, cx| {
                                    this.picked = Some(path);
                                    cx.notify();
                                })
                                .ok();
                            }),
                    ),
            )
            .when_some(progress, |body, progress| {
                body.when_some(progress.message.clone(), |body, message| {
                    body.child(div().text_sm().text_color(muted).child(message))
                })
                .child(ProgressBar::job(&progress))
            })
            .map(|body| match &self.outcome {
                Some(Outcome::Failed(error)) => body.child(ErrorBanner::new(error.clone()).icon()),
                Some(Outcome::Said(said)) => {
                    body.child(div().text_sm().text_color(muted).child(said.clone()))
                }
                None => body,
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("rip-close")
                            .ghost()
                            .label(t("close"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(match busy {
                        true => Button::new("rip-cancel")
                            .outline()
                            .label(t("cancel"))
                            .loading(self.starting)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        false => Button::new("rip-start")
                            .primary()
                            .label(t("cd_rip_flac"))
                            .disabled(empty || self.keys.is_empty())
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                    }),
            )
    }
}
