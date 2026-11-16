//! Material Design 3 Button components.
//!
//! MD3 defines six button types, each with a distinct level of visual emphasis:
//!
//! - **Elevated** — a filled-tonal button with a shadow for extra emphasis
//! - **Filled** — high emphasis, solid primary background
//! - **Filled Tonal** — medium emphasis, secondary-container background
//! - **Outlined** — medium emphasis, transparent with an outline border
//! - **Text** — low emphasis, no background or border
//! - **Icon** — a round button containing only an icon
//!
//! All button types follow a builder pattern and implement `IntoElement`.
//! Click handlers use GPUI's `on_mouse_down` signature, fully compatible
//! with `cx.listener(...)`.
//!
//! # Examples
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::button::*;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Filled primary action
//! let accept = FilledButton::new("Accept", theme)
//!     .on_click(cx.listener(|this, _, _, cx| { /* ... */ }));
//!
//! // Outlined secondary action
//! let cancel = OutlinedButton::new("Cancel", theme)
//!     .on_click(cx.listener(|this, _, _, cx| { /* ... */ }));
//!
//! // Icon button
//! let fav = IconButton::new("♥", theme)
//!     .filled()
//!     .on_click(cx.listener(|this, _, _, cx| { /* ... */ }));
//! ```

use gpui::{div, prelude::*, px, rgb, MouseButton, MouseDownEvent, Stateful, Window};

use super::theme::MaterialTheme;

// ═══════════════════════════════════════════════════════════════════════════════
//  Common helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Standard MD3 button height (40 dp).
const BUTTON_HEIGHT: f32 = 40.0;
/// Standard horizontal padding for buttons with text.
const BUTTON_PADDING_H: f32 = 24.0;
/// Horizontal padding for buttons that also have a leading icon.
const BUTTON_PADDING_H_WITH_ICON: f32 = 16.0;
/// Gap between leading icon and label text.
const ICON_LABEL_GAP: f32 = 8.0;
/// Standard MD3 button corner radius (full pill).
const BUTTON_RADIUS: f32 = 20.0;

/// Icon button diameter.
const ICON_BUTTON_SIZE: f32 = 40.0;
/// Icon button corner radius.
const _ICON_BUTTON_RADIUS: f32 = 20.0;

// A type alias for the click handler to reduce repetition.
type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ═══════════════════════════════════════════════════════════════════════════════
//  ElevatedButton
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Elevated Button**.
///
/// Elevated buttons are filled-tonal buttons with a shadow, used when a
/// button needs slightly more emphasis than a tonal button but less than
/// a filled button. They sit on top of the surface with a small elevation
/// shadow.
///
/// MD3 spec: surface-container-low background, primary text, level-1 shadow.
pub struct ElevatedButton {
    label: String,
    icon: Option<String>,
    theme: MaterialTheme,
    on_click: Option<ClickHandler>,
    disabled: bool,
    id: Option<gpui::ElementId>,
}

impl ElevatedButton {
    /// Create a new elevated button with the given label.
    pub fn new(label: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            label: label.into(),
            icon: None,
            theme,
            on_click: None,
            disabled: false,
            id: None,
        }
    }

    /// Set a leading icon (emoji or short string).
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Mark the button as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set an explicit element ID (required if multiple elevated buttons
    /// exist in the same parent and need distinct identities for GPUI).
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for ElevatedButton {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let has_icon = self.icon.is_some();

        let bg = if self.disabled {
            t.on_surface // will be at 12% opacity via the container
        } else {
            t.surface_container_low
        };
        let fg = if self.disabled {
            t.on_surface
        } else {
            t.primary
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("elevated-btn".into()));

        let mut btn = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(ICON_LABEL_GAP))
            .h(px(BUTTON_HEIGHT))
            .rounded(px(BUTTON_RADIUS))
            .text_sm()
            .cursor_pointer();

        // Padding
        if has_icon {
            btn = btn
                .pl(px(BUTTON_PADDING_H_WITH_ICON))
                .pr(px(BUTTON_PADDING_H));
        } else {
            btn = btn.px(px(BUTTON_PADDING_H));
        }

        if self.disabled {
            btn = btn
                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.12))
                .text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
        } else {
            btn = btn
                .bg(rgb(bg))
                .text_color(rgb(fg))
                // Simulate a subtle shadow with a border on the bottom
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.08));
        }

        // Leading icon
        if let Some(icon) = self.icon {
            btn = btn.child(div().text_lg().child(icon));
        }

        // Label
        btn = btn.child(self.label);

        // Click handler
        if let Some(handler) = self.on_click {
            if !self.disabled {
                btn = btn.on_mouse_down(MouseButton::Left, handler);
            }
        }

        btn.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  FilledButton
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Filled Button**.
///
/// The highest-emphasis button. Uses the primary color as background
/// with on-primary text. Suitable for the single most important action
/// on a screen.
pub struct FilledButton {
    label: String,
    icon: Option<String>,
    theme: MaterialTheme,
    on_click: Option<ClickHandler>,
    disabled: bool,
    id: Option<gpui::ElementId>,
}

