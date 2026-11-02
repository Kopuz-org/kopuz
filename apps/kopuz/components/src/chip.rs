//! The one chip, a small pressable label that can be picked or open a menu, and the static badge.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu};
use gpui_kit::component::{Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Context, ElementId, FontWeight, Hsla, IntoElement, ParentElement as _, Pixels,
    RenderOnce, SharedString, Styled as _, Window, div, px,
};

use crate::{focus, icons, skin, style};

/// How large a chip or badge is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ChipSize {
    /// `px-2 py-0.5 rounded`: inside a popover's header or a row.
    #[default]
    Small,
    /// `h-7 px-3 rounded-lg`: a page's filter or picker.
    Medium,
}

/// What a chip's colours say.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ChipTone {
    /// One colour as a wash and as the text: an action that is a stop, a retry, a clear.
    Ink(Hsla),
    /// A quiet raised chip that lifts to white when picked.
    #[default]
    Neutral,
    /// A filter that fills with the accent while it narrows.
    Filter,
}

impl ChipTone {
    /// The fill at rest, the fill under the pointer, and the text.
    fn colours(self, selected: bool, cx: &App) -> (Hsla, Hsla, Hsla) {
        match (self, selected) {
            (ChipTone::Ink(ink), _) => (ink.opacity(style::HOVER), ink.opacity(style::ACTIVE), ink),
            (ChipTone::Neutral, true) => (
                style::white(style::ACTIVE, cx),
                style::white(style::ACTIVE, cx),
                cx.theme().foreground,
            ),
            (ChipTone::Neutral, false) => (
                style::white(style::RAISED, cx),
                style::white(style::HOVER, cx),
                style::white(style::TEXT_SECONDARY, cx),
            ),
            (ChipTone::Filter, true) => (
                cx.theme().primary,
                cx.theme().primary_hover,
                cx.theme().primary_foreground,
            ),
            (ChipTone::Filter, false) => (
                style::white(style::HOVER, cx),
                style::white(style::ACTIVE, cx),
                style::white(style::TEXT_SECONDARY, cx),
            ),
        }
    }
}

/// Material's small corner, which every chip takes.
const M3_CHIP_RADIUS: Pixels = px(8.);
/// Material's outline-variant: the text colour at 18% over the surface.
const M3_OUTLINE_VARIANT: f32 = 0.18;
/// Material's secondary container, as the accent's tonal wash.
const M3_CONTAINER: f32 = 0.24;

/// A Material chip's fill, hover and label: clear and outlined until picked, then the tonal container.
fn material_colours(tone: ChipTone, selected: bool, cx: &App) -> (Hsla, Hsla, Hsla) {
    let accent = cx.theme().primary;
    match (tone, selected) {
        (ChipTone::Ink(ink), _) => (ink.opacity(style::HOVER), ink.opacity(style::ACTIVE), ink),
        (_, true) => (
            accent.opacity(M3_CONTAINER),
            accent.opacity(M3_CONTAINER + style::HOVER),
            cx.theme().foreground,
        ),
        (_, false) => (
            gpui_kit::transparent_black(),
            style::white(style::HOVER, cx),
            style::white(style::TEXT_SECONDARY, cx),
        ),
    }
}

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type Menu = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// A pressable label: picked or not, a tab stop, and optionally the trigger of a menu.
#[derive(IntoElement)]
pub struct Chip {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<&'static str>,
    tone: ChipTone,
    size: ChipSize,
    selected: bool,
    pill: bool,
    focusable: bool,
    stop: bool,
    click: Option<OnClick>,
    menu: Option<Menu>,
}

