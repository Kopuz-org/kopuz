#![allow(unused_imports)]
//! Material Design 3 List & Miscellaneous components.
//!
//! This module provides several commonly-used MD3 components:
//!
//! - **ListTile** — a list item row with leading, title, subtitle, trailing
//! - **Divider** — a thin horizontal separator line
//! - **Badge** — a small status indicator (dot or text pill)
//! - **Tooltip** — a text label that appears on hover/press
//! - **SegmentedButton** — a group of togglable segments
//! - **Chip** — compact elements: Assist, Filter, Input, Suggestion
//! - **BottomAppBar** — a bottom bar with icons and an optional FAB
//! - **ExpansionTile** — a collapsible list tile with children
//!
//! All components follow a builder pattern and implement `IntoElement`.

use gpui::{
    div, prelude::*, px, rgb, AnyElement, Hsla, MouseButton, MouseDownEvent, Stateful, Window,
};

use super::theme::{color, MaterialTheme, ShapeScale, TRANSPARENT};

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ═══════════════════════════════════════════════════════════════════════════════
//  ListTile
// ═══════════════════════════════════════════════════════════════════════════════

/// The density of a list tile — controls vertical padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListTileDensity {
    /// Standard density — 56dp minimum height for one-line, 72dp for two/three.
    #[default]
    Standard,
    /// Compact density — reduced vertical padding.
    Compact,
}

/// A Material Design 3 **ListTile**.
///
/// A single fixed-height row that can contain a leading element (icon or
/// avatar), title text, optional subtitle, and trailing element (icon,
/// switch, text, etc.).
///
/// # Layout
///
/// ```text
/// ┌────────────────────────────────────────────────────────────────┐
/// │  [Leading]  Title                               [Trailing]    │
/// │             Subtitle                                          │
/// └────────────────────────────────────────────────────────────────┘
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile::ListTile;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// // Simple one-line tile
/// let tile = ListTile::new(theme)
///     .leading_icon("📧")
///     .title("Email")
///     .trailing_text("24")
///     .on_click(cx.listener(|this, _, _, cx| { /* ... */ }));
///
/// // Two-line tile with avatar
/// let tile = ListTile::new(theme)
///     .leading_avatar("JD", 0x6750A4)
///     .title("Jane Doe")
///     .subtitle("Online now")
///     .trailing_icon("💬");
///
/// // Three-line tile
/// let tile = ListTile::new(theme)
///     .leading_icon("📄")
///     .title("Document.pdf")
///     .subtitle("Last modified 2 hours ago")
///     .supporting_text("Shared with 3 people")
///     .trailing_icon("⋮");
/// ```
pub struct ListTile {
    theme: MaterialTheme,
    // Leading
    leading: Option<AnyElement>,
    // Content
    title: Option<String>,
    subtitle: Option<String>,
    supporting: Option<String>,
    // Trailing
    trailing: Option<AnyElement>,
    // Behaviour
    on_click: Option<ClickHandler>,
    selected: bool,
    disabled: bool,
    density: ListTileDensity,
    id: Option<gpui::ElementId>,
    /// Whether to show a divider below this tile.
    show_divider: bool,
}

impl ListTile {
    /// Create a new list tile.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            leading: None,
            title: None,
            subtitle: None,
            supporting: None,
            trailing: None,
            on_click: None,
            selected: false,
            disabled: false,
            density: ListTileDensity::Standard,
            id: None,
            show_divider: false,
        }
    }

    // ── Leading element ──────────────────────────────────────────────

    /// Set a leading icon (emoji or short text string).
    pub fn leading_icon(mut self, icon: impl Into<String>) -> Self {
        let icon_str = icon.into();
        let t = self.theme;
        self.leading = Some(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.0))
                .text_xl()
                .text_color(rgb(t.on_surface_variant))
                .child(icon_str)
                .into_any_element(),
        );
        self
    }

    /// Set a leading text avatar (initials on a coloured circle).
    pub fn leading_avatar(mut self, text: impl Into<String>, bg: u32) -> Self {
        let text_str = text.into();
        let t = self.theme;
        self.leading = Some(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(40.0))
                .rounded_full()
                .bg(rgb(bg))
                .text_color(rgb(t.on_primary))
                .text_sm()
                .child(text_str)
                .into_any_element(),
        );
        self
    }

    /// Set a leading image/thumbnail placeholder (coloured rectangle).
    pub fn leading_image(mut self, emoji: impl Into<String>, bg: u32) -> Self {
        let emoji_str = emoji.into();
        self.leading = Some(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(56.0))
                .rounded(px(ShapeScale::SMALL))
                .bg(rgb(bg))
                .text_2xl()
                .child(emoji_str)
                .into_any_element(),
        );
        self
    }

    /// Set a custom leading element.
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
        self
    }

    // ── Content ──────────────────────────────────────────────────────

    /// Set the title text (primary line).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the subtitle text (secondary line).
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Set the supporting text (third line — only shown if subtitle is also set).
    pub fn supporting_text(mut self, text: impl Into<String>) -> Self {
        self.supporting = Some(text.into());
        self
    }

    // ── Trailing element ─────────────────────────────────────────────

    /// Set a trailing icon (emoji or short text string).
    pub fn trailing_icon(mut self, icon: impl Into<String>) -> Self {
        let icon_str = icon.into();
        let t = self.theme;
        self.trailing = Some(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.0))
                .text_xl()
                .text_color(rgb(t.on_surface_variant))
                .child(icon_str)
                .into_any_element(),
        );
        self
    }

    /// Set trailing text (e.g. a count, time, or metadata).
    pub fn trailing_text(mut self, text: impl Into<String>) -> Self {
        let text_str = text.into();
        let t = self.theme;
        self.trailing = Some(
            div()
                .text_sm()
                .text_color(rgb(t.on_surface_variant))
                .child(text_str)
                .into_any_element(),
        );
        self
    }

    /// Set a custom trailing element (e.g. a Switch or Checkbox).
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }

    // ── Behaviour ────────────────────────────────────────────────────

    /// Set the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Mark this tile as selected (active background tint).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Mark this tile as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set the density mode.
    pub fn density(mut self, density: ListTileDensity) -> Self {
        self.density = density;
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Show a divider line below this tile.
    pub fn divider(mut self, show: bool) -> Self {
        self.show_divider = show;
        self
    }
}