impl FilledButton {
    /// Create a new filled button with the given label.
    pub fn new(label: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            label: label.into(),
            icon: None,
            theme,
            on_click: None,
            disabled: false,
            id: None,
        }
    }

    /// Set a leading icon.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Mark the button as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for FilledButton {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let has_icon = self.icon.is_some();

        let bg = if self.disabled {
            t.on_surface
        } else {
            t.primary
        };
        let fg = if self.disabled {
            t.on_surface
        } else {
            t.on_primary
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("filled-btn".into()));

        let mut btn = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(ICON_LABEL_GAP))
            .h(px(BUTTON_HEIGHT))
            .rounded(px(BUTTON_RADIUS))
            .text_sm()
            .cursor_pointer();

        if has_icon {
            btn = btn
                .pl(px(BUTTON_PADDING_H_WITH_ICON))
                .pr(px(BUTTON_PADDING_H));
        } else {
            btn = btn.px(px(BUTTON_PADDING_H));
        }

        if self.disabled {
            btn = btn
                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.12))
                .text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
        } else {
            btn = btn.bg(rgb(bg)).text_color(rgb(fg));
        }

        if let Some(icon) = self.icon {
            btn = btn.child(div().text_lg().child(icon));
        }

        btn = btn.child(self.label);

        if let Some(handler) = self.on_click {
            if !self.disabled {
                btn = btn.on_mouse_down(MouseButton::Left, handler);
            }
        }

        btn.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  FilledTonalButton
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Filled Tonal Button**.
///
/// A medium-emphasis button that uses the secondary-container color as
/// its background. Sits between filled and outlined in visual weight.
/// Ideal for actions that are important but not the primary action.
pub struct FilledTonalButton {
    label: String,
    icon: Option<String>,
    theme: MaterialTheme,
    on_click: Option<ClickHandler>,
    disabled: bool,
    id: Option<gpui::ElementId>,
}

impl FilledTonalButton {
    /// Create a new filled tonal button with the given label.
    pub fn new(label: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            label: label.into(),
            icon: None,
            theme,
            on_click: None,
            disabled: false,
            id: None,
        }
    }

    /// Set a leading icon.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Mark the button as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for FilledTonalButton {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let has_icon = self.icon.is_some();

        let bg = if self.disabled {
            t.on_surface
        } else {
            t.secondary_container
        };
        let fg = if self.disabled {
            t.on_surface
        } else {
            t.on_secondary_container
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("tonal-btn".into()));

        let mut btn = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(ICON_LABEL_GAP))
            .h(px(BUTTON_HEIGHT))
            .rounded(px(BUTTON_RADIUS))
            .text_sm()
            .cursor_pointer();

        if has_icon {
            btn = btn
                .pl(px(BUTTON_PADDING_H_WITH_ICON))
                .pr(px(BUTTON_PADDING_H));
        } else {
            btn = btn.px(px(BUTTON_PADDING_H));
        }

        if self.disabled {
            btn = btn
                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.12))
                .text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
        } else {
            btn = btn.bg(rgb(bg)).text_color(rgb(fg));
        }

        if let Some(icon) = self.icon {
            btn = btn.child(div().text_lg().child(icon));
        }

        btn = btn.child(self.label);

        if let Some(handler) = self.on_click {
            if !self.disabled {
                btn = btn.on_mouse_down(MouseButton::Left, handler);
            }
        }

        btn.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  OutlinedButton
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Outlined Button**.
///
/// A medium-emphasis button with a transparent background and a thin
/// outline border. Uses primary-colored text. Suitable for secondary
/// actions, especially when paired with a filled button.
pub struct OutlinedButton {
    label: String,
    icon: Option<String>,
    theme: MaterialTheme,
    on_click: Option<ClickHandler>,
    disabled: bool,
    id: Option<gpui::ElementId>,
}

