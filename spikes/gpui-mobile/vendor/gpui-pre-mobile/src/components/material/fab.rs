//! Material Design 3 Floating Action Button (FAB) components.
//!
//! FABs represent the primary action of a screen. MD3 defines four FAB sizes:
//!
//! - **Small** — 40 × 40 dp, 12 dp corner radius
//! - **Regular** — 56 × 56 dp, 16 dp corner radius
//! - **Large** — 96 × 96 dp, 28 dp corner radius
//! - **Extended** — 56 dp tall, variable width, icon + label
//!
//! Each FAB also supports three color variants:
//!
//! - **Primary** (default) — primary-container background
//! - **Secondary** — secondary-container background
//! - **Tertiary** — tertiary-container background
//! - **Surface** — surface-container-high background (lowered emphasis)
//!
//! # Examples
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::fab::*;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Regular FAB
//! let fab = FloatingActionButton::new("✏️", theme)
//!     .on_click(cx.listener(|this, _, _, cx| { /* compose */ }));
//!
//! // Small FAB
//! let small = FloatingActionButton::new("+", theme)
//!     .small()
//!     .secondary()
//!     .on_click(|_, _, _| {});
//!
//! // Large FAB
//! let large = FloatingActionButton::new("📷", theme)
//!     .large()
//!     .tertiary();
//!
//! // Extended FAB
//! let extended = ExtendedFab::new("✏️", "Compose", theme)
//!     .on_click(cx.listener(|this, _, _, cx| { /* compose */ }));
//! ```

use gpui::{div, prelude::*, px, rgb, MouseButton, MouseDownEvent, Stateful, Window};

use super::theme::MaterialTheme;

// ── Constants ────────────────────────────────────────────────────────────────

const FAB_SMALL_SIZE: f32 = 40.0;
const FAB_SMALL_RADIUS: f32 = 12.0;
const FAB_SMALL_ICON_SIZE: f32 = 24.0;

const FAB_REGULAR_SIZE: f32 = 56.0;
const FAB_REGULAR_RADIUS: f32 = 16.0;
const FAB_REGULAR_ICON_SIZE: f32 = 24.0;

const FAB_LARGE_SIZE: f32 = 96.0;
const FAB_LARGE_RADIUS: f32 = 28.0;
const FAB_LARGE_ICON_SIZE: f32 = 36.0;

const FAB_EXTENDED_HEIGHT: f32 = 56.0;
const FAB_EXTENDED_RADIUS: f32 = 16.0;
const FAB_EXTENDED_PADDING_H: f32 = 16.0;
const FAB_EXTENDED_GAP: f32 = 12.0;

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ── FAB Size ─────────────────────────────────────────────────────────────────

/// The size variant of a floating action button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FabSize {
    /// 40 × 40 dp — for compact layouts or secondary actions.
    Small,
    /// 56 × 56 dp — the default FAB size.
    #[default]
    Regular,
    /// 96 × 96 dp — for the most important action, maximally prominent.
    Large,
}

impl FabSize {
    /// Returns the side length in logical pixels for this size variant.
    fn side(self) -> f32 {
        match self {
            FabSize::Small => FAB_SMALL_SIZE,
            FabSize::Regular => FAB_REGULAR_SIZE,
            FabSize::Large => FAB_LARGE_SIZE,
        }
    }

    /// Returns the corner radius in logical pixels for this size variant.
    fn radius(self) -> f32 {
        match self {
            FabSize::Small => FAB_SMALL_RADIUS,
            FabSize::Regular => FAB_REGULAR_RADIUS,
            FabSize::Large => FAB_LARGE_RADIUS,
        }
    }

    /// Returns the icon font size in logical pixels for this size variant.
    fn icon_size(self) -> f32 {
        match self {
            FabSize::Small => FAB_SMALL_ICON_SIZE,
            FabSize::Regular => FAB_REGULAR_ICON_SIZE,
            FabSize::Large => FAB_LARGE_ICON_SIZE,
        }
    }
}

// ── FAB Color Variant ────────────────────────────────────────────────────────

/// The color variant of a floating action button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FabColor {
    /// Primary container background (default).
    #[default]
    Primary,
    /// Secondary container background.
    Secondary,
    /// Tertiary container background.
    Tertiary,
    /// Surface container high background (lowered emphasis).
    Surface,
}