impl IntoElement for ListTile {
    type Element = <gpui::Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let has_supporting = self.supporting.is_some();

        // Vertical padding based on density and number of lines
        let py_val = match self.density {
            ListTileDensity::Standard => {
                if has_supporting {
                    12.0
                } else {
                    8.0
                }
            }
            ListTileDensity::Compact => 4.0,
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("list-tile".into()));

        let mut container = div().flex().flex_col().w_full();

        // Main row
        let mut row = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .px(px(16.0))
            .py(px(py_val))
            .gap(px(16.0));

        // Selected background
        if self.selected {
            row = row.bg(rgb(t.secondary_container));
        }

        // Cursor
        if self.on_click.is_some() && !self.disabled {
            row = row.cursor_pointer();
        }

        // Disabled opacity
        if self.disabled {
            row = row.opacity(0.38);
        }

        // Leading
        if let Some(leading_el) = self.leading {
            row = row.child(leading_el);
        }

        // Content column (title + subtitle + supporting)
        let mut content_col = div().flex().flex_col().flex_1().gap(px(2.0));

        if let Some(title_text) = self.title {
            content_col = content_col.child(
                div()
                    .text_base()
                    .line_height(px(24.0))
                    .text_color(rgb(t.on_surface))
                    .child(title_text),
            );
        }

        if let Some(subtitle_text) = self.subtitle {
            content_col = content_col.child(
                div()
                    .text_sm()
                    .line_height(px(20.0))
                    .text_color(rgb(t.on_surface_variant))
                    .child(subtitle_text),
            );
        }

        if let Some(supporting_text) = self.supporting {
            content_col = content_col.child(
                div()
                    .text_sm()
                    .line_height(px(20.0))
                    .text_color(rgb(t.on_surface_variant))
                    .child(supporting_text),
            );
        }

        row = row.child(content_col);

        // Trailing
        if let Some(trailing_el) = self.trailing {
            row = row.child(trailing_el);
        }

        // Click handler
        if let Some(handler) = self.on_click {
            if !self.disabled {
                row = row.on_mouse_down(MouseButton::Left, handler);
            }
        }

        container = container.child(row);

        // Divider
        if self.show_divider {
            container = container.child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .ml(px(16.0))
                    .bg(rgb(t.outline_variant)),
            );
        }

        container.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Divider
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Divider**.
///
/// A thin horizontal or vertical line used to separate content. MD3 defines
/// two divider types:
///
/// - **Full-width** — extends across the entire width of the container
/// - **Inset** — indented on the leading side (typically by 16dp)
///
/// # Examples
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile::Divider;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// let full = Divider::new(theme);
/// let inset = Divider::new(theme).inset(16.0);
/// let vertical = Divider::new(theme).vertical();
/// ```
pub struct Divider {
    theme: MaterialTheme,
    /// Leading inset in logical pixels (0 = full-width).
    inset_start: f32,
    /// Trailing inset in logical pixels.
    inset_end: f32,
    /// Whether the divider is vertical instead of horizontal.
    is_vertical: bool,
    /// Thickness of the divider line (default 1dp).
    thickness: f32,
}

impl Divider {
    /// Create a new full-width horizontal divider.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            inset_start: 0.0,
            inset_end: 0.0,
            is_vertical: false,
            thickness: 1.0,
        }
    }

    /// Set a leading inset (left indent in LTR).
    pub fn inset(mut self, start: f32) -> Self {
        self.inset_start = start;
        self
    }

    /// Set both leading and trailing insets.
    pub fn insets(mut self, start: f32, end: f32) -> Self {
        self.inset_start = start;
        self.inset_end = end;
        self
    }

    /// Make the divider vertical instead of horizontal.
    pub fn vertical(mut self) -> Self {
        self.is_vertical = true;
        self
    }

    /// Override the thickness (default 1dp).
    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self
    }

    /// Create a middle-inset divider (indented on both sides by 16dp).
    pub fn middle(theme: MaterialTheme) -> Self {
        Self::new(theme).insets(16.0, 16.0)
    }

    /// Create a full-bleed horizontal divider with standard inset (16dp start).
    pub fn inset_standard(theme: MaterialTheme) -> Self {
        Self::new(theme).inset(16.0)
    }
}

impl IntoElement for Divider {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let color = rgb(self.theme.outline_variant);

        if self.is_vertical {
            div()
                .id("divider-v")
                .w(px(self.thickness))
                .h_full()
                .mt(px(self.inset_start))
                .mb(px(self.inset_end))
                .bg(color)
                .into_element()
        } else {
            div()
                .id("divider-h")
                .w_full()
                .h(px(self.thickness))
                .ml(px(self.inset_start))
                .mr(px(self.inset_end))
                .bg(color)
                .into_element()
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Badge
// ═══════════════════════════════════════════════════════════════════════════════

/// The visual type of a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeType {
    /// A small dot indicator (no text) — 6×6dp.
    #[default]
    Dot,
    /// A larger pill with text content — 16dp tall, variable width.
    Label,
}

/// A Material Design 3 **Badge**.
///
/// A small status indicator that appears on or near an icon, avatar, or
/// other element. Badges come in two forms:
///
/// - **Dot** — a small 6×6dp circle indicating new content or status
/// - **Label** — a pill-shaped badge with text (e.g. "3", "99+", "New")
///
/// Badges use the `error` color by default for high visibility.
///
/// # Examples
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile::Badge;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// let dot = Badge::dot(theme);
/// let count = Badge::label("3", theme);
/// let large = Badge::label("99+", theme);
/// ```
pub struct Badge {
    theme: MaterialTheme,
    badge_type: BadgeType,
    text: Option<String>,
    /// Custom background colour override (None = use error).
    bg_override: Option<u32>,
    /// Custom text colour override (None = use on_error).
    fg_override: Option<u32>,
}

impl Badge {
    /// Create a **dot** badge (no text).
    pub fn dot(theme: MaterialTheme) -> Self {
        Self {
            theme,
            badge_type: BadgeType::Dot,
            text: None,
            bg_override: None,
            fg_override: None,
        }
    }