impl OutlinedButton {
    /// Create a new outlined button with the given label.
    pub fn new(label: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            label: label.into(),
            icon: None,
            theme,
            on_click: None,
            disabled: false,
            id: None,
        }
    }

    /// Set a leading icon.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Mark the button as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for OutlinedButton {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let has_icon = self.icon.is_some();

        let fg = if self.disabled {
            t.on_surface
        } else {
            t.primary
        };
        let border = if self.disabled {
            t.on_surface
        } else {
            t.outline
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("outlined-btn".into()));

        let mut btn = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(ICON_LABEL_GAP))
            .h(px(BUTTON_HEIGHT))
            .rounded(px(BUTTON_RADIUS))
            .text_sm()
            .cursor_pointer();

        if has_icon {
            btn = btn
                .pl(px(BUTTON_PADDING_H_WITH_ICON))
                .pr(px(BUTTON_PADDING_H));
        } else {
            btn = btn.px(px(BUTTON_PADDING_H));
        }

        if self.disabled {
            btn = btn
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.12))
                .text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
        } else {
            btn = btn.border_1().border_color(rgb(border)).text_color(rgb(fg));
        }

        if let Some(icon) = self.icon {
            btn = btn.child(div().text_lg().child(icon));
        }

        btn = btn.child(self.label);

        if let Some(handler) = self.on_click {
            if !self.disabled {
                btn = btn.on_mouse_down(MouseButton::Left, handler);
            }
        }

        btn.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  TextButton
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Text Button**.
///
/// The lowest-emphasis button type. No background or border — just
/// primary-colored text. Use for tertiary actions, inline links, or
/// dialog dismiss buttons.
pub struct TextButton {
    label: String,
    icon: Option<String>,
    theme: MaterialTheme,
    on_click: Option<ClickHandler>,
    disabled: bool,
    id: Option<gpui::ElementId>,
}

impl TextButton {
    /// Create a new text button with the given label.
    pub fn new(label: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            label: label.into(),
            icon: None,
            theme,
            on_click: None,
            disabled: false,
            id: None,
        }
    }

    /// Set a leading icon.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Mark the button as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for TextButton {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let has_icon = self.icon.is_some();

        let fg = if self.disabled {
            t.on_surface
        } else {
            t.primary
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("text-btn".into()));

        let mut btn = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(ICON_LABEL_GAP))
            .h(px(BUTTON_HEIGHT))
            .rounded(px(BUTTON_RADIUS))
            .text_sm()
            .cursor_pointer();

        // Text buttons have 12dp horizontal padding (less than other buttons)
        if has_icon {
            btn = btn.pl(px(12.0)).pr(px(16.0));
        } else {
            btn = btn.px(px(12.0));
        }

        if self.disabled {
            btn = btn.text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
        } else {
            btn = btn.text_color(rgb(fg));
        }

        if let Some(icon) = self.icon {
            btn = btn.child(div().text_lg().child(icon));
        }

        btn = btn.child(self.label);

        if let Some(handler) = self.on_click {
            if !self.disabled {
                btn = btn.on_mouse_down(MouseButton::Left, handler);
            }
        }

        btn.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  IconButton
// ═══════════════════════════════════════════════════════════════════════════════

/// The visual style of an icon button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconButtonStyle {
    /// Standard — no background, just the icon.
    #[default]
    Standard,
    /// Filled — solid primary background.
    Filled,
    /// Filled tonal — secondary-container background.
    FilledTonal,
    /// Outlined — transparent with an outline border.
    Outlined,
}

/// A Material Design 3 **Icon Button**.
///
/// A compact, icon-only button typically used in app bars, cards, and
/// other dense UIs. Supports four visual styles via [`IconButtonStyle`].
///
/// The default size is 40×40 dp (standard MD3 icon button target).
pub struct IconButton {
    icon: String,
    theme: MaterialTheme,
    style: IconButtonStyle,
    on_click: Option<ClickHandler>,
    disabled: bool,
    selected: bool,
    size: f32,
    id: Option<gpui::ElementId>,
    /// Optional tooltip text (rendered as a title attribute for accessibility).
    tooltip: Option<String>,
}

impl IconButton {
    /// Create a new standard icon button.
    pub fn new(icon: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            icon: icon.into(),
            theme,
            style: IconButtonStyle::Standard,
            on_click: None,
            disabled: false,
            selected: false,
            size: ICON_BUTTON_SIZE,
            id: None,
            tooltip: None,
        }
    }

    /// Use the **filled** style (primary background).
    pub fn filled(mut self) -> Self {
        self.style = IconButtonStyle::Filled;
        self
    }

    /// Use the **filled tonal** style (secondary-container background).
    pub fn filled_tonal(mut self) -> Self {
        self.style = IconButtonStyle::FilledTonal;
        self
    }

    /// Use the **outlined** style (outline border, no fill).
    pub fn outlined(mut self) -> Self {
        self.style = IconButtonStyle::Outlined;
        self
    }

    /// Set the visual style explicitly.
    pub fn style(mut self, style: IconButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Set the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Mark as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Mark as selected / toggled (changes colours for filled and tonal).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Override the default size (40 dp).
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set a tooltip / accessibility label.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
}

