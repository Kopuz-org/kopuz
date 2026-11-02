//! The one search box, and the one field-shaped button that opens quick search.

use std::rc::Rc;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement, Keystroke, ParentElement as _,
    Pixels, RenderOnce, SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled,
    Window, div, px,
};

use crate::{focus, icons, skin, style};

type OnOpen = Rc<dyn Fn(&mut Window, &mut App)>;

/// The widest a trigger in the chrome grows.
const FIELD_MAX: Pixels = px(440.);
/// Material's surface-container-high: the text colour at 10% over the surface.
const M3_SEARCH_FILL: f32 = 0.10;

/// A text box that filters, with the search glyph and whatever narrows it before the caret.
#[derive(IntoElement)]
pub struct SearchField {
    state: Entity<InputState>,
    cleanable: bool,
    bare: bool,
    chips: Vec<AnyElement>,
    style: StyleRefinement,
}

impl SearchField {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            cleanable: false,
            bare: false,
            chips: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    /// An x at the end that empties the box.
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.cleanable = cleanable;
        self
    }

    /// No frame of its own, for a box that is already the header of a panel.
    pub fn bare(mut self, bare: bool) -> Self {
        self.bare = bare;
        self
    }

    /// Something after the glyph that says what the box is narrowed to.
    pub fn chip(mut self, chip: Option<impl IntoElement>) -> Self {
        self.chips.extend(chip.map(IntoElement::into_any_element));
        self
    }
}

impl Styled for SearchField {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for SearchField {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        Input::new(&self.state)
            .cleanable(self.cleanable)
            .appearance(!self.bare)
            .prefix(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(icons::icon(icons::SEARCH).small())
                    .children(self.chips),
            )
            .refine_style(&self.style)
    }
}

/// How large a trigger is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TriggerSize {
    /// The chrome's field: as wide as it is given, up to a limit.
    #[default]
    Field,
    /// A chip beside a page's title.
    Compact,
}

/// Looks like a search box and opens quick search, which is where typing actually goes.
#[derive(IntoElement)]
pub struct SearchTrigger {
    id: SharedString,
    label: SharedString,
    shortcut: Option<Keystroke>,
    size: TriggerSize,
    on_open: Option<OnOpen>,
}

impl SearchTrigger {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            shortcut: None,
            size: TriggerSize::Field,
            on_open: None,
        }
    }

    /// What opening does; the owner decides where typing goes.
    pub fn on_open(mut self, open: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open = Some(Rc::new(open));
        self
    }

    /// The keys that open quick search from anywhere, drawn as a keycap at the end.
    pub fn shortcut(mut self, shortcut: Option<Keystroke>) -> Self {
        self.shortcut = shortcut;
        self
    }

    pub fn size(mut self, size: TriggerSize) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for SearchTrigger {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus::handle(format!("search-trigger:{}", self.id), window, cx);
        let focused = focus.is_focused(window);
        let field = self.size == TriggerSize::Field;
        let (ink, edge) = match field {
            true => (style::TEXT_MUTED, style::BORDER),
            false => (style::TEXT_SECONDARY, style::BORDER_SOFT),
        };
        let open = self.on_open;
        let activate = open.clone();
        focus::stop(h_flex().id(self.id), &focus, move |window, cx| {
            if let Some(open) = activate.as_ref() {
                open(window, cx);
            }
        })
        .relative()
        .gap_2()
        .items_center()
        // Material's search bar: a full pill on surface-container-high, with no outline.
        .map(|trigger| match skin::material() {
            true => trigger.rounded_full().bg(style::white(M3_SEARCH_FILL, cx)),
            false => trigger
                .rounded(style::radius_lg())
                .border_1()
                .border_color(style::white(edge, cx))
                .bg(style::white(style::RAISED, cx)),
        })
        .cursor_pointer()
        .hover(|trigger| trigger.bg(style::white(style::HOVER, cx)))
        .text_color(style::white(ink, cx))
        .map(|trigger| match field {
            true => trigger
                .w_full()
                .max_w(FIELD_MAX)
                .h(style::chrome().search_height)
                .px_2()
                .text_sm(),
            false => trigger.flex_shrink_0().h_8().px_3().text_xs(),
        })
        .on_click(move |_, window, cx| {
            if let Some(open) = open.as_ref() {
                open(window, cx);
            }
        })
        .child(
            icons::icon(icons::SEARCH)
                .with_size(match field {
                    true => px(14.),
                    false => style::text_11(),
                })
                .flex_shrink_0(),
        )
        .child(
            div()
                .when(field, |label| label.flex_1())
                .min_w_0()
                .truncate()
                .child(self.label),
        )
        .children(
            self.shortcut
                .map(|keys| crate::keycap::Keycap::stroke(&keys)),
        )
        .when(focused, |trigger| {
            trigger.child(focus::ring(style::radius_lg(), cx))
        })
    }
}
