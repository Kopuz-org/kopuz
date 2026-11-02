//! The first scan or sync: what the daemon reports about it, what stops it, and what it found.

use api::{SourceInfo, SourceState};
use components::format::{t, t_with};
use components::{
    Callout, CalloutTone, Heading, JobLine, ProgressBar, SectionHeading, StatusLine, icons, style,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled,
    Window, div,
};
use link::Link;
use store::{JobsModel, SourcesModel};

use crate::{FirstRun, Outcome, ready_to_sync};

/// `h-1.5`, the bar under the heading.
const BAR: gpui_kit::Pixels = gpui_kit::px(6.);

/// What is in the way of the fill, in the order it is worth saying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Blocker {
    /// The internet is down and this source lives on it.
    NetworkDown,
    /// Named by its address when it has one, so only then is there an address to edit.
    Unreachable {
        name: String,
        address: Option<String>,
    },
    SignedOut,
    WaitingSignIn,
}

fn blocker(source: &SourceInfo, internet_down: bool) -> Option<Blocker> {
    if internet_down && !source.works_offline {
        return Some(Blocker::NetworkDown);
    }
    // The daemon probes a source with no credentials yet as unreachable, so the sign-in comes first.
    if !ready_to_sync(source) {
        return Some(Blocker::WaitingSignIn);
    }
    match source.state {
        Some(SourceState::Offline) => Some(Blocker::Unreachable {
            name: source.name.clone(),
            address: source.detail.clone().filter(|detail| !detail.is_empty()),
        }),
        Some(SourceState::AuthExpired) => Some(Blocker::SignedOut),
        _ => None,
    }
}

/// The bar's fill across the running jobs: their mean once each knows its total, else a sweep.
fn share(fractions: &[Option<f32>]) -> Option<f32> {
    let known: Vec<f32> = fractions.iter().copied().collect::<Option<_>>()?;
    (!known.is_empty()).then(|| known.iter().sum::<f32>() / known.len() as f32)
}

impl FirstRun {
    /// What still stands between the saved source and its first fill.
    pub(crate) fn connection_blocked(&self, cx: &App) -> Option<Blocker> {
        let internet_down = SourcesModel::global(cx).read(cx).network().internet_down;
        self.added_source(cx)
            .and_then(|source| blocker(&source, internet_down))
    }