impl FabColor {
    /// Returns the (background, foreground) color pair for this variant.
    fn colors(self, theme: &MaterialTheme) -> (u32, u32) {
        match self {
            FabColor::Primary => (theme.primary_container, theme.on_primary_container),
            FabColor::Secondary => (theme.secondary_container, theme.on_secondary_container),
            FabColor::Tertiary => (theme.tertiary_container, theme.on_tertiary_container),
            FabColor::Surface => (theme.surface_container_high, theme.primary),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  FloatingActionButton
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Floating Action Button** (FAB).
///
/// A square (rounded-corner) button containing a single icon. FABs
/// represent the primary action of a screen and float above the main
/// content, typically in the bottom-right corner (positioned by the
/// parent layout or [`Scaffold`](super::scaffold::Scaffold)).
///
/// By default the FAB is **regular** size with **primary** container
/// colors. Use the builder methods to customize size, color, and behavior.
pub struct FloatingActionButton {
    icon: String,
    theme: MaterialTheme,
    fab_size: FabSize,
    color: FabColor,
    on_click: Option<ClickHandler>,
    id: Option<gpui::ElementId>,
    /// If true, apply a lowered elevation style (no shadow simulation).
    lowered: bool,
}

impl FloatingActionButton {
    /// Create a new regular-size FAB with the given icon.
    ///
    /// The `icon` parameter is typically an emoji or short icon string
    /// (e.g. `"✏️"`, `"+"`, `"📷"`).
    pub fn new(icon: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            icon: icon.into(),
            theme,
            fab_size: FabSize::Regular,
            color: FabColor::Primary,
            on_click: None,
            id: None,
            lowered: false,
        }
    }

    // ── Size setters ─────────────────────────────────────────────────

    /// Use the **small** size (40 × 40 dp).
    pub fn small(mut self) -> Self {
        self.fab_size = FabSize::Small;
        self
    }

    /// Use the **regular** size (56 × 56 dp) — this is the default.
    pub fn regular(mut self) -> Self {
        self.fab_size = FabSize::Regular;
        self
    }

    /// Use the **large** size (96 × 96 dp).
    pub fn large(mut self) -> Self {
        self.fab_size = FabSize::Large;
        self
    }

    /// Set the size explicitly.
    pub fn fab_size(mut self, size: FabSize) -> Self {
        self.fab_size = size;
        self
    }

    // ── Color setters ────────────────────────────────────────────────

    /// Use the **primary** container colors (default).
    pub fn primary(mut self) -> Self {
        self.color = FabColor::Primary;
        self
    }

    /// Use the **secondary** container colors.
    pub fn secondary(mut self) -> Self {
        self.color = FabColor::Secondary;
        self
    }

    /// Use the **tertiary** container colors.
    pub fn tertiary(mut self) -> Self {
        self.color = FabColor::Tertiary;
        self
    }

    /// Use the **surface** container colors (lowered emphasis).
    pub fn surface(mut self) -> Self {
        self.color = FabColor::Surface;
        self
    }

    /// Set the color variant explicitly.
    pub fn fab_color(mut self, color: FabColor) -> Self {
        self.color = color;
        self
    }

    // ── Behavior ─────────────────────────────────────────────────────

    /// Set the click handler.
    ///
    /// The handler signature matches GPUI's `on_mouse_down`, so
    /// `cx.listener(...)` can be used directly.
    pub fn on_click(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Apply the "lowered" elevation style.
    ///
    /// Lowered FABs sit closer to the surface and omit the shadow
    /// simulation. Used when the FAB is embedded in a container that
    /// already has elevation.
    pub fn lowered(mut self, lowered: bool) -> Self {
        self.lowered = lowered;
        self
    }
}

impl IntoElement for FloatingActionButton {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let side = self.fab_size.side();
        let radius = self.fab_size.radius();
        let _icon_size = self.fab_size.icon_size();
        let (bg, fg) = self.color.colors(&self.theme);

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("fab".into()));

        let mut fab = div()
            .id(elem_id)
            .flex()
            .items_center()
            .justify_center()
            .size(px(side))
            .rounded(px(radius))
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .cursor_pointer();

        // Set icon text size based on FAB size
        fab = match self.fab_size {
            FabSize::Small => fab.text_lg(),
            FabSize::Regular => fab.text_2xl(),
            FabSize::Large => fab.text_3xl(),
        };

        // Shadow simulation for non-lowered FABs (a subtle border)
        if !self.lowered {
            fab = fab.border_1().border_color(gpui::hsla(0.0, 0.0, 0.0, 0.06));
        }

        fab = fab.child(self.icon);

        if let Some(handler) = self.on_click {
            fab = fab.on_mouse_down(MouseButton::Left, handler);
        }

        fab.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ExtendedFab
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Extended Floating Action Button**.
///
/// An extended FAB is wider than a standard FAB and includes both an
/// icon and a text label. It has a fixed height of 56 dp, variable
/// width, and 16 dp corner radius.
///
/// Extended FABs are useful when the action needs descriptive text
/// (e.g. "Compose", "Create", "New Chat").
pub struct ExtendedFab {
    icon: String,
    label: String,
    theme: MaterialTheme,
    color: FabColor,
    on_click: Option<ClickHandler>,
    id: Option<gpui::ElementId>,
    lowered: bool,
    /// If true, only show the icon (collapse the label). Useful for
    /// scroll-to-collapse behavior.
    collapsed: bool,
}

impl ExtendedFab {
    /// Create a new extended FAB with an icon and label.
    pub fn new(icon: impl Into<String>, label: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            icon: icon.into(),
            label: label.into(),
            theme,
            color: FabColor::Primary,
            on_click: None,
            id: None,
            lowered: false,
            collapsed: false,
        }
    }

    /// Use the **primary** container colors (default).
    pub fn primary(mut self) -> Self {
        self.color = FabColor::Primary;
        self
    }

    /// Use the **secondary** container colors.
    pub fn secondary(mut self) -> Self {
        self.color = FabColor::Secondary;
        self
    }

    /// Use the **tertiary** container colors.
    pub fn tertiary(mut self) -> Self {
        self.color = FabColor::Tertiary;
        self
    }

    /// Use the **surface** container colors.
    pub fn surface(mut self) -> Self {
        self.color = FabColor::Surface;
        self
    }

    /// Set the color variant explicitly.
    pub fn fab_color(mut self, color: FabColor) -> Self {
        self.color = color;
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

    /// Set an explicit element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Apply the "lowered" elevation style.
    pub fn lowered(mut self, lowered: bool) -> Self {
        self.lowered = lowered;
        self
    }

    /// Collapse the extended FAB to show only the icon (no label).
    ///
    /// This is useful for implementing scroll-to-collapse behavior where
    /// the extended FAB shrinks to a regular FAB on scroll.
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl IntoElement for ExtendedFab {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let (bg, fg) = self.color.colors(&self.theme);

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("ext-fab".into()));

        let mut fab = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .h(px(FAB_EXTENDED_HEIGHT))
            .rounded(px(FAB_EXTENDED_RADIUS))
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .cursor_pointer();

        if self.collapsed {
            // When collapsed, center the icon in a square
            fab = fab
                .justify_center()
                .w(px(FAB_EXTENDED_HEIGHT))
                .child(div().text_xl().child(self.icon));
        } else {
            fab = fab
                .gap(px(FAB_EXTENDED_GAP))
                .px(px(FAB_EXTENDED_PADDING_H))
                .child(div().text_xl().child(self.icon))
                .child(div().text_sm().child(self.label));
        }

        if !self.lowered {
            fab = fab.border_1().border_color(gpui::hsla(0.0, 0.0, 0.0, 0.06));
        }

        if let Some(handler) = self.on_click {
            fab = fab.on_mouse_down(MouseButton::Left, handler);
        }

        fab.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Legacy compatibility
// ═══════════════════════════════════════════════════════════════════════════════

/// Legacy composite showcase — all FAB sizes in a row.
///
/// Kept for backward compatibility with the existing component gallery.
/// New code should use [`FloatingActionButton`] and [`ExtendedFab`] directly.
pub fn fabs(dark: bool) -> impl IntoElement {
    let md_primary_container = if dark { 0x4f378b_u32 } else { 0xeaddff };
    let md_on_primary_container = if dark { 0xeaddff_u32 } else { 0x21005e };
    let md_secondary_container = if dark { 0x4a4458_u32 } else { 0xe8def8 };
    let md_on_secondary_container = if dark { 0xe8def8_u32 } else { 0x1d192b };
    let md_tertiary_container = if dark { 0x633b48_u32 } else { 0xffd8e4 };
    let md_on_tertiary_container = if dark { 0xffd8e4_u32 } else { 0x31111d };

    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_end()
        .gap_3()
        // Small FAB
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(FAB_SMALL_SIZE))
                .rounded(px(FAB_SMALL_RADIUS))
                .bg(rgb(md_secondary_container))
                .text_lg()
                .text_color(rgb(md_on_secondary_container))
                .child("+"),
        )
        // Regular FAB
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(FAB_REGULAR_SIZE))
                .rounded(px(FAB_REGULAR_RADIUS))
                .bg(rgb(md_primary_container))
                .text_2xl()
                .text_color(rgb(md_on_primary_container))
                .child("✏️"),
        )
        // Large FAB
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(FAB_LARGE_SIZE))
                .rounded(px(FAB_LARGE_RADIUS))
                .bg(rgb(md_tertiary_container))
                .text_3xl()
                .text_color(rgb(md_on_tertiary_container))
                .child("📷"),
        )
        // Extended FAB
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .h(px(FAB_EXTENDED_HEIGHT))
                .rounded(px(FAB_EXTENDED_RADIUS))
                .bg(rgb(md_primary_container))
                .text_color(rgb(md_on_primary_container))
                .child(div().text_lg().child("✏️"))
                .child(div().text_sm().child("Compose")),
        )
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Renders a comprehensive showcase of all MD3 FAB types and color
/// variants using the struct-based builders.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::fab;
///
/// let demo = fab::fab_demo(true); // dark mode
/// ```
pub fn fab_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        // Size variants (primary color)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SIZE VARIANTS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_end()
                        .gap_3()
                        .child(FloatingActionButton::new("+", theme).small().id("fab-sm"))
                        .child(
                            FloatingActionButton::new("✏️", theme)
                                .regular()
                                .id("fab-reg"),
                        )
                        .child(FloatingActionButton::new("📷", theme).large().id("fab-lg")),
                ),
        )
        // Color variants (regular size)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("COLOR VARIANTS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(
                            FloatingActionButton::new("✏️", theme)
                                .primary()
                                .id("fab-c-pri"),
                        )
                        .child(
                            FloatingActionButton::new("🔗", theme)
                                .secondary()
                                .id("fab-c-sec"),
                        )
                        .child(
                            FloatingActionButton::new("♥", theme)
                                .tertiary()
                                .id("fab-c-ter"),
                        )
                        .child(
                            FloatingActionButton::new("⚙️", theme)
                                .surface()
                                .id("fab-c-srf"),
                        ),
                ),
        )
        // Extended FABs
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("EXTENDED"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            ExtendedFab::new("✏️", "Compose", theme)
                                .primary()
                                .id("ext-fab-pri"),
                        )
                        .child(
                            ExtendedFab::new("➕", "Create", theme)
                                .secondary()
                                .id("ext-fab-sec"),
                        )
                        .child(
                            ExtendedFab::new("💬", "New Chat", theme)
                                .tertiary()
                                .id("ext-fab-ter"),
                        ),
                ),
        )
        // Lowered variant
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("LOWERED"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(
                            FloatingActionButton::new("✏️", theme)
                                .lowered(true)
                                .id("fab-low"),
                        )
                        .child(
                            ExtendedFab::new("✏️", "Compose", theme)
                                .lowered(true)
                                .id("ext-fab-low"),
                        ),
                ),
        )
        // Collapsed extended FAB
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("COLLAPSED EXTENDED"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(
                            ExtendedFab::new("✏️", "Compose", theme)
                                .collapsed(false)
                                .id("ext-fab-expanded"),
                        )
                        .child(
                            ExtendedFab::new("✏️", "Compose", theme)
                                .collapsed(true)
                                .id("ext-fab-collapsed"),
                        ),
                ),
        )
}
