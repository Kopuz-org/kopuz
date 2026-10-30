//! One configured source as a card: its mark, name, status and actions, with room for whatever it publishes.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, FontWeight, InteractiveElement as _, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div,
};

use crate::format::t;
use crate::{Badge, SourceMark, motion, style};

/// `bg-white/8` over `bg-white/5`, the card a source's own row lifts to under the pointer.
const HOVER: f32 = 0.08;

/// A source's row: identity on the left, its actions on the right, and its body underneath.
#[derive(IntoElement)]
pub struct SourceCard {
    id: SharedString,
    name: SharedString,
    mark: Option<SourceMark>,
    service: Option<SharedString>,
    detail: Option<SharedString>,
    active: bool,
    status: Option<AnyElement>,
    actions: Vec<AnyElement>,
    body: Vec<AnyElement>,
}

impl SourceCard {
    pub fn new(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            mark: None,
            service: None,
            detail: None,
            active: false,
            status: None,
            actions: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn mark(mut self, mark: SourceMark) -> Self {
        self.mark = Some(mark);
        self
    }

    /// The line naming the service the source belongs to.
    pub fn service(mut self, service: impl Into<SharedString>) -> Self {
        self.service = Some(service.into());
        self
    }

    /// The address or path under it; empty says nothing.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into()).filter(|detail: &SharedString| !detail.is_empty());
        self
    }

    /// The live source wears the active badge.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn status(mut self, status: impl IntoElement) -> Self {
        self.status = Some(status.into_any_element());
        self
    }

    /// A button on the right of the header; they sit in the order given.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl ParentElement for SourceCard {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl RenderOnce for SourceCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (hovered, pointer) =
            motion::hovered(motion::id("source-row-hover", &self.id), window, cx);
        let surface = motion::color(
            motion::id("source-row-bg", &self.id),
            style::white(if hovered { HOVER } else { style::RAISED }, cx),
            window,
            cx,
        );
        let hint = |text: SharedString| {
            div()
                .text_xs()
                .text_color(style::white(0.60, cx))
                .child(text)
        };
        v_flex()
            .id(motion::id("source-row", &self.id))
            .w_full()
            .gap_2()
            .p_3()
            .rounded(style::radius(cx))
            .bg(surface)
            .on_hover(motion::track(pointer))
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_start()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .items_start()
                            .children(self.mark.map(|mark| div().pt_0p5().child(mark)))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_sm()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(self.name),
                                            )
                                            .when(self.active, |row| {
                                                row.child(Badge::new(t("active_server")))
                                            }),
                                    )
                                    .children(self.service.map(&hint))
                                    .children(self.detail.map(&hint))
                                    .children(self.status.map(|status| div().mt_1().child(status))),
                            ),
                    )
                    .child(h_flex().gap_2().items_center().children(self.actions)),
            )
            .children(self.body)
    }
}