    /// Create a **label** badge with text content.
    pub fn label(text: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            theme,
            badge_type: BadgeType::Label,
            text: Some(text.into()),
            bg_override: None,
            fg_override: None,
        }
    }

    /// Override the background colour.
    pub fn bg(mut self, color: u32) -> Self {
        self.bg_override = Some(color);
        self
    }

    /// Override the text colour (label badges only).
    pub fn fg(mut self, color: u32) -> Self {
        self.fg_override = Some(color);
        self
    }
}

impl IntoElement for Badge {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let bg_color = self.bg_override.unwrap_or(t.error);
        let fg_color = self.fg_override.unwrap_or(t.on_error);

        match self.badge_type {
            BadgeType::Dot => div()
                .id("badge-dot")
                .size(px(6.0))
                .rounded_full()
                .bg(rgb(bg_color))
                .into_element(),

            BadgeType::Label => {
                let text = self.text.unwrap_or_default();
                let single_char = text.len() <= 1;

                let mut badge = div()
                    .id("badge-label")
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(16.0))
                    .rounded_full()
                    .bg(rgb(bg_color))
                    .text_color(rgb(fg_color))
                    .text_size(px(11.0))
                    .line_height(px(16.0));

                if single_char {
                    badge = badge.min_w(px(16.0));
                } else {
                    badge = badge.min_w(px(16.0)).px(px(4.0));
                }

                badge.child(text).into_element()
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Tooltip
// ═══════════════════════════════════════════════════════════════════════════════

/// The visual variant of a tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TooltipVariant {
    /// Plain tooltip — a single line of text on an inverse surface.
    #[default]
    Plain,
    /// Rich tooltip — supports a title, body text, and an optional action button.
    Rich,
}

/// A Material Design 3 **Tooltip**.
///
/// Tooltips display informative text when users hover over, focus on, or
/// tap an element. MD3 defines two tooltip variants:
///
/// - **Plain** — a short single-line label on `inverse_surface`
/// - **Rich** — a multi-line tooltip with optional title and action
///
/// Since GPUI does not have built-in hover-triggered overlays, this
/// component renders the tooltip as an inline element. In a real app,
/// you would show/hide it based on interaction state.
///
/// # Layout (Plain)
///
/// ```text
/// ╭──────────────────────╮
/// │  Save to favorites   │
/// ╰──────────────────────╯
/// ```
///
/// # Layout (Rich)
///
/// ```text
/// ╭──────────────────────────────────╮
/// │  Rich tooltip title              │
/// │  Supporting text that explains   │
/// │  the element in more detail.     │
/// │                       [Action]   │
/// ╰──────────────────────────────────╯
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile::Tooltip;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// let plain = Tooltip::plain("Save to favorites", theme);
/// let rich = Tooltip::rich(theme)
///     .title("Auto-delete")
///     .body("Items in the trash will be permanently deleted after 30 days.")
///     .action("Learn more", |_, _, _| {});
/// ```
pub struct Tooltip {
    theme: MaterialTheme,
    variant: TooltipVariant,
    text: Option<String>,
    title: Option<String>,
    body_text: Option<String>,
    action_label: Option<String>,
    action_handler: Option<ClickHandler>,
}

impl Tooltip {
    /// Create a **plain** tooltip with a single line of text.
    pub fn plain(text: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            theme,
            variant: TooltipVariant::Plain,
            text: Some(text.into()),
            title: None,
            body_text: None,
            action_label: None,
            action_handler: None,
        }
    }

    /// Create a **rich** tooltip (use builder methods to add content).
    pub fn rich(theme: MaterialTheme) -> Self {
        Self {
            theme,
            variant: TooltipVariant::Rich,
            text: None,
            title: None,
            body_text: None,
            action_label: None,
            action_handler: None,
        }
    }

    /// Set the title (rich tooltip only).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the body text (rich tooltip only).
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body_text = Some(body.into());
        self
    }

    /// Set an action button (rich tooltip only).
    pub fn action(
        mut self,
        label: impl Into<String>,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.action_label = Some(label.into());
        self.action_handler = Some(Box::new(handler));
        self
    }
}

impl IntoElement for Tooltip {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        match self.variant {
            TooltipVariant::Plain => {
                let text = self.text.unwrap_or_default();

                div()
                    .id("tooltip-plain")
                    .flex()
                    .items_center()
                    .px(px(8.0))
                    .py(px(4.0))
                    .min_h(px(24.0))
                    .rounded(px(ShapeScale::EXTRA_SMALL))
                    .bg(rgb(t.inverse_surface))
                    .text_color(rgb(t.inverse_on_surface))
                    .text_size(px(12.0))
                    .line_height(px(16.0))
                    .child(text)
                    .into_element()
            }

