//! The one section heading: a shelf's title, a band over results, a popover's label, a queue group.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, FontWeight, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled,
    Window, div,
};

use crate::{icons, style};

/// How loud a section's title is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Heading {
    /// `text-3xl`: the shelf that leads a page.
    Lead,
    /// `text-2xl`: a shelf among shelves.
    #[default]
    Shelf,
    /// `text-xl text-white/80`: a band over search results.
    Band,
    /// `text-base text-white/90`: one step of a flow.
    Step,
    /// `text-sm font-bold text-slate-400`: a group inside a page, such as a disc or the pinned stations.
    Group,
    /// `text-xs text-white/60`: a popover's own title.
    Label,
    /// `text-[11px] text-white/50`: a group of rows in a dense list, such as the queue.
    Caption,
}

/// A section's heading row: an optional strapline over the title, and what sits at its end.
#[derive(IntoElement)]
pub struct SectionHeading {
    title: SharedString,
    strapline: Option<SharedString>,
    heading: Heading,
    icon: Option<&'static str>,
    count: Option<SharedString>,
    beside: Vec<AnyElement>,
    trailing: Vec<AnyElement>,
}

impl SectionHeading {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            strapline: None,
            heading: Heading::Shelf,
            icon: None,
            count: None,
            beside: Vec::new(),
            trailing: Vec::new(),
        }
    }

    /// The small line over the title, such as what a catalog shelf is grouped by.
    pub fn strapline(mut self, strapline: Option<impl Into<SharedString>>) -> Self {
        self.strapline = strapline.map(Into::into);
        self
    }

    pub fn heading(mut self, heading: Heading) -> Self {
        self.heading = heading;
        self
    }

    /// A glyph before the title.
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// How many the section holds, set right after the title in its own type.
    pub fn count(mut self, count: impl ToString) -> Self {
        self.count = Some(count.to_string().into());
        self
    }

    /// Something right after the title and count, such as a disclosure chevron.
    pub fn beside(mut self, element: impl IntoElement) -> Self {
        self.beside.push(element.into_any_element());
        self
    }

    /// Something at the row's end, such as a "show all" link; several line up in order.
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing.push(element.into_any_element());
        self
    }

    /// Whether the title is drawn small enough to sit in a band of chrome rather than over a shelf.
    pub(crate) fn is_compact(&self) -> bool {
        self.heading != Heading::Lead && self.heading != Heading::Shelf
    }
}

impl RenderOnce for SectionHeading {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let stacked = self.strapline.is_some();
        let heading = self.heading;
        let title = h_flex()
            .min_w_0()
            .items_center()
            .gap_2()
            .map(|title| match heading {
                Heading::Lead => title.text_3xl().font_weight(FontWeight::SEMIBOLD),
                Heading::Shelf => title.text_2xl().font_weight(FontWeight::SEMIBOLD),
                Heading::Band => title
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(style::white(0.8, cx)),
                Heading::Step => title
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(style::white(style::TEXT_STRONG, cx)),
                Heading::Group => title
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .text_color(style::slate_400(cx)),
                Heading::Label => title
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(style::white(style::TEXT_SECONDARY, cx)),
                Heading::Caption => title
                    .text_size(style::text_11())
                    .text_color(style::white(style::TEXT_MUTED, cx)),
            })
            .when_some(self.icon, |title, icon| {
                title.child(icons::icon(icon).flex_shrink_0())
            })
            .child(div().min_w_0().truncate().child(self.title))
            .when_some(self.count, |title, count| {
                title.child(div().flex_shrink_0().child(count))
            })
            .children(self.beside);
        h_flex()
            .w_full()
            .justify_between()
            .gap_4()
            .map(|row| match stacked {
                true => row.items_end(),
                false => row.items_center(),
            })
            .child(
                v_flex()
                    .min_w_0()
                    .when_some(self.strapline, |column, strapline| {
                        column.child(
                            div()
                                .mb_0p5()
                                .text_size(style::text_10())
                                .font_weight(FontWeight::BOLD)
                                .text_color(style::white(0.4, cx))
                                .child(strapline),
                        )
                    })
                    .child(title),
            )
            .when(!self.trailing.is_empty(), |row| {
                row.child(
                    h_flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_4()
                        .children(self.trailing),
                )
            })
    }
}