impl IntoElement for IconButton {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let radius = self.size / 2.0;

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("icon-btn".into()));

        let mut btn = div()
            .id(elem_id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(self.size))
            .rounded(px(radius))
            .cursor_pointer()
            .text_xl();

        match self.style {
            IconButtonStyle::Standard => {
                let fg = if self.disabled {
                    t.on_surface
                } else if self.selected {
                    t.primary
                } else {
                    t.on_surface_variant
                };
                if self.disabled {
                    btn = btn.text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
                } else {
                    btn = btn.text_color(rgb(fg));
                }
            }
            IconButtonStyle::Filled => {
                let (bg, fg) = if self.disabled {
                    (t.on_surface, t.on_surface)
                } else if self.selected {
                    (t.primary, t.on_primary)
                } else {
                    (t.surface_container_highest, t.primary)
                };
                if self.disabled {
                    btn = btn
                        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.12))
                        .text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
                } else {
                    btn = btn.bg(rgb(bg)).text_color(rgb(fg));
                }
            }
            IconButtonStyle::FilledTonal => {
                let (bg, fg) = if self.disabled {
                    (t.on_surface, t.on_surface)
                } else if self.selected {
                    (t.secondary_container, t.on_secondary_container)
                } else {
                    (t.surface_container_highest, t.on_surface_variant)
                };
                if self.disabled {
                    btn = btn
                        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.12))
                        .text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
                } else {
                    btn = btn.bg(rgb(bg)).text_color(rgb(fg));
                }
            }
            IconButtonStyle::Outlined => {
                let fg = if self.disabled {
                    t.on_surface
                } else if self.selected {
                    t.inverse_on_surface
                } else {
                    t.on_surface_variant
                };
                if self.disabled {
                    btn = btn
                        .border_1()
                        .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.12))
                        .text_color(gpui::hsla(0.0, 0.0, 0.0, 0.38));
                } else if self.selected {
                    btn = btn.bg(rgb(t.inverse_surface)).text_color(rgb(fg));
                } else {
                    btn = btn
                        .border_1()
                        .border_color(rgb(t.outline))
                        .text_color(rgb(fg));
                }
            }
        }

        btn = btn.child(self.icon);

        if let Some(handler) = self.on_click {
            if !self.disabled {
                btn = btn.on_mouse_down(MouseButton::Left, handler);
            }
        }

        btn.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Legacy compatibility — free functions
// ═══════════════════════════════════════════════════════════════════════════════

/// Legacy helper: renders a filled button (static, no click handler).
///
/// Prefer [`FilledButton`] for interactive use.
pub fn button_filled(label: &str, bg: u32, fg: u32) -> impl IntoElement {
    div()
        .px_6()
        .py(px(10.0))
        .rounded(px(BUTTON_RADIUS))
        .bg(rgb(bg))
        .text_sm()
        .text_color(rgb(fg))
        .child(label.to_string())
}

/// Legacy helper: renders a tonal button (static, no click handler).
///
/// Prefer [`FilledTonalButton`] for interactive use.
pub fn button_tonal(label: &str, bg: u32, fg: u32) -> impl IntoElement {
    div()
        .px_6()
        .py(px(10.0))
        .rounded(px(BUTTON_RADIUS))
        .bg(rgb(bg))
        .text_sm()
        .text_color(rgb(fg))
        .child(label.to_string())
}

/// Legacy helper: renders an outlined button (static, no click handler).
///
/// Prefer [`OutlinedButton`] for interactive use.
pub fn button_outlined(label: &str, outline: u32) -> impl IntoElement {
    div()
        .px_6()
        .py(px(10.0))
        .rounded(px(BUTTON_RADIUS))
        .border_1()
        .border_color(rgb(outline))
        .text_sm()
        .text_color(rgb(outline))
        .child(label.to_string())
}

