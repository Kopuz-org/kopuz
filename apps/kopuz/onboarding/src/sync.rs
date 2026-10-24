//! The first scan or sync: what the daemon reports about it, what stops it, and what it found.

use api::{SourceInfo, SourceState};
use components::format::{t, t_with};
use components::{
    Callout, CalloutTone, Heading, JobLine, ProgressBar, SectionHeading, StatusLine, icons, style,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement as _, Styled, Window, div};
use link::Link;
use store::{JobsModel, SourcesModel};

use crate::{FirstRun, Outcome, ready_to_sync};

/// `h-1.5`, the bar under the heading.
const BAR: gpui_kit::Pixels = gpui_kit::px(6.);

/// What is in the way of the fill, in the order it is worth saying.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Blocker {
    /// The internet is down and this source lives on it.
    NetworkDown,
    Unreachable(String),
    SignedOut,
    WaitingSignIn,
}

fn blocker(source: &SourceInfo, internet_down: bool) -> Option<Blocker> {
    if internet_down && !source.works_offline {
        return Some(Blocker::NetworkDown);
    }
    match source.state {
        Some(SourceState::Offline) => {
            return Some(Blocker::Unreachable(
                source.detail.clone().unwrap_or_default(),
            ));
        }
        Some(SourceState::AuthExpired) => return Some(Blocker::SignedOut),
        _ => {}
    }
    (!ready_to_sync(source)).then_some(Blocker::WaitingSignIn)
}

impl FirstRun {
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
                // Whichever library job the daemon started for the source, scan or sync alike.
                let progress = JobsModel::global(cx)
                    .read(cx)
                    .running()
                    .iter()
                    .find(|job| matches!(job.kind, api::JobKind::Scan | api::JobKind::LibrarySync))
                    .cloned();
                let share = progress.as_ref().and_then(components::job::fraction);
                column
                    .child(crate::ui::hint(t("first_run_sync_hint"), cx))
                    .child(ProgressBar::new("first-run-progress", share).height(BAR))
                    .when_some(progress, |step, progress| {
                        step.child(
                            div()
                                .text_xs()
                                .text_color(style::white(style::TEXT_SECONDARY, cx))
                                .child(JobLine::new(&progress)),
                        )
                    })
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
            Blocker::Unreachable(url) => v_flex()
                .w_full()
                .gap_3()
                .child(
                    Callout::new(t_with(
                        "onboarding_probe_unreachable",
                        &[("url", url.into())],
                    ))
                    .id("first-run-unreachable")
                    .tone(CalloutTone::Warning)
                    .icon(icons::WARNING),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("first-run-edit-url")
                                .outline()
                                .small()
                                .label(t("edit_url"))
                                .on_click(cx.listener(|this: &mut Self, _, window, cx| {
                                    this.back(window, cx)
                                })),
                        )
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
    fn retry(&mut self, cx: &mut Context<Self>) {
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
        store::start_sign_in(&source, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(state: Option<SourceState>, works_offline: bool) -> SourceInfo {
        SourceInfo {
            authenticated: true,
            state,
            works_offline,
            detail: Some("http://nas:4533".into()),
            ..Default::default()
        }
    }

    #[test]
    fn an_offline_server_names_its_address() {
        assert_eq!(
            blocker(&source(Some(SourceState::Offline), false), false),
            Some(Blocker::Unreachable("http://nas:4533".into()))
        );
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