            TooltipVariant::Rich => {
                let mut container = div()
                    .id("tooltip-rich")
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .min_w(px(200.0))
                    .max_w(px(320.0))
                    .rounded(px(ShapeScale::MEDIUM))
                    .bg(rgb(t.surface_container))
                    .border_1()
                    .border_color(rgb(t.outline_variant));

                // Title
                if let Some(title_text) = self.title {
                    container = container.child(
                        div()
                            .text_base()
                            .line_height(px(24.0))
                            .text_color(rgb(t.on_surface))
                            .child(title_text),
                    );
                }

                // Body
                if let Some(body_content) = self.body_text {
                    container = container.child(
                        div()
                            .text_sm()
                            .line_height(px(20.0))
                            .text_color(rgb(t.on_surface_variant))
                            .child(body_content),
                    );
                }

                // Action button
                if let Some(label) = self.action_label {
                    let mut action_row = div().flex().flex_row().justify_end().pt(px(8.0));

                    let mut btn = div()
                        .id("tooltip-action")
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded(px(ShapeScale::FULL))
                        .text_sm()
                        .text_color(rgb(t.primary))
                        .cursor_pointer()
                        .child(label);

                    if let Some(handler) = self.action_handler {
                        btn = btn.on_mouse_down(MouseButton::Left, handler);
                    }

                    action_row = action_row.child(btn);
                    container = container.child(action_row);
                }

                container.into_element()
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  SegmentedButton
// ═══════════════════════════════════════════════════════════════════════════════

/// The selection mode for a segmented button group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SegmentedButtonMode {
    /// Single-select mode — exactly one segment is selected at a time.
    #[default]
    Single,
    /// Multi-select mode — any number of segments can be toggled.
    Multi,
}

/// Internal descriptor for a single segment.
struct SegmentEntry {
    /// Icon text (empty = no icon).
    icon: String,
    /// Label text.
    label: String,
    /// Whether this segment is currently selected.
    selected: bool,
    /// Click handler.
    on_click: ClickHandler,
    /// Whether this segment is disabled.
    disabled: bool,
}

/// A Material Design 3 **Segmented Button** group.
///
/// Segmented buttons help people select options, switch views, or sort
/// elements. They can be single-select (like radio buttons) or
/// multi-select (like checkboxes).
///
/// # Layout
///
/// ```text
/// ┌─────────┬─────────┬─────────┐
/// │ ✓ Day   │  Week   │  Month  │
/// └─────────┴─────────┴─────────┘
/// ```
///
/// Selected segments have a filled background with a check icon prefix.
/// The segments share equal width and are visually connected with shared
/// borders.
///
/// # Examples
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile::SegmentedButton;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// // Single-select
/// let segments = SegmentedButton::new(theme)
///     .segment("", "Day", current == 0, cx.listener(|this, _, _, cx| { this.view = 0; cx.notify(); }))
///     .segment("", "Week", current == 1, cx.listener(|this, _, _, cx| { this.view = 1; cx.notify(); }))
///     .segment("", "Month", current == 2, cx.listener(|this, _, _, cx| { this.view = 2; cx.notify(); }));
///
/// // Multi-select with icons
/// let filters = SegmentedButton::new(theme)
///     .multi()
///     .segment("$", "Budget", has_budget, |_, _, _| {})
///     .segment("⭐", "Favorites", has_fav, |_, _, _| {})
///     .segment("📍", "Nearby", has_nearby, |_, _, _| {});
/// ```
pub struct SegmentedButton {
    theme: MaterialTheme,
    mode: SegmentedButtonMode,
    segments: Vec<SegmentEntry>,
    density: ListTileDensity,
}

impl SegmentedButton {
    /// Create a new single-select segmented button group.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            mode: SegmentedButtonMode::Single,
            segments: Vec::new(),
            density: ListTileDensity::Standard,
        }
    }

    /// Switch to multi-select mode.
    pub fn multi(mut self) -> Self {
        self.mode = SegmentedButtonMode::Multi;
        self
    }

    /// Set the density mode (affects segment height).
    pub fn density(mut self, density: ListTileDensity) -> Self {
        self.density = density;
        self
    }

    /// Add a segment with an optional icon, label, selection state, and handler.
    ///
    /// Pass an empty string for `icon` to omit the icon.
    pub fn segment(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        selected: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.segments.push(SegmentEntry {
            icon: icon.into(),
            label: label.into(),
            selected,
            on_click: Box::new(on_click),
            disabled: false,
        });
        self
    }

    /// Add a disabled segment.
    pub fn disabled_segment(mut self, icon: impl Into<String>, label: impl Into<String>) -> Self {
        self.segments.push(SegmentEntry {
            icon: icon.into(),
            label: label.into(),
            selected: false,
            on_click: Box::new(|_, _, _| {}),
            disabled: true,
        });
        self
    }
}

impl IntoElement for SegmentedButton {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let count = self.segments.len();
        let seg_height = match self.density {
            ListTileDensity::Standard => 40.0,
            ListTileDensity::Compact => 32.0,
        };

        let mut row = div()
            .id("segmented-button")
            .flex()
            .flex_row()
            .items_center()
            .h(px(seg_height))
            .rounded(px(20.0))
            .border_1()
            .border_color(rgb(t.outline))
            .overflow_hidden();

        for (i, entry) in self.segments.into_iter().enumerate() {
            let is_first = i == 0;
            let _is_last = i == count - 1;

            let bg: Hsla = if entry.selected {
                color(t.secondary_container)
            } else {
                TRANSPARENT
            };

            let fg: Hsla = if entry.disabled {
                gpui::hsla(0.0, 0.0, 0.5, 0.38)
            } else if entry.selected {
                color(t.on_secondary_container)
            } else {
                color(t.on_surface)
            };

            let mut seg = div()
                .id(gpui::ElementId::Name(format!("seg-{i}").into()))
                .flex()
                .flex_1()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .h_full()
                .bg(bg)
                .text_sm()
                .text_color(fg)
                .cursor_pointer();

            // Minimum width per segment
            seg = seg.min_w(px(48.0)).px(px(12.0));

            // Inner border between segments (not on first)
            if !is_first {
                seg = seg.border_l_1().border_color(rgb(t.outline));
            }

            // Disabled state
            if entry.disabled {
                seg = seg.opacity(0.38);
            }

            // Check icon for selected segments
            if entry.selected {
                seg = seg.child(div().text_size(px(14.0)).child("✓"));
            }

            // Optional icon (only if not empty and not already showing check)
            if !entry.icon.is_empty() && !entry.selected {
                seg = seg.child(div().text_size(px(14.0)).child(entry.icon));
            }

            // Label
            seg = seg.child(entry.label);

            // Click handler
            if !entry.disabled {
                seg = seg.on_mouse_down(MouseButton::Left, entry.on_click);
            }

            row = row.child(seg);
        }

        row.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Chip variants
// ═══════════════════════════════════════════════════════════════════════════════

/// The type of a Material Design 3 chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChipType {
    /// Assist chip — helps the user complete a task (e.g. "Add to calendar").
    #[default]
    Assist,
    /// Filter chip — narrows content selection (toggleable, has check when selected).
    Filter,
    /// Input chip — represents a user-provided piece of information (e.g. a tag).
    /// Has a close/remove button.
    Input,
    /// Suggestion chip — a dynamically generated shortcut or recommendation.
    Suggestion,
}