/// Legacy helper: renders a text button (static, no click handler).
///
/// Prefer [`TextButton`] for interactive use.
pub fn button_text(label: &str, color: u32) -> impl IntoElement {
    div()
        .px_3()
        .py(px(10.0))
        .text_sm()
        .text_color(rgb(color))
        .child(label.to_string())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Legacy composite showcase — all button styles in rows.
///
/// This is kept for backward compatibility with the existing component
/// gallery. New code should use the struct-based button builders directly.
pub fn buttons(dark: bool) -> impl IntoElement {
    let md_primary = if dark { 0xd0bcff_u32 } else { 0x6750a4 };
    let md_on_primary = if dark { 0x381e72_u32 } else { 0xffffff };
    let md_secondary = if dark { 0x332d41_u32 } else { 0xe8def8 };
    let md_on_secondary = if dark { 0xe8def8_u32 } else { 0x1d192b };
    let md_outline = if dark { 0x938f99_u32 } else { 0x79747e };
    let md_on_surface = if dark { 0xe6e1e5_u32 } else { 0x1c1b1f };
    let md_surface_variant = if dark { 0x49454f_u32 } else { 0xe7e0ec };

    div()
        .flex()
        .flex_col()
        .gap_3()
        // Row 1: Filled buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_filled("Filled", md_primary, md_on_primary))
                .child(button_filled("Accept", 0xa6e3a1, 0x1e1e2e))
                .child(button_filled("Delete", 0xf38ba8, 0x1e1e2e)),
        )
        // Row 2: Tonal buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_tonal("Tonal", md_secondary, md_on_secondary))
                .child(button_tonal("Secondary", md_surface_variant, md_on_surface)),
        )
        // Row 3: Outlined buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_outlined("Outlined", md_outline))
                .child(button_outlined("Cancel", md_outline)),
        )
        // Row 4: Text buttons
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(button_text("Text Button", md_primary))
                .child(button_text("Learn More", md_primary)),
        )
}

/// Renders a comprehensive showcase of all MD3 button types using the
/// new struct-based builders.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::button;
///
/// let demo = button::button_demo(true); // dark mode
/// ```
pub fn button_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        // Elevated buttons
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("ELEVATED"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(ElevatedButton::new("Elevated", theme).id("demo-elev-1"))
                        .child(
                            ElevatedButton::new("With Icon", theme)
                                .icon("📎")
                                .id("demo-elev-2"),
                        )
                        .child(
                            ElevatedButton::new("Disabled", theme)
                                .disabled(true)
                                .id("demo-elev-3"),
                        ),
                ),
        )
        // Filled buttons
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("FILLED"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(FilledButton::new("Filled", theme).id("demo-filled-1"))
                        .child(
                            FilledButton::new("With Icon", theme)
                                .icon("✓")
                                .id("demo-filled-2"),
                        )
                        .child(
                            FilledButton::new("Disabled", theme)
                                .disabled(true)
                                .id("demo-filled-3"),
                        ),
                ),
        )
        // Filled tonal buttons
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("FILLED TONAL"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(FilledTonalButton::new("Tonal", theme).id("demo-tonal-1"))
                        .child(
                            FilledTonalButton::new("With Icon", theme)
                                .icon("🔔")
                                .id("demo-tonal-2"),
                        )
                        .child(
                            FilledTonalButton::new("Disabled", theme)
                                .disabled(true)
                                .id("demo-tonal-3"),
                        ),
                ),
        )
        // Outlined buttons
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("OUTLINED"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(OutlinedButton::new("Outlined", theme).id("demo-out-1"))
                        .child(
                            OutlinedButton::new("With Icon", theme)
                                .icon("➕")
                                .id("demo-out-2"),
                        )
                        .child(
                            OutlinedButton::new("Disabled", theme)
                                .disabled(true)
                                .id("demo-out-3"),
                        ),
                ),
        )
        // Text buttons
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("TEXT"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(TextButton::new("Text Button", theme).id("demo-text-1"))
                        .child(
                            TextButton::new("With Icon", theme)
                                .icon("ℹ️")
                                .id("demo-text-2"),
                        )
                        .child(
                            TextButton::new("Disabled", theme)
                                .disabled(true)
                                .id("demo-text-3"),
                        ),
                ),
        )
        // Icon buttons — all four styles
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("ICON BUTTONS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(IconButton::new("⚙️", theme).id("demo-icon-std"))
                        .child(IconButton::new("♥", theme).filled().id("demo-icon-fill"))
                        .child(
                            IconButton::new("♥", theme)
                                .filled()
                                .selected(true)
                                .id("demo-icon-fill-sel"),
                        )
                        .child(
                            IconButton::new("🔔", theme)
                                .filled_tonal()
                                .id("demo-icon-tonal"),
                        )
                        .child(
                            IconButton::new("📌", theme)
                                .outlined()
                                .id("demo-icon-outlined"),
                        )
                        .child(
                            IconButton::new("🗑️", theme)
                                .disabled(true)
                                .id("demo-icon-disabled"),
                        ),
                ),
        )
}
