//! Picking a service, in its group, and answering the shared "add source" form for it.

use components::format::{t, text};
use components::source_mark::{self, SourceMark};
use components::{ChoiceCard, EmptyState, Heading, Loading, SectionHeading};
use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled, Window, px,
};

use crate::{FirstRun, flow};

impl FirstRun {
    pub(crate) fn pick_step(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let query = self.services.read(cx);
        let Some(services) = query.value().filter(|services| !services.is_empty()) else {
            if query.is_loading() {
                return Loading::new().compact().into_any_element();
            }
            if let Some(error) = query.error() {
                let query = self.services.clone();
                return EmptyState::failed(t("first_run_services_failed"), move |_, _, cx| {
                    query.update(cx, |query, cx| query.refresh(cx));
                })
                .detail(store::human(error))
                .compact()
                .into_any_element();
            }
            return if query.value().is_some() {
                EmptyState::new(t("first_run_no_services"))
                    .compact()
                    .into_any_element()
            } else {
                Loading::new().compact().into_any_element()
            };
        };
        let groups = flow::grouped(services);
        let mut first = None;
        let mut column = v_flex().w_full().gap_6();
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
