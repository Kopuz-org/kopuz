//! What a running job shows: how far along it is, and how to stop it.

use std::rc::Rc;

use api::JobProgress;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString,
    Styled as _, Window, div, px,
};
use store::JobsModel;

use crate::format::{t, t_with};
use crate::{Badge, BadgeTone, Chip, ChipTone, skin, style};

/// Material's linear progress indicator: 4dp tall.
const M3_PROGRESS: Pixels = px(4.);
/// Material's secondary container, as the accent's tonal wash.
const M3_CONTAINER: f32 = 0.24;
/// How far along a job is, or nothing while it cannot say.
pub fn fraction(progress: &JobProgress) -> Option<f32> {
    let total = progress.total.filter(|total| *total > 0)?;
    let current = progress.current?.min(total);
    Some(current as f32 / total as f32)
}

/// The thin accent bar under a job: filled to `fraction`, or sweeping while there is none.
#[derive(IntoElement)]
pub struct ProgressBar {
    id: SharedString,
    fraction: Option<f32>,
    height: Pixels,
}

impl ProgressBar {
    pub fn new(id: impl Into<SharedString>, fraction: Option<f32>) -> Self {
        Self {
            id: id.into(),
            fraction,
            height: px(3.),
        }
    }

    /// The bar for a job, read off its progress.
    pub fn job(progress: &JobProgress) -> Self {
        Self::new(format!("job-progress-{}", progress.id), fraction(progress))
    }

    pub fn height(mut self, height: Pixels) -> Self {
        self.height = height;
        self
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // Material's linear progress: 4dp, fully rounded, primary on its tonal container.
        let material = skin::material();
        let bar = Progress::new(self.id)
            .xsmall()
            .h(match material {
                true => M3_PROGRESS,
                false => self.height,
            })
            .map(|bar| match self.fraction {
                Some(fraction) => bar.value(fraction * 100.),
                None => bar.loading(true),
            })
            .color(cx.theme().primary);
        div()
            .w_full()
            .when(material, |track| {
                track
                    .rounded_full()
                    .overflow_hidden()
                    .bg(cx.theme().primary.opacity(M3_CONTAINER))
            })
            .child(bar)
    }
}

/// A danger chip that stops a job, which reads "Stopping" once it has been asked to.
#[derive(IntoElement)]
pub struct JobStop {
    id: String,
}

impl JobStop {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for JobStop {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let stopping = JobsModel::global(cx).read(cx).is_stopping(&self.id);
        let id = self.id.clone();
        stop_control(
            &self.id,
            stopping,
            move |_, cx| {
                let id = id.clone();
                JobsModel::global(cx).update(cx, |jobs, cx| jobs.cancel(id, cx));
            },
            cx,
        )
    }
}

/// The danger chip that stops a job, or the muted word once it has been asked to.
fn stop_control(
    id: &str,
    stopping: bool,
    stop: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    if stopping {
        return Badge::new(t("stopping"))
            .tone(BadgeTone::Muted)
            .into_any_element();
    }
    Chip::new(SharedString::from(format!("stop-job-{id}")))
        .label(t("stop"))
        .tone(ChipTone::Ink(cx.theme().danger))
        .on_click(move |_, window, cx| stop(window, cx))
        .into_any_element()
}

/// How many of a job's steps are done, or nothing while it cannot say; `of` words it for a line of prose.
pub fn counted(progress: &JobProgress, of: bool) -> Option<SharedString> {
    let total = progress.total.filter(|total| *total > 0)?;
    let done = progress.current.unwrap_or(0);
    Some(match of {
        true => t_with(
            "first_run_progress",
            &[("current", done.into()), ("total", total.into())],
        ),
        false => SharedString::from(format!("{done} / {total}")),
    })
}

/// How a job line sits: one row of phase, file and count, or a titled block with its bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum JobLayout {
    #[default]
    Inline,
    Stacked,
}

type OnStop = Rc<dyn Fn(&mut Window, &mut App)>;