/// A Material Design 3 **Chip**.
///
/// Chips are compact elements that represent an input, attribute, or action.
/// MD3 defines four chip types:
///
/// - **Assist** — guides the user toward a related action
/// - **Filter** — narrows content using tags (toggleable)
/// - **Input** — represents user input (tag, contact, etc.), removable
/// - **Suggestion** — dynamically generated shortcuts
///
/// # Layout
///
/// ```text
/// ┌───────────────────────┐
/// │  🎵  Music          ✕ │  ← Input chip with icon and close button
/// └───────────────────────┘
///
/// ┌──────────────────┐
/// │  ✓ Nearby        │  ← Filter chip (selected)
/// └──────────────────┘
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile::{Chip, ChipType};
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// // Assist chip
/// let assist = Chip::new("Add to calendar", theme)
///     .chip_type(ChipType::Assist)
///     .icon("📅")
///     .on_click(|_, _, _| {});
///
/// // Filter chip (toggleable)
/// let filter = Chip::new("Nearby", theme)
///     .chip_type(ChipType::Filter)
///     .selected(true)
///     .on_click(|_, _, _| {});
///
/// // Input chip with remove
/// let input = Chip::new("John Doe", theme)
///     .chip_type(ChipType::Input)
///     .icon("👤")
///     .on_remove(|_, _, _| {});
///
/// // Suggestion chip
/// let suggestion = Chip::new("Try this", theme)
///     .chip_type(ChipType::Suggestion)
///     .on_click(|_, _, _| {});
/// ```
pub struct Chip {
    theme: MaterialTheme,
    chip_type: ChipType,
    label: String,
    icon: Option<String>,
    selected: bool,
    disabled: bool,
    on_click: Option<ClickHandler>,
    on_remove: Option<ClickHandler>,
    id: Option<gpui::ElementId>,
    elevated: bool,
}

impl Chip {
    /// Create a new chip with the given label.
    pub fn new(label: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            theme,
            chip_type: ChipType::Assist,
            label: label.into(),
            icon: None,
            selected: false,
            disabled: false,
            on_click: None,
            on_remove: None,
            id: None,
            elevated: false,
        }
    }

    /// Set the chip type.
    pub fn chip_type(mut self, chip_type: ChipType) -> Self {
        self.chip_type = chip_type;
        self
    }

    /// Set a leading icon.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the selected state (relevant for Filter and Input chips).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Mark as disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set the main click handler (tap on the chip body).
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Set the remove handler (only visible on Input chips).
    pub fn on_remove(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_remove = Some(Box::new(handler));
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Use the elevated style (shadow/border for extra emphasis).
    pub fn elevated(mut self, elevated: bool) -> Self {
        self.elevated = elevated;
        self
    }
}

impl IntoElement for Chip {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        // Determine colors based on type and selection state (all Hsla for type consistency)
        let (bg, border, fg): (Hsla, Hsla, Hsla) = if self.disabled {
            (
                TRANSPARENT,
                gpui::hsla(0.0, 0.0, 0.5, 0.12),
                gpui::hsla(0.0, 0.0, 0.5, 0.38),
            )
        } else if self.selected {
            (
                color(t.secondary_container),
                color(t.secondary_container), // no border when selected
                color(t.on_secondary_container),
            )
        } else {
            (TRANSPARENT, color(t.outline), color(t.on_surface_variant))
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("chip".into()));

        let mut chip = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .h(px(32.0))
            .rounded(px(8.0))
            .text_sm()
            .cursor_pointer();

        // Background
        if self.selected || self.elevated {
            chip = chip.bg(bg);
        }

        // Border (not when selected/filled)
        if !self.selected {
            chip = chip.border_1().border_color(border);
        }

        // Elevated style (shadow simulation)
        if self.elevated && !self.selected {
            chip = chip
                .bg(rgb(t.surface_container_low))
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.08));
        }

        // Disabled opacity
        if self.disabled {
            chip = chip.opacity(0.38);
        }

        // Text color
        chip = chip.text_color(fg);

        // Padding — depends on whether there is a leading icon and/or trailing remove
        let has_icon = self.icon.is_some();
        let has_remove = self.on_remove.is_some() && self.chip_type == ChipType::Input;
        let show_check = self.selected && self.chip_type == ChipType::Filter;

        if has_icon || show_check {
            chip = chip.pl(px(8.0));
        } else {
            chip = chip.pl(px(16.0));
        }

        if has_remove {
            chip = chip.pr(px(8.0));
        } else {
            chip = chip.pr(px(16.0));
        }

        // Leading check mark (Filter chips when selected)
        if show_check {
            chip = chip.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(18.0))
                    .text_size(px(14.0))
                    .child("✓"),
            );
        }

        // Leading icon
        if let Some(icon_text) = self.icon {
            chip = chip.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(18.0))
                    .text_size(px(16.0))
                    .child(icon_text),
            );
        }

        // Label
        chip = chip.child(self.label);

        // Trailing remove button (Input chips only)
        if has_remove {
            let mut remove_btn = div()
                .id("chip-remove")
                .flex()
                .items_center()
                .justify_center()
                .size(px(18.0))
                .rounded_full()
                .text_size(px(12.0))
                .text_color(rgb(t.on_surface_variant))
                .cursor_pointer()
                .child("✕");

            if let Some(handler) = self.on_remove {
                remove_btn = remove_btn.on_mouse_down(MouseButton::Left, handler);
            }

            chip = chip.child(remove_btn);
        }

        // Main click handler
        if let Some(handler) = self.on_click {
            if !self.disabled {
                chip = chip.on_mouse_down(MouseButton::Left, handler);
            }
        }

        chip.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  BottomAppBar
// ═══════════════════════════════════════════════════════════════════════════════

/// Internal icon entry for the bottom app bar.
struct BottomAppBarIcon {
    icon: String,
    on_click: ClickHandler,
    selected: bool,
}

