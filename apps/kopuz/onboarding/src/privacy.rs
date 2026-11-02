//! What Kopuz asks of other services on its own, each with a line on what it does and its current answer.

use config::{PlaybackEdit, SettingsEdit, SystemEdit};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement as _, Styled};
use store::ConfigModel;

use crate::FirstRun;
use crate::ui::toggle;

impl FirstRun {
    pub(crate) fn privacy_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let settings = ConfigModel::global(cx).read(cx).config().clone();
        // Only a source that keeps a watch history has one to feed.
        let history = self
            .added_source(cx)
            .is_some_and(|source| source.capabilities.watch_history);
        v_flex()
            .w_full()
            .gap_3()
            .child(
                crate::ui::card(cx)
                    .child(toggle(
                        "auto_check_updates",
                        "auto_check_updates",
                        "onboarding_privacy_updates",
                        settings.system.auto_check_updates,
                        |on| SettingsEdit::System(SystemEdit::AutoCheckUpdates(on)),
                        cx,
                    ))
                    .when(history, |card| {
                        card.child(toggle(
                            "pause_watch_history",
                            "pause_watch_history",
                            "onboarding_privacy_watch_history",
                            settings.playback.pause_watch_history,
                            |on| SettingsEdit::Playback(PlaybackEdit::PauseWatchHistory(on)),
                            cx,
                        ))
                    }),
            )
            .into_any_element()
    }
}