impl Chip {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            tone: ChipTone::Neutral,
            size: ChipSize::Small,
            selected: false,
            pill: false,
            focusable: true,
            stop: false,
            click: None,
            menu: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A glyph before the label, such as the x of a chip that clears something.
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn tone(mut self, tone: ChipTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn size(mut self, size: ChipSize) -> Self {
        self.size = size;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Fully rounded ends, as the library's filters wear.
    pub fn pill(mut self, pill: bool) -> Self {
        self.pill = pill;
        self
    }

    /// Whether Tab stops on it; on by default.
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// Keep the press from reaching what the chip sits in, such as a row that would play.
    pub fn stop_propagation(mut self, stop: bool) -> Self {
        self.stop = stop;
        self
    }

    pub fn on_click(
        mut self,
        click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click = Some(Rc::new(click));
        self
    }

    /// A menu the chip opens, for a chip that picks a value rather than toggling.
    pub fn dropdown(
        mut self,
        menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.menu = Some(Rc::new(menu));
        self
    }
}

impl RenderOnce for Chip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let material = skin::material();
        let (fill, hover, ink) = match material {
            true => material_colours(self.tone, self.selected, cx),
            false => self.tone.colours(self.selected, cx),
        };
        let radius = match (self.pill, self.size) {
            (true, _) => cx.theme().radius_full(),
            // Material's chips take the small corner whatever their size.
            (false, _) if material => M3_CHIP_RADIUS,
            (false, ChipSize::Small) => style::to_px(style::radius(cx), window),
            (false, ChipSize::Medium) => style::to_px(style::radius_lg(), window),
        };
        // An unpicked Material chip is outlined; a picked one is filled and leads with a check.
        let outlined = material && !self.selected && !matches!(self.tone, ChipTone::Ink(_));
        let checked = material && self.selected && self.icon.is_none();
        let stop = self.stop;
        let button = Button::new(self.id)
            .custom(style::button(fill, hover, ink, cx))
            .xsmall()
            .selected(self.selected)
            .tab_stop(self.focusable)
            .rounded(radius)
            .text_xs()
            .map(|chip| match (self.size, material) {
                (ChipSize::Small, _) => chip.h_5().px_2(),
                (ChipSize::Medium, false) => chip.h_7().px_3(),
                (ChipSize::Medium, true) => chip.h_8().px_4(),
            })
            .when(outlined, |chip| {
                chip.border_1()
                    .border_color(style::white(M3_OUTLINE_VARIANT, cx))
            })
            .when(checked, |chip| chip.icon(icons::icon(icons::CHECK)))
            .when_some(self.icon, |chip, icon| chip.icon(icons::icon(icon)))
            .when_some(self.label, |chip, label| chip.label(label))
            .when_some(self.click, |chip, click| {
                chip.on_click(move |event, window, cx| {
                    if stop {
                        cx.stop_propagation();
                    }
                    click(event, window, cx);
                })
            });
        let button = focus::quiet(button, cx);
        match self.menu {
            Some(menu) => button
                .dropdown_menu(move |popup, window, cx| menu(popup, window, cx))
                .into_any_element(),
            None => button.into_any_element(),
        }
    }
}

/// What a badge's colours say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BadgeTone {
    /// The accent as a wash: what a source or a mode is set to.
    #[default]
    Accent,
    /// A raised grey tag, such as a station's genre.
    Neutral,
    /// A chip that is already doing what it would start, so it cannot be pressed.
    Muted,
}

/// A label in a chip's shape that nothing presses.
#[derive(IntoElement)]
pub struct Badge {
    label: SharedString,
    tone: BadgeTone,
    size: ChipSize,
}

impl Badge {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            tone: BadgeTone::Accent,
            size: ChipSize::Small,
        }
    }

    pub fn tone(mut self, tone: BadgeTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn size(mut self, size: ChipSize) -> Self {
        self.size = size;
        self
    }
}

/// `bg-indigo-500/16`, the accent wash a badge sits on.
const ACCENT_WASH: f32 = 0.16;

impl RenderOnce for Badge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (fill, ink) = match self.tone {
            BadgeTone::Accent => (style::accent(ACCENT_WASH, cx), cx.theme().primary_hover),
            BadgeTone::Neutral => (style::white(style::RAISED, cx), style::slate_500(cx)),
            BadgeTone::Muted => (
                style::white(style::RAISED, cx),
                style::white(style::TEXT_MUTED, cx),
            ),
        };
        div()
            .flex_shrink_0()
            .text_xs()
            .bg(fill)
            .text_color(ink)
            .when(self.tone == BadgeTone::Neutral, |badge| {
                badge.font_weight(FontWeight::MEDIUM)
            })
            .map(|badge| match self.size {
                ChipSize::Small => badge.px_2().py_0p5().rounded(style::radius(cx)),
                ChipSize::Medium => badge.px_3().py_1().rounded(style::radius_lg()),
            })
            .child(self.label)
    }
}