    /// The saved source while it connects: its sign-in, its address trouble, or the wait for its first answer.
    pub(crate) fn connection_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let source = self.added_source(cx);
        let line = match self.connection_blocked(cx) {
            Some(blocker) => self.blocker_line(blocker, cx),
            None => StatusLine::new(store::SourceStatus::Connecting).into_any_element(),
        };
        v_flex()
            .w_full()
            .gap_3()
            .children(source.map(|source| SectionHeading::new(source.name).heading(Heading::Step)))
            .child(line)
            .into_any_element()
    }

    pub(crate) fn sync_step(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let source = self.added_source(cx);
        let internet_down = SourcesModel::global(cx).read(cx).network().internet_down;
        let blocked = source
            .as_ref()
            .and_then(|source| blocker(source, internet_down));
        let column = v_flex()
            .w_full()
            .gap_3()
            .child(SectionHeading::new(t("first_run_sync_title")).heading(Heading::Step));
        if let Some(outcome) = self.outcome {
            let folders = source
                .as_ref()
                .is_some_and(|source| source.capabilities.scan_folders);
            return column
                .child(self.result(outcome, folders, cx))
                .into_any_element();
        }
        let column = match blocked {
            Some(blocker) => column.child(self.blocker_line(blocker, cx)),
            None => {
                // Every job filling the source, scan or syncs alike: a streaming library's tracks arrive through more than one.
                let running: Vec<api::JobProgress> = JobsModel::global(cx)
                    .read(cx)
                    .running()
                    .iter()
                    .filter(|job| self.ours(&job.id, job.kind))
                    .cloned()
                    .collect();
                let fractions: Vec<Option<f32>> =
                    running.iter().map(components::job::fraction).collect();
                column
                    .child(crate::ui::hint(t("first_run_sync_hint"), cx))
                    .child(ProgressBar::new("first-run-progress", share(&fractions)).height(BAR))
                    .children(running.iter().map(|progress| {
                        div()
                            .id(gpui_kit::ElementId::Name(
                                format!("first-run-job-{}", progress.id).into(),
                            ))
                            .text_xs()
                            .text_color(style::white(style::TEXT_SECONDARY, cx))
                            .child(JobLine::new(progress))
                    }))
            }
        };
        column.into_any_element()
    }

    fn blocker_line(&mut self, blocker: Blocker, cx: &mut Context<Self>) -> AnyElement {
        match blocker {
            Blocker::NetworkDown => Callout::new(t("error_network_down"))
                .id("first-run-network-down")
                .tone(CalloutTone::Warning)
                .icon(icons::WARNING)
                .into_any_element(),
            Blocker::Unreachable { name, address } => v_flex()
                .w_full()
                .gap_3()
                .child(
                    Callout::new(t_with(
                        "onboarding_probe_unreachable",
                        &[("url", address.clone().unwrap_or(name).into())],
                    ))
                    .id("first-run-unreachable")
                    .tone(CalloutTone::Warning)
                    .icon(icons::WARNING),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .when(address.is_some(), |row| {
                            row.child(
                                Button::new("first-run-edit-url")
                                    .outline()
                                    .small()
                                    .label(t("edit_url"))
                                    .on_click(cx.listener(|this: &mut Self, _, window, cx| {
                                        this.back(window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("first-run-retry")
                                .outline()
                                .small()
                                .label(t("retry"))
                                .on_click(cx.listener(|this: &mut Self, _, _, cx| this.retry(cx))),
                        ),
                )
                .into_any_element(),
            Blocker::SignedOut | Blocker::WaitingSignIn => {
                let label = match blocker {
                    Blocker::SignedOut => t("library_sign_in"),
                    _ => t("first_run_waiting_sign_in"),
                };
                StatusLine::new(store::SourceStatus::Disconnected)
                    .label(label)
                    .on_sign_in(
                        cx.listener(|this: &mut Self, _, window, cx| {
                            this.sign_in_again(window, cx)
                        }),
                    )
                    .into_any_element()
            }
        }
    }

    fn result(&mut self, outcome: Outcome, folders: bool, cx: &mut Context<Self>) -> AnyElement {
        let summary = t_with(
            "onboarding_ready_summary",
            &[
                ("tracks", outcome.tracks.into()),
                ("albums", outcome.albums.into()),
            ],
        );
        let empty = outcome.tracks == 0 && folders;
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        icons::icon(match outcome.ok && !empty {
                            true => icons::CHECK,
                            false => icons::WARNING,
                        })
                        .size(gpui_kit::px(16.)),
                    )
                    .child(div().text_sm().child(summary)),
            )
            .when(empty, |column| {
                column
                    .child(crate::ui::hint(t("onboarding_probe_empty"), cx))
                    .child(
                        Button::new("first-run-other-folder")
                            .outline()
                            .small()
                            .label(t("library_add_folder"))
                            .on_click(
                                cx.listener(|this: &mut Self, _, window, cx| this.back(window, cx)),
                            )
                            .self_start(),
                    )
            })
            .into_any_element()
    }

    /// Ask the daemon to probe the source again; its answer arrives as a status event.
    pub(crate) fn retry(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.added.as_ref().map(|added| added.id.clone()) else {
            return;
        };
        let call = Link::global(cx).call(move |api| async move { api.validate_source(id).await });
        cx.spawn(async move |_, cx| {
            if let Err(error) = call.await {
                tracing::debug!(%error, "source probe failed");
            }
            cx.update(|cx| {
                SourcesModel::global(cx).update(cx, |sources, cx| sources.refetch(cx));
            });
        })
        .detach();
    }

    /// Ask for the sign-in again, which is the daemon's browser flow or the registered form.
    fn sign_in_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.added_source(cx) else {
            return;
        };
        store::sign_in_again(&source, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(state: Option<SourceState>, works_offline: bool) -> SourceInfo {
        SourceInfo {
            name: "NAS".into(),
            authenticated: true,
            state,
            works_offline,
            detail: Some("http://nas:4533".into()),
            ..Default::default()
        }
    }

    #[test]
    fn the_bar_fills_only_once_every_job_knows_its_total() {
        assert_eq!(share(&[]), None);
        assert_eq!(share(&[Some(0.5), None]), None);
        assert_eq!(share(&[Some(1.0), Some(0.5)]), Some(0.75));
    }

    #[test]
    fn an_offline_server_names_its_address() {
        assert_eq!(
            blocker(&source(Some(SourceState::Offline), false), false),
            Some(Blocker::Unreachable {
                name: "NAS".into(),
                address: Some("http://nas:4533".into()),
            })
        );
    }

    #[test]
    fn a_source_without_an_address_is_named_and_has_none_to_edit() {
        let mut streaming = source(Some(SourceState::Offline), false);
        streaming.detail = None;
        assert_eq!(
            blocker(&streaming, false),
            Some(Blocker::Unreachable {
                name: "NAS".into(),
                address: None,
            })
        );
    }

    #[test]
    fn a_source_waiting_on_its_sign_in_is_not_called_unreachable() {
        let mut waiting = source(Some(SourceState::Offline), false);
        waiting.authenticated = false;
        waiting.detail = None;
        assert_eq!(blocker(&waiting, false), Some(Blocker::WaitingSignIn));
    }

    #[test]
    fn a_dead_internet_outranks_the_server_it_cut_off() {
        assert_eq!(
            blocker(&source(Some(SourceState::Offline), false), true),
            Some(Blocker::NetworkDown)
        );
        assert_eq!(blocker(&source(None, true), true), None);
    }

    #[test]
    fn an_expired_sign_in_asks_for_another() {
        assert_eq!(
            blocker(&source(Some(SourceState::AuthExpired), false), false),
            Some(Blocker::SignedOut)
        );
    }

    #[test]
    fn a_source_never_signed_in_waits_for_it() {
        let mut waiting = source(None, false);
        waiting.authenticated = false;
        assert_eq!(blocker(&waiting, false), Some(Blocker::WaitingSignIn));
    }
}