/// A Material Design 3 **Bottom App Bar**.
///
/// A bottom app bar displays navigation and key actions at the bottom of
/// the screen. Unlike a [`NavigationBar`](super::navigation_bar::NavigationBarBuilder),
/// the bottom app bar focuses on actions (not destinations) and includes
/// an optional FAB.
///
/// # Layout
///
/// ```text
/// ┌────────────────────────────────────────────────────────────┐
/// │  🔍    📧    📸    🗑️                              ╭────╮ │
/// │                                                    │ ✏️  │ │
/// │                                                    ╰────╯ │
/// └────────────────────────────────────────────────────────────┘
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile::BottomAppBar;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// let bar = BottomAppBar::new(theme)
///     .icon("🔍", false, cx.listener(|this, _, _, cx| { /* search */ }))
///     .icon("📧", false, cx.listener(|this, _, _, cx| { /* mail */ }))
///     .icon("📸", false, cx.listener(|this, _, _, cx| { /* camera */ }))
///     .icon("🗑️", false, cx.listener(|this, _, _, cx| { /* delete */ }))
///     .fab("✏️", cx.listener(|this, _, _, cx| { /* compose */ }));
/// ```
pub struct BottomAppBar {
    theme: MaterialTheme,
    icons: Vec<BottomAppBarIcon>,
    fab_icon: Option<String>,
    fab_on_click: Option<ClickHandler>,
}

impl BottomAppBar {
    /// Create a new bottom app bar.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            icons: Vec::new(),
            fab_icon: None,
            fab_on_click: None,
        }
    }

    /// Add an action icon to the left side of the bar.
    ///
    /// # Parameters
    ///
    /// - `icon` — emoji or short icon string
    /// - `selected` — whether this icon is in a selected/active state
    /// - `on_click` — handler invoked when the icon is tapped
    pub fn icon(
        mut self,
        icon: impl Into<String>,
        selected: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.icons.push(BottomAppBarIcon {
            icon: icon.into(),
            on_click: Box::new(on_click),
            selected,
        });
        self
    }

    /// Set the FAB (floating action button) displayed on the right side.
    ///
    /// The FAB is rendered as a regular-sized (56dp) rounded button inside
    /// the bar, following the MD3 bottom app bar spec.
    pub fn fab(
        mut self,
        icon: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.fab_icon = Some(icon.into());
        self.fab_on_click = Some(Box::new(on_click));
        self
    }
}

impl IntoElement for BottomAppBar {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let mut bar = div()
            .id("bottom-app-bar")
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(80.0))
            .px(px(4.0))
            .bg(rgb(t.surface_container));

        // Icons (left side)
        let mut icons_row = div().flex().flex_row().items_center().flex_1();

        for (i, entry) in self.icons.into_iter().enumerate() {
            let fg = if entry.selected {
                t.on_surface
            } else {
                t.on_surface_variant
            };

            let icon_btn = div()
                .id(gpui::ElementId::Name(
                    format!("bottom-app-bar-icon-{i}").into(),
                ))
                .flex()
                .items_center()
                .justify_center()
                .size(px(48.0))
                .rounded_full()
                .cursor_pointer()
                .text_xl()
                .text_color(rgb(fg))
                .child(entry.icon)
                .on_mouse_down(MouseButton::Left, entry.on_click);

            icons_row = icons_row.child(icon_btn);
        }

        bar = bar.child(icons_row);

        // FAB (right side)
        if let Some(fab_icon) = self.fab_icon {
            let mut fab = div()
                .id("bottom-app-bar-fab")
                .flex()
                .items_center()
                .justify_center()
                .size(px(56.0))
                .rounded(px(16.0))
                .bg(rgb(t.primary_container))
                .text_color(rgb(t.on_primary_container))
                .text_2xl()
                .cursor_pointer()
                .child(fab_icon);

            if let Some(handler) = self.fab_on_click {
                fab = fab.on_mouse_down(MouseButton::Left, handler);
            }

            bar = bar.child(div().pr(px(12.0)).child(fab));
        }

        bar.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ExpansionTile
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Expansion Tile** (collapsible list item).
///
/// An expansion tile is a list tile that can expand to reveal additional
/// content (children tiles, text, etc.). The expansion state is controlled
/// by the caller — this component does not own state.
///
/// # Layout
///
/// ```text
/// ┌────────────────────────────────────────────────────┐
/// │  📁  Documents                               ▼    │  ← header tile
/// │  ├── 📄  Report.pdf                               │  ← child (visible when expanded)
/// │  ├── 📄  Budget.xlsx                              │
/// │  └── 📄  Notes.txt                                │
/// └────────────────────────────────────────────────────┘
/// ```
///
/// # Examples
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile::ExpansionTile;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// let tile = ExpansionTile::new(theme)
///     .leading_icon("📁")
///     .title("Documents")
///     .expanded(self.docs_expanded)
///     .on_toggle(cx.listener(|this, _, _, cx| {
///         this.docs_expanded = !this.docs_expanded;
///         cx.notify();
///     }))
///     .child(ListTile::new(theme).leading_icon("📄").title("Report.pdf"))
///     .child(ListTile::new(theme).leading_icon("📄").title("Budget.xlsx"));
/// ```
pub struct ExpansionTile {
    theme: MaterialTheme,
    leading: Option<AnyElement>,
    title: Option<String>,
    subtitle: Option<String>,
    expanded: bool,
    on_toggle: Option<ClickHandler>,
    children: Vec<AnyElement>,
    id: Option<gpui::ElementId>,
    /// Whether to show a divider below the expanded content.
    show_divider: bool,
}

impl ExpansionTile {
    /// Create a new expansion tile.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            leading: None,
            title: None,
            subtitle: None,
            expanded: false,
            on_toggle: None,
            children: Vec::new(),
            id: None,
            show_divider: false,
        }
    }

    /// Set a leading icon.
    pub fn leading_icon(mut self, icon: impl Into<String>) -> Self {
        let icon_str = icon.into();
        let t = self.theme;
        self.leading = Some(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.0))
                .text_xl()
                .text_color(rgb(t.on_surface_variant))
                .child(icon_str)
                .into_any_element(),
        );
        self
    }

    /// Set a custom leading element.
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
        self
    }

    /// Set the title text.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the subtitle text.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Set the expanded state.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    /// Set the toggle handler, invoked when the header tile is tapped.
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Box::new(handler));
        self
    }

    /// Add a child element (visible when expanded).
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Show a divider below the expanded content.
    pub fn divider(mut self, show: bool) -> Self {
        self.show_divider = show;
        self
    }
}