/// A running job as the daemon reports it, with a stop control when the owner wires one.
#[derive(IntoElement)]
pub struct JobLine {
    progress: JobProgress,
    layout: JobLayout,
    title: Option<SharedString>,
    fallback: Option<SharedString>,
    stopping: bool,
    on_stop: Option<OnStop>,
}

impl JobLine {
    pub fn new(progress: &JobProgress) -> Self {
        Self {
            progress: progress.clone(),
            layout: JobLayout::Inline,
            title: None,
            fallback: None,
            stopping: false,
            on_stop: None,
        }
    }

    pub fn layout(mut self, layout: JobLayout) -> Self {
        self.layout = layout;
        self
    }

    /// The heading of a stacked line, such as what kind of job it is.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// What an inline line says while the job names no phase.
    pub fn idle_phase(mut self, phase: impl Into<SharedString>) -> Self {
        self.fallback = Some(phase.into());
        self
    }

    /// A stop was already asked for, so the control reads as such.
    pub fn stopping(mut self, stopping: bool) -> Self {
        self.stopping = stopping;
        self
    }

    /// Without it the line has no stop control.
    pub fn on_stop(mut self, stop: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_stop = Some(Rc::new(stop));
        self
    }
}

impl RenderOnce for JobLine {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let progress = self.progress;
        let message = progress
            .message
            .clone()
            .filter(|message| !message.is_empty())
            .map(SharedString::from);
        let phase = (!progress.phase.is_empty())
            .then(|| SharedString::from(progress.phase.clone()))
            .or(self.fallback);
        // TODO(i18n): no "scheduled" key yet; "Automatic" is the nearest the locale has.
        let automatic = progress
            .automatic
            .then(|| Badge::new(t("playback_browser_auto")).tone(BadgeTone::Neutral));
        let stop = self.on_stop.map(|stop| {
            stop_control(
                &progress.id,
                self.stopping,
                move |window, cx| stop(window, cx),
                cx,
            )
        });
        match self.layout {
            JobLayout::Inline => h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .children(phase.map(|phase| div().flex_shrink_0().child(phase)))
                .children(automatic)
                .child(div().flex_1().min_w_0().truncate().children(message))
                .children(counted(&progress, true).map(|counted| {
                    div()
                        .flex_shrink_0()
                        .text_color(style::slate_500(cx))
                        .child(counted)
                }))
                .children(stop)
                .into_any_element(),
            JobLayout::Stacked => {
                let detail = message.or(phase);
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(cx.theme().foreground)
                                    .children(self.title),
                            )
                            .children(automatic)
                            .children(counted(&progress, false).map(|counted| {
                                div()
                                    .flex_shrink_0()
                                    .text_size(style::text_10())
                                    .text_color(style::white(style::TEXT_MUTED, cx))
                                    .child(counted)
                            }))
                            .children(stop),
                    )
                    .child(ProgressBar::job(&progress))
                    .children(detail.map(|detail| {
                        div()
                            .w_full()
                            .truncate()
                            .text_size(style::text_10())
                            .text_color(style::white(style::TEXT_MUTED, cx))
                            .child(detail)
                    }))
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::JobKind;

    fn progress(current: Option<u64>, total: Option<u64>) -> JobProgress {
        JobProgress {
            id: "job".into(),
            kind: JobKind::Scan,
            phase: String::new(),
            current,
            total,
            message: None,
            automatic: false,
        }
    }

    #[test]
    fn a_job_without_a_count_and_a_total_has_nothing_to_fill() {
        assert_eq!(fraction(&progress(Some(3), None)), None);
        assert_eq!(fraction(&progress(None, Some(10))), None);
        assert_eq!(fraction(&progress(Some(3), Some(0))), None);
    }

    #[test]
    fn a_counted_job_fills_its_share_and_never_past_the_end() {
        assert_eq!(fraction(&progress(Some(5), Some(10))), Some(0.5));
        assert_eq!(fraction(&progress(Some(30), Some(10))), Some(1.));
    }
}
