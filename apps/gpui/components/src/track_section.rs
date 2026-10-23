use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderOnce, SharedString, Styled as _, Window, div, px,
};

use crate::heading::SectionHeading;
use crate::style;
use crate::track_list::TrackList;

/// A heading over a track list sized to its rows, for a page that stacks lists in one scroll.
#[derive(IntoElement)]
pub struct TrackSection {
    id: SharedString,
    list: Entity<TrackList>,
    heading: Option<SectionHeading>,
    trailing: Vec<AnyElement>,
    gap: Pixels,
    wash: bool,
}

impl TrackSection {
    pub fn new(id: impl Into<SharedString>, list: Entity<TrackList>) -> Self {
        Self {
            id: id.into(),
            list,
            heading: None,
            trailing: Vec::new(),
            gap: px(24.),
            wash: false,
        }
    }

    pub fn heading(mut self, heading: SectionHeading) -> Self {
        self.heading = Some(heading);
        self
    }

    /// Something at the heading's end, such as a "show all" link.
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing.push(element.into_any_element());
        self
    }

    /// The space between the heading and its rows; `gap-6` unless a page says otherwise.
    pub fn gap(mut self, gap: Pixels) -> Self {
        self.gap = gap;
        self
    }

    /// Wash the heading under the pointer, the way a disc's does.
    pub fn wash(mut self, wash: bool) -> Self {
        self.wash = wash;
        self
    }
}

impl RenderOnce for TrackSection {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let rows = self.list.read(cx).rows().len();
        let hover = style::white(style::RAISED, cx);
        let heading = self.heading.map(|heading| {
            let heading = self
                .trailing
                .into_iter()
                .fold(heading, SectionHeading::trailing);
            match self.wash {
                true => div()
                    .id(SharedString::from(format!("{}-heading", self.id)))
                    .w_full()
                    .p_2()
                    .rounded(style::radius_lg())
                    .hover(move |heading| heading.bg(hover))
                    .child(heading)
                    .into_any_element(),
                false => heading.into_any_element(),
            }
        });
        v_flex().w_full().gap(self.gap).children(heading).child(
            div()
                .w_full()
                .h(style::media_row() * rows as f32)
                .child(self.list),
        )
    }
}