impl IntoElement for ExpansionTile {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("expansion-tile".into()));

        let mut container = div()
            .id("expansion-tile-container")
            .flex()
            .flex_col()
            .w_full();

        // Header tile (always visible)
        let mut header_row = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .px(px(16.0))
            .py(px(8.0))
            .gap(px(16.0))
            .cursor_pointer();

        // Leading
        if let Some(leading_el) = self.leading {
            header_row = header_row.child(leading_el);
        }

        // Title column
        let mut title_col = div().flex().flex_col().flex_1().gap(px(2.0));

        if let Some(title_text) = self.title {
            title_col = title_col.child(
                div()
                    .text_base()
                    .line_height(px(24.0))
                    .text_color(rgb(t.on_surface))
                    .child(title_text),
            );
        }

        if let Some(subtitle_text) = self.subtitle {
            title_col = title_col.child(
                div()
                    .text_sm()
                    .line_height(px(20.0))
                    .text_color(rgb(t.on_surface_variant))
                    .child(subtitle_text),
            );
        }

        header_row = header_row.child(title_col);

        // Expand/collapse indicator
        let indicator_icon = if self.expanded { "▲" } else { "▼" };
        header_row = header_row.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.0))
                .text_size(px(12.0))
                .text_color(rgb(t.on_surface_variant))
                .child(indicator_icon),
        );

        // Toggle handler on header
        if let Some(handler) = self.on_toggle {
            header_row = header_row.on_mouse_down(MouseButton::Left, handler);
        }

        container = container.child(header_row);

        // Children (visible when expanded)
        if self.expanded && !self.children.is_empty() {
            let mut children_container = div().flex().flex_col().w_full().pl(px(16.0));

            for child_el in self.children {
                children_container = children_container.child(child_el);
            }

            container = container.child(children_container);
        }

        // Divider
        if self.show_divider {
            container = container.child(div().w_full().h(px(1.0)).bg(rgb(t.outline_variant)));
        }

        container.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Legacy chip compatibility
// ═══════════════════════════════════════════════════════════════════════════════

/// Legacy helper: renders a single chip (static, limited interactivity).
///
/// Prefer [`Chip`] for interactive use.
pub fn chip(
    label: &str,
    selected: bool,
    outline: u32,
    on_surface: u32,
    selected_bg: u32,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(32.0))
        .px_3()
        .rounded(px(8.0))
        .when(selected, |d| d.bg(rgb(selected_bg)))
        .when(!selected, |d| d.border_1().border_color(rgb(outline)))
        .text_sm()
        .text_color(rgb(on_surface))
        .child(label.to_string())
}

