//! What the daemon is working on: every running job, its progress, and how to stop it.

use api::{JobKind, JobProgress};
use components::format::t;
use components::{Badge, BadgeTone, Chip, ChipTone, JobLayout, JobLine, StatusRow, style};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled, Subscription, Window, div,
};
use store::{Finished, JobsModel};

use crate::{DONE, FAILED, header};
use components::EmptyState;

/// The activity popover's body.
pub struct ActivityList {
    _subscriptions: Vec<Subscription>,
}

impl ActivityList {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let jobs = JobsModel::global(cx);
        Self {
            _subscriptions: vec![cx.observe(&jobs, |_, _, cx| cx.notify())],
        }
    }

    fn job(&self, progress: &JobProgress, cx: &App) -> AnyElement {
        let id = progress.id.clone();
        div()
            .w_full()
            .px_3()
            .py_2()
            .child(
                JobLine::new(progress)
                    .layout(JobLayout::Stacked)
                    .title(store::kind_label(progress.kind))
                    .stopping(JobsModel::global(cx).read(cx).is_stopping(&progress.id))
                    .on_stop(move |_, cx| {
                        let id = id.clone();
                        JobsModel::global(cx).update(cx, |jobs, cx| jobs.cancel(id, cx));
                    }),
            )
            .into_any_element()
    }

    /// A job that has just stopped, with what it ended as.
    fn outcome(&self, finished: &Finished, cx: &App) -> AnyElement {
        let (glyph, ink) = match finished.ok {
            true => (DONE, cx.theme().success),
            false => (FAILED, cx.theme().danger),
        };
        let detail = finished
            .message
            .clone()
            .unwrap_or_else(|| match finished.ok {
                true => t("job_done"),
                false => t("job_stopped"),
            });
        let row = StatusRow::new(format!("outcome-{}", finished.id))
            .glyph(glyph, ink)
            .title(store::kind_label(finished.kind))
            .detail(detail, None);
        match finished.automatic {
            // TODO(i18n): no "scheduled" key yet; "Automatic" is the nearest the locale has.
            true => row
                .trailing(Badge::new(t("playback_browser_auto")).tone(BadgeTone::Neutral))
                .into_any_element(),
            false => row.into_any_element(),
        }
    }
}

impl Render for ActivityList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let jobs = JobsModel::global(cx);
        let (rows, scan_running) = {
            let jobs = jobs.read(cx);
            let rows: Vec<AnyElement> = jobs
                .running()
                .iter()
                .map(|progress| self.job(progress, cx))
                .chain(
                    jobs.recent()
                        .iter()
                        .map(|finished| self.outcome(finished, cx)),
                )
                .collect();
            (rows, jobs.progress(JobKind::Scan).is_some())
        };
        v_flex()
            .w_full()
            .child(header(t("activity"), None, cx))
            .when(rows.is_empty(), |body| {
                body.child(EmptyState::new(t("activity_idle")).compact())
            })
            .when(!rows.is_empty(), |body| {
                body.child(
                    v_flex()
                        .id("activity-rows")
                        .w_full()
                        .py_1()
                        .max_h(crate::LIST_HEIGHT)
                        .overflow_y_scroll()
                        .children(rows),
                )
            })
            .when(!scan_running, |body| {
                body.child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(style::white(style::BORDER_SOFT, cx))
                        .child(
                            Chip::new("start-scan")
                                .label(t("rescan_library"))
                                .tone(ChipTone::Ink(cx.theme().primary))
                                .on_click(|_, _, cx| {
                                    JobsModel::global(cx)
                                        .update(cx, |jobs, cx| jobs.start(JobKind::Scan, cx));
                                }),
                        ),
                )
            })
    }
}
