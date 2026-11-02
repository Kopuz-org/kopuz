//! Picking a service, in its group, and answering the shared "add source" form for it.

use api::ServiceInfo;
use components::format::{t, text};
use components::source_mark::{self, SourceMark};
use components::{ChoiceCard, EmptyState, Heading, SectionHeading};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled, Window, px,
};

use crate::{FirstRun, flow};

impl FirstRun {
    pub(crate) fn pick_step(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let services: Vec<ServiceInfo> =
            self.services.read(cx).value().cloned().unwrap_or_default();
        let groups = flow::grouped(&services);
        let mut first = None;
        let mut column = v_flex().w_full().gap_6().when(services.is_empty(), |step| {
            step.child(EmptyState::new(t("first_run_no_services")).compact())
        });
        for (group, members) in groups {
            let mut cards = v_flex().w_full().gap_3().child(
                v_flex()
                    .gap_1()
                    .child(SectionHeading::new(t(group.heading())).heading(Heading::Step))
                    .children(group.hint().map(|hint| crate::ui::hint(t(hint), cx))),
            );
            for service in members {
                let id = SharedString::from(format!("first-run-service-{}", service.id));
                let handle = components::focus::handle(id.clone(), window, cx);
                first.get_or_insert_with(|| handle.clone());
                let accent = source_mark::accent(Some(&service), cx);
                let summary = text(&service.summary);
                let detail = match service.experimental {
                    true => SharedString::from(format!("{summary} ({})", t("experimental"))),
                    false => summary,
                };
                let entity = cx.entity().downgrade();
                let chosen = service.clone();
                cards = cards.child(
                    ChoiceCard::new(id, text(&service.name))
                        .glyph(SourceMark::new(&service).size(px(18.)))
                        .accent(accent)
                        .detail((!detail.is_empty()).then_some(detail))
                        .focus(handle)
                        .on_activate(move |window, cx| {
                            entity
                                .update(cx, |this, cx| {
                                    this.open_service(chosen.clone(), window, cx)
                                })
                                .ok();
                        }),
                );
            }
            column = column.child(cards);
        }
        if self.focus_first
            && let Some(first) = first
        {
            self.focus_first = false;
            window.defer(cx, move |window, cx| window.focus(&first, cx));
        }
        column.into_any_element()
    }

    pub(crate) fn connect_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.added.is_some() {
            return self.connection_step(cx);
        }
        let service = self.add.read(cx).service().cloned();
        v_flex()
            .w_full()
            .gap_3()
            .children(
                service
                    .map(|service| SectionHeading::new(text(&service.name)).heading(Heading::Step)),
            )
            .child(self.add.clone())
            .into_any_element()
    }
}