/// Legacy composite showcase — assist and filter chips.
///
/// Prefer using [`Chip`] with explicit types for new code.
pub fn chips(dark: bool) -> impl IntoElement {
    let outline = if dark { 0x938f99_u32 } else { 0x79747e };
    let on_surface = if dark { 0xe6e1e5_u32 } else { 0x1c1b1f };
    let selected_bg = if dark { 0x4a4458_u32 } else { 0xe8def8 };

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(chip("🎵 Music", false, outline, on_surface, selected_bg))
                .child(chip("📸 Photos", true, outline, on_surface, selected_bg))
                .child(chip("🎬 Videos", false, outline, on_surface, selected_bg))
                .child(chip("📄 Docs", false, outline, on_surface, selected_bg)),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(chip("✓ Nearby", true, outline, on_surface, selected_bg))
                .child(chip("Open Now", false, outline, on_surface, selected_bg))
                .child(chip("✓ 4+ Stars", true, outline, on_surface, selected_bg))
                .child(chip("Free WiFi", false, outline, on_surface, selected_bg)),
        )
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Renders a comprehensive showcase of ListTile, Divider, Badge, Tooltip,
/// SegmentedButton, Chip, BottomAppBar, and ExpansionTile components.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::list_tile;
///
/// let demo = list_tile::list_tile_demo(true); // dark mode
/// ```
pub fn list_tile_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        // ── ListTile variants ────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("LIST TILES"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        // One-line tile
                        .child(
                            ListTile::new(theme)
                                .leading_icon("📧")
                                .title("One-line item")
                                .trailing_text("24")
                                .divider(true)
                                .id("tile-one"),
                        )
                        // Two-line tile with avatar
                        .child(
                            ListTile::new(theme)
                                .leading_avatar("JD", theme.primary)
                                .title("Jane Doe")
                                .subtitle("Online now")
                                .trailing_icon("💬")
                                .divider(true)
                                .id("tile-two"),
                        )
                        // Three-line tile
                        .child(
                            ListTile::new(theme)
                                .leading_icon("📄")
                                .title("Document.pdf")
                                .subtitle("Last modified 2 hours ago")
                                .supporting_text("Shared with 3 people")
                                .trailing_icon("⋮")
                                .divider(true)
                                .id("tile-three"),
                        )
                        // Selected tile
                        .child(
                            ListTile::new(theme)
                                .leading_icon("⭐")
                                .title("Selected item")
                                .subtitle("This item is selected")
                                .selected(true)
                                .divider(true)
                                .id("tile-selected"),
                        )
                        // Disabled tile
                        .child(
                            ListTile::new(theme)
                                .leading_icon("🔒")
                                .title("Disabled item")
                                .subtitle("This item is not interactive")
                                .disabled(true)
                                .id("tile-disabled"),
                        ),
                ),
        )
        // ── Dividers ─────────────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("DIVIDERS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .p_4()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(theme.on_surface))
                                .child("Content above full-width divider"),
                        )
                        .child(Divider::new(theme))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(theme.on_surface))
                                .child("Content between dividers"),
                        )
                        .child(Divider::inset_standard(theme))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(theme.on_surface))
                                .child("Content below inset divider"),
                        )
                        .child(Divider::middle(theme))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(theme.on_surface))
                                .child("Content below middle divider"),
                        ),
                ),
        )
        // ── Badges ───────────────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("BADGES"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .child(Badge::dot(theme))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme.on_surface_variant))
                                        .child("Dot"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .child(Badge::label("3", theme))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme.on_surface_variant))
                                        .child("Single"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .child(Badge::label("24", theme))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme.on_surface_variant))
                                        .child("Double"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .child(Badge::label("99+", theme))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme.on_surface_variant))
                                        .child("Overflow"),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .child(Badge::label("New", theme))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme.on_surface_variant))
                                        .child("Text"),
                                ),
                        )
                        // Badge on an icon
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_1()
                                .child(
                                    div().relative().text_xl().child("🔔").child(
                                        div()
                                            .absolute()
                                            .top(px(-2.0))
                                            .right(px(-6.0))
                                            .child(Badge::label("5", theme)),
                                    ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme.on_surface_variant))
                                        .child("On icon"),
                                ),
                        ),
                ),
        )
        // ── Tooltips ─────────────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("TOOLTIPS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(Tooltip::plain("Save to favorites", theme))
                        .child(
                            Tooltip::rich(theme)
                                .title("Auto-delete")
                                .body(
                                    "Items in the trash will be permanently deleted after 30 days.",
                                )
                                .action("Learn more", |_, _, _| {}),
                        ),
                ),
        )
        // ── Segmented Buttons ────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SEGMENTED BUTTONS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        // Single-select
                        .child(
                            SegmentedButton::new(theme)
                                .segment("", "Day", true, |_, _, _| {})
                                .segment("", "Week", false, |_, _, _| {})
                                .segment("", "Month", false, |_, _, _| {}),
                        )
                        // Multi-select with icons
                        .child(
                            SegmentedButton::new(theme)
                                .multi()
                                .segment("$", "Budget", true, |_, _, _| {})
                                .segment("⭐", "Favorites", false, |_, _, _| {})
                                .segment("📍", "Nearby", true, |_, _, _| {}),
                        )
                        // With disabled segment
                        .child(
                            SegmentedButton::new(theme)
                                .segment("", "Enabled", false, |_, _, _| {})
                                .segment("", "Selected", true, |_, _, _| {})
                                .disabled_segment("", "Disabled"),
                        ),
                ),
        )
        // ── Chip variants ────────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("CHIP VARIANTS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        // Assist chips
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Chip::new("Add to calendar", theme)
                                        .chip_type(ChipType::Assist)
                                        .icon("📅")
                                        .id("chip-assist-1"),
                                )
                                .child(
                                    Chip::new("Get directions", theme)
                                        .chip_type(ChipType::Assist)
                                        .icon("🗺️")
                                        .id("chip-assist-2"),
                                )
                                .child(
                                    Chip::new("Elevated assist", theme)
                                        .chip_type(ChipType::Assist)
                                        .icon("🔗")
                                        .elevated(true)
                                        .id("chip-assist-elev"),
                                ),
                        )
                        // Filter chips
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Chip::new("Nearby", theme)
                                        .chip_type(ChipType::Filter)
                                        .selected(true)
                                        .id("chip-filter-1"),
                                )
                                .child(
                                    Chip::new("Open Now", theme)
                                        .chip_type(ChipType::Filter)
                                        .selected(false)
                                        .id("chip-filter-2"),
                                )
                                .child(
                                    Chip::new("4+ Stars", theme)
                                        .chip_type(ChipType::Filter)
                                        .selected(true)
                                        .id("chip-filter-3"),
                                ),
                        )
                        // Input chips
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Chip::new("John Doe", theme)
                                        .chip_type(ChipType::Input)
                                        .icon("👤")
                                        .on_remove(|_, _, _| {})
                                        .id("chip-input-1"),
                                )
                                .child(
                                    Chip::new("jane@example.com", theme)
                                        .chip_type(ChipType::Input)
                                        .icon("📧")
                                        .on_remove(|_, _, _| {})
                                        .id("chip-input-2"),
                                ),
                        )
                        // Suggestion chips
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Chip::new("I'll be there", theme)
                                        .chip_type(ChipType::Suggestion)
                                        .id("chip-sug-1"),
                                )
                                .child(
                                    Chip::new("Sounds good", theme)
                                        .chip_type(ChipType::Suggestion)
                                        .id("chip-sug-2"),
                                )
                                .child(
                                    Chip::new("Thanks!", theme)
                                        .chip_type(ChipType::Suggestion)
                                        .id("chip-sug-3"),
                                ),
                        )
                        // Disabled chip
                        .child(
                            div().flex().flex_row().flex_wrap().gap_2().child(
                                Chip::new("Disabled", theme)
                                    .chip_type(ChipType::Filter)
                                    .disabled(true)
                                    .id("chip-disabled"),
                            ),
                        ),
                ),
        )
        // ── Bottom App Bar ───────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("BOTTOM APP BAR"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            BottomAppBar::new(theme)
                                .icon("🔍", false, |_, _, _| {})
                                .icon("📧", false, |_, _, _| {})
                                .icon("📸", false, |_, _, _| {})
                                .icon("🗑️", false, |_, _, _| {})
                                .fab("✏️", |_, _, _| {}),
                        ),
                ),
        )
        // ── Expansion Tile ───────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("EXPANSION TILES"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        // Collapsed tile
                        .child(
                            ExpansionTile::new(theme)
                                .leading_icon("📁")
                                .title("Collapsed folder")
                                .subtitle("3 items")
                                .expanded(false)
                                .divider(true)
                                .id("exp-collapsed")
                                .child(
                                    ListTile::new(theme)
                                        .leading_icon("📄")
                                        .title("Hidden item")
                                        .id("exp-child-hidden"),
                                ),
                        )
                        // Expanded tile
                        .child(
                            ExpansionTile::new(theme)
                                .leading_icon("📁")
                                .title("Expanded folder")
                                .subtitle("3 items")
                                .expanded(true)
                                .divider(true)
                                .id("exp-expanded")
                                .child(
                                    ListTile::new(theme)
                                        .leading_icon("📄")
                                        .title("Report.pdf")
                                        .divider(true)
                                        .id("exp-child-1"),
                                )
                                .child(
                                    ListTile::new(theme)
                                        .leading_icon("📄")
                                        .title("Budget.xlsx")
                                        .divider(true)
                                        .id("exp-child-2"),
                                )
                                .child(
                                    ListTile::new(theme)
                                        .leading_icon("📄")
                                        .title("Notes.txt")
                                        .id("exp-child-3"),
                                ),
                        )
                        // Another collapsed tile
                        .child(
                            ExpansionTile::new(theme)
                                .leading_icon("📁")
                                .title("Empty folder")
                                .expanded(false)
                                .id("exp-empty"),
                        ),
                ),
        )
}
