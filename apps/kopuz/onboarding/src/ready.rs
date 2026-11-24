//! The last card: what the library now holds, and the way into it.

use components::format::{t, t_with};
use components::{icons, style};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{AnyElement, Context, FontWeight, IntoElement, ParentElement as _, Styled, div, px};

use crate::FirstRun;

impl FirstRun {
    pub(crate) fn ready_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // The fill may still be running by now, and a guide that added no source has nothing to count.
        let summary = match (self.outcome, self.added.is_some()) {
            (Some(outcome), _) => Some(t_with(
                "onboarding_ready_summary",
                &[
                    ("tracks", outcome.tracks.into()),
                    ("albums", outcome.albums.into()),
                ],
            )),
            (None, true) => Some(t("first_run_sync_hint")),
            (None, false) => None,
        };
        let failed = self.outcome.is_some_and(|outcome| !outcome.ok);
        let (glyph, ink) = match failed {
            true => (icons::WARNING, cx.theme().warning),
            false => (icons::CHECK, cx.theme().primary),
        };
        crate::ui::card(cx)
            .items_center()
            .gap_3()
            .px_6()
            .py_8()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size_12()
                    .rounded_full()
                    .bg(style::accent(0.15, cx))
                    .child(icons::icon(glyph).size(px(22.)).text_color(ink)),
            )
            .children(summary.map(|summary| {
                v_flex().items_center().gap_1().child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_center()
                        .child(summary),
                )
            }))
            .when(failed, |card| {
                card.child(crate::ui::hint(t("onboarding_fill_failed"), cx))
            })
            .into_any_element()
    }
}
