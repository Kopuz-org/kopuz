//! The one sort dropdown: a stack of criteria, the first deciding and the rest breaking ties.

use std::rc::Rc;

use config::{SortCriterion, SortDirection};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};

use crate::format::t;
use crate::{focus, icons, motion, style};

const FRAME: u64 = 0;
const HOVER: u64 = 1;
const WASH: u64 = 2;
const INK: u64 = 3;

type Write<F> = Rc<dyn Fn(Vec<SortCriterion<F>>, &mut App)>;

/// Picking the deciding field flips it; any other field becomes the deciding one, ascending.
pub fn promote<F: Copy + PartialEq>(
    mut criteria: Vec<SortCriterion<F>>,
    field: F,
) -> Vec<SortCriterion<F>> {
    match criteria.first_mut() {
        Some(first) if first.field == field => {
            first.direction = match first.direction {
                SortDirection::Asc => SortDirection::Desc,
                SortDirection::Desc => SortDirection::Asc,
            };
        }
        _ => {
            criteria.retain(|criterion| criterion.field != field);
            criteria.insert(0, SortCriterion::new(field, SortDirection::Asc));
        }
    }
    criteria
}

/// "Sort by: Title", dropping every field it can order by with the direction each one runs.
#[derive(IntoElement)]
pub struct SortControl<F: Copy + PartialEq + 'static> {
    id: SharedString,
    fields: Vec<(F, SharedString)>,
    criteria: Vec<SortCriterion<F>>,
    clearable: bool,
    write: Option<Write<F>>,
}

impl<F: Copy + PartialEq + 'static> SortControl<F> {
    pub fn new(id: impl Into<SharedString>, criteria: Vec<SortCriterion<F>>) -> Self {
        Self {
            id: id.into(),
            fields: Vec::new(),
            criteria,
            clearable: true,
            write: None,
        }
    }

    /// The fields worth offering, each with its label.
    pub fn fields(mut self, fields: impl IntoIterator<Item = (F, SharedString)>) -> Self {
        self.fields.extend(fields);
        self
    }

    /// Offer "no sorting", for an order that can fall back to the one it came in; on by default.
    pub fn clearable(mut self, clearable: bool) -> Self {
        self.clearable = clearable;
        self
    }

    /// Where the edited stack goes.
    pub fn on_change(mut self, write: impl Fn(Vec<SortCriterion<F>>, &mut App) + 'static) -> Self {
        self.write = Some(Rc::new(write));
        self
    }
}

impl<F: Copy + PartialEq + 'static> RenderOnce for SortControl<F> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let (hovered, pointer) = motion::hovered(motion::channel(&id, HOVER), window, cx);
        let wash = motion::color(
            motion::channel(&id, WASH),
            style::white(
                match hovered {
                    true => style::HOVER,
                    false => style::RAISED,
                },
                cx,
            ),
            window,
            cx,
        );
        let ink = motion::color(
            motion::channel(&id, INK),
            match hovered {
                true => cx.theme().foreground,
                false => style::white(0.7, cx),
            },
            window,
            cx,
        );
        let deciding = self.criteria.first().and_then(|criterion| {
            self.fields
                .iter()
                .find(|(field, _)| *field == criterion.field)
                .map(|(_, label)| label.clone())
        });
        let label = match deciding {
            Some(field) => SharedString::from(format!("{}: {field}", t("sort_by"))),
            None => t("sort_by"),
        };
        let (fields, criteria, clearable, write) =
            (self.fields, self.criteria, self.clearable, self.write);
        // A Button paints its own hover, so the frame carries the wash and the variant the ink.
        let button = Button::new(id.clone())
            .custom(style::bare_button(ink, cx))
            .icon(icons::icon(icons::SORT))
            .label(label)
            .h_7()
            .px_3()
            .text_xs();
        let button = focus::quiet(button, cx).dropdown_menu(move |mut menu, _, _| {
            for (field, label) in fields.iter().cloned() {
                let position = criteria
                    .iter()
                    .position(|criterion| criterion.field == field);
                let next = promote(criteria.clone(), field);
                let write = write.clone();
                let item = PopupMenuItem::new(label)
                    .checked(position.is_some())
                    .on_click(move |_, _, cx| {
                        if let Some(write) = write.as_ref() {
                            write(next.clone(), cx);
                        }
                    });
                menu = menu.item(match position.map(|index| criteria[index].direction) {
                    Some(SortDirection::Asc) => item.icon(icons::icon(icons::CHEVRON_UP)),
                    Some(SortDirection::Desc) => item.icon(icons::icon(icons::CHEVRON_DOWN)),
                    None => item,
                });
            }
            if !clearable {
                return menu;
            }
            let write = write.clone();
            menu.separator().item(
                PopupMenuItem::new(t("sort_none"))
                    .icon(icons::icon(icons::CLOSE))
                    .on_click(move |_, _, cx| {
                        if let Some(write) = write.as_ref() {
                            write(Vec::new(), cx);
                        }
                    }),
            )
        });
        div()
            .id(motion::channel(&id, FRAME))
            .flex()
            .flex_shrink_0()
            .rounded(style::radius_lg())
            .bg(wash)
            .border_1()
            .border_color(style::white(style::BORDER_SOFT, cx))
            .on_hover(motion::track(pointer))
            .child(button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deciding_field_flips_and_any_other_takes_its_place_ascending() {
        let start = vec![
            SortCriterion::new('a', SortDirection::Asc),
            SortCriterion::new('b', SortDirection::Desc),
        ];
        let flipped = promote(start.clone(), 'a');
        assert_eq!(flipped[0].direction, SortDirection::Desc);
        assert_eq!(flipped.len(), 2);
        let promoted = promote(start, 'b');
        assert_eq!(promoted[0].field, 'b');
        assert_eq!(promoted[0].direction, SortDirection::Asc);
        assert_eq!(promoted[1].field, 'a');
    }
}
