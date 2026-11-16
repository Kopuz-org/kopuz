//! Material Design 3 Navigation Rail component.
//!
//! A navigation rail provides access to primary destinations in an app
//! when using a tablet or desktop layout. It is a vertical bar displayed
//! along the leading edge (left in LTR) of the screen, containing 3–7
//! destination icons with optional labels.
//!
//! MD3 navigation rails feature:
//!
//! - A vertical column of 3–7 icon + label destinations
//! - An **active indicator** pill behind the selected item's icon
//! - An optional **FAB** or **menu icon** at the top
//! - An optional header area above the destinations
//! - Compact (icon-only) and standard (icon + label) modes
//!
//! # Examples
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::navigation_rail::NavigationRail;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! let rail = NavigationRail::new(theme)
//!     .item("🏠", "Home", true, cx.listener(move |this, _, _, cx| {
//!         this.navigate_to(Screen::Home);
//!         cx.notify();
//!     }))
//!     .item("🔍", "Search", false, cx.listener(move |this, _, _, cx| {
//!         this.navigate_to(Screen::Search);
//!         cx.notify();
//!     }))
//!     .item("📚", "Library", false, cx.listener(move |this, _, _, cx| {
//!         this.navigate_to(Screen::Library);
//!         cx.notify();
//!     }))
//!     .fab("✏️", cx.listener(move |this, _, _, cx| {
//!         this.compose_new();
//!         cx.notify();
//!     }));
//! ```

use gpui::{div, prelude::*, px, rgb, AnyElement, MouseButton, MouseDownEvent, Window};

use super::theme::{MaterialTheme, ShapeScale};

// ── Constants ────────────────────────────────────────────────────────────────

/// Standard rail width (80 dp).
const RAIL_WIDTH: f32 = 80.0;

/// Compact rail width (icon-only, no labels — 56 dp).
const RAIL_WIDTH_COMPACT: f32 = 56.0;

/// Height of each navigation item destination area.
const ITEM_HEIGHT: f32 = 56.0;

/// Gap between the icon and label within a rail item.
const ICON_LABEL_GAP: f32 = 4.0;

/// Active indicator pill dimensions.
const INDICATOR_WIDTH: f32 = 56.0;
const INDICATOR_HEIGHT: f32 = 32.0;
const INDICATOR_RADIUS: f32 = 16.0;

/// Vertical spacing between items.
const ITEM_GAP: f32 = 12.0;

/// Padding above the first item / below the FAB.
const TOP_PADDING: f32 = 12.0;

/// FAB container size within the rail.
const FAB_SIZE: f32 = 56.0;
const FAB_RADIUS: f32 = 16.0;

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ── Label mode ───────────────────────────────────────────────────────────────

/// Controls how labels are displayed in the navigation rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RailLabelMode {
    /// Show labels on all items (default).
    #[default]
    All,
    /// Show the label only on the selected / active item.
    Selected,
    /// Never show labels (compact mode).
    None,
}

// ── Alignment ────────────────────────────────────────────────────────────────

/// Vertical alignment of the navigation items within the rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RailAlignment {
    /// Items are vertically centered in the available space (default).
    #[default]
    Center,
    /// Items are aligned to the top, directly below the header/FAB.
    Top,
    /// Items are aligned to the bottom of the rail.
    Bottom,
}

// ── Internal item entry ──────────────────────────────────────────────────────

/// Internal descriptor for a single rail destination.
struct RailItemEntry {
    icon: String,
    label: String,
    active: bool,
    on_click: ClickHandler,
    /// Optional badge text (e.g. "3", "99+"). If `Some("")`, shows a dot badge.
    badge: Option<String>,
}

// ── FAB entry ────────────────────────────────────────────────────────────────

/// Internal descriptor for the rail's optional FAB.
struct RailFabEntry {
    icon: String,
    on_click: ClickHandler,
}

// ── Menu icon entry ──────────────────────────────────────────────────────────

/// Internal descriptor for the rail's optional top menu icon.
struct RailMenuEntry {
    icon: String,
    on_click: ClickHandler,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  NavigationRail
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Navigation Rail**.
///
/// A vertical navigation component for tablet and desktop layouts that
/// sits along the leading edge of the screen. It contains 3–7 icon + label
/// destinations, with an optional FAB and menu icon at the top.
///
/// Construct with [`NavigationRail::new`], add items with
/// [`item`](NavigationRail::item), and render by converting to an element.
///
/// # Layout
///
/// ```text
/// ┌──────┐
/// │  ☰   │  ← optional menu icon
/// │      │
/// │  ✏️   │  ← optional FAB
/// │      │
/// │ ┌──┐ │
/// │ │🏠│ │  ← active item with indicator
/// │ └──┘ │
/// │ Home │
/// │      │
/// │  🔍  │  ← inactive item
/// │Search│
/// │      │
/// │  📚  │  ← inactive item
/// │ Lib  │
/// │      │
/// └──────┘
/// ```
pub struct NavigationRail {
    theme: MaterialTheme,
    items: Vec<RailItemEntry>,
    fab: Option<RailFabEntry>,
    menu_icon: Option<RailMenuEntry>,
    header: Option<AnyElement>,
    label_mode: RailLabelMode,
    alignment: RailAlignment,
    /// If true, use the compact width (56 dp) instead of standard (80 dp).
    compact: bool,
}

impl NavigationRail {
    /// Create a new navigation rail with the given theme.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            items: Vec::new(),
            fab: None,
            menu_icon: None,
            header: None,
            label_mode: RailLabelMode::All,
            alignment: RailAlignment::Center,
            compact: false,
        }
    }

    /// Add a navigation destination item.
    ///
    /// # Parameters
    ///
    /// - `icon` — emoji or short icon string displayed in the rail
    /// - `label` — text label displayed below the icon (visibility
    ///   controlled by [`label_mode`](Self::label_mode))
    /// - `active` — whether this item is the currently selected destination
    /// - `on_click` — handler invoked when the item is tapped; compatible
    ///   with `cx.listener(...)`
    pub fn item(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.items.push(RailItemEntry {
            icon: icon.into(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
            badge: None,
        });
        self
    }

    /// Add a navigation destination item with a badge.
    ///
    /// The `badge` parameter controls what is shown:
    /// - `""` (empty string) — shows a small dot indicator
    /// - `"3"`, `"99+"` etc. — shows the text in a badge pill
    pub fn item_with_badge(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        badge: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.items.push(RailItemEntry {
            icon: icon.into(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
            badge: Some(badge.into()),
        });
        self
    }

    /// Set a floating action button at the top of the rail.
    ///
    /// The FAB is rendered above the navigation items.
    pub fn fab(
        mut self,
        icon: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.fab = Some(RailFabEntry {
            icon: icon.into(),
            on_click: Box::new(on_click),
        });
        self
    }

    /// Set a menu icon at the very top of the rail (above the FAB).
    ///
    /// Typically a hamburger menu icon ("☰") used to open a navigation drawer.
    pub fn menu_icon(
        mut self,
        icon: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.menu_icon = Some(RailMenuEntry {
            icon: icon.into(),
            on_click: Box::new(on_click),
        });
        self
    }

    /// Set a custom header element above the destinations.
    ///
    /// This replaces both the menu icon and FAB slots with a custom element.
    pub fn header(mut self, element: impl IntoElement) -> Self {
        self.header = Some(element.into_any_element());
        self
    }

    /// Set the label display mode.
    ///
    /// - [`RailLabelMode::All`] — labels visible on all items (default)
    /// - [`RailLabelMode::Selected`] — label only on the active item
    /// - [`RailLabelMode::None`] — no labels (compact mode)
    pub fn label_mode(mut self, mode: RailLabelMode) -> Self {
        self.label_mode = mode;
        self
    }

    /// Set the vertical alignment of items within the rail.
    pub fn alignment(mut self, alignment: RailAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Use the compact width (56 dp) instead of the standard (80 dp).
    ///
    /// In compact mode, labels are hidden regardless of the label mode setting.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        if compact {
            self.label_mode = RailLabelMode::None;
        }
        self
    }

    /// Build a single rail item element.
    fn build_item(
        theme: &MaterialTheme,
        index: usize,
        entry: RailItemEntry,
        label_mode: RailLabelMode,
    ) -> AnyElement {
        let active_indicator_bg = theme.secondary_container;
        let active_color = theme.on_secondary_container;
        let inactive_color = theme.on_surface_variant;

        let fg = if entry.active {
            active_color
        } else {
            inactive_color
        };

        let show_label = match label_mode {
            RailLabelMode::All => true,
            RailLabelMode::Selected => entry.active,
            RailLabelMode::None => false,
        };

        let mut item_col = div()
            .id(gpui::ElementId::Name(format!("rail-item-{index}").into()))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(ICON_LABEL_GAP))
            .w_full()
            .min_h(px(ITEM_HEIGHT))
            .justify_center()
            .cursor_pointer();

        // Icon container with optional active indicator
        let mut icon_container = div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(INDICATOR_WIDTH))
            .h(px(INDICATOR_HEIGHT))
            .rounded(px(INDICATOR_RADIUS));

        if entry.active {
            icon_container = icon_container.bg(rgb(active_indicator_bg));
        }

        // Icon with optional badge
        let mut icon_wrapper = div()
            .relative()
            .text_xl()
            .text_color(rgb(fg))
            .child(entry.icon);

        // Badge overlay
        if let Some(badge_text) = &entry.badge {
            if badge_text.is_empty() {
                // Dot badge
                icon_wrapper = icon_wrapper.child(
                    div()
                        .absolute()
                        .top(px(-2.0))
                        .right(px(-4.0))
                        .size(px(6.0))
                        .rounded_full()
                        .bg(rgb(theme.error)),
                );
            } else {
                // Text badge
                icon_wrapper = icon_wrapper.child(
                    div()
                        .absolute()
                        .top(px(-4.0))
                        .right(px(-8.0))
                        .min_w(px(16.0))
                        .h(px(16.0))
                        .px(px(4.0))
                        .rounded_full()
                        .bg(rgb(theme.error))
                        .text_color(rgb(theme.on_error))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(px(16.0))
                                .child(badge_text.clone()),
                        ),
                );
            }
        }

        icon_container = icon_container.child(icon_wrapper);
        item_col = item_col.child(icon_container);

        // Label
        if show_label {
            item_col = item_col.child(
                div()
                    .text_size(px(12.0))
                    .line_height(px(16.0))
                    .text_color(rgb(fg))
                    .child(entry.label),
            );
        }

        // Click handler
        item_col = item_col.on_mouse_down(MouseButton::Left, entry.on_click);

        item_col.into_any_element()
    }
}

impl IntoElement for NavigationRail {
    type Element = <gpui::Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let rail_width = if self.compact {
            RAIL_WIDTH_COMPACT
        } else {
            RAIL_WIDTH
        };

        let mut rail = div()
            .flex()
            .flex_col()
            .w(px(rail_width))
            .h_full()
            .bg(rgb(t.surface))
            .items_center()
            .pt(px(TOP_PADDING));

        // ── Header area (menu icon, FAB, or custom header) ───────────

        if let Some(header_el) = self.header {
            rail = rail.child(header_el).child(div().h(px(TOP_PADDING)));
        } else {
            // Menu icon
            if let Some(menu) = self.menu_icon {
                rail = rail.child(
                    div()
                        .id("rail-menu")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(48.0))
                        .rounded(px(ShapeScale::FULL))
                        .cursor_pointer()
                        .text_xl()
                        .text_color(rgb(t.on_surface_variant))
                        .child(menu.icon)
                        .on_mouse_down(MouseButton::Left, menu.on_click),
                );
                rail = rail.child(div().h(px(ITEM_GAP)));
            }

            // FAB
            if let Some(fab_entry) = self.fab {
                rail = rail.child(
                    div()
                        .id("rail-fab")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(FAB_SIZE))
                        .rounded(px(FAB_RADIUS))
                        .bg(rgb(t.primary_container))
                        .text_color(rgb(t.on_primary_container))
                        .text_2xl()
                        .cursor_pointer()
                        .child(fab_entry.icon)
                        .on_mouse_down(MouseButton::Left, fab_entry.on_click),
                );
                rail = rail.child(div().h(px(TOP_PADDING)));
            }
        }

        // ── Items container ──────────────────────────────────────────

        let mut items_container = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(ITEM_GAP))
            .w_full();

        // Alignment: if Center, add flex_1 spacer before and after.
        // If Top, items go at start. If Bottom, add flex_1 spacer before.
        let needs_top_spacer = matches!(
            self.alignment,
            RailAlignment::Center | RailAlignment::Bottom
        );
        let needs_bottom_spacer = matches!(self.alignment, RailAlignment::Center);

        if needs_top_spacer {
            rail = rail.child(div().flex_1());
        }

        for (index, entry) in self.items.into_iter().enumerate() {
            let item_el = Self::build_item(&t, index, entry, self.label_mode);
            items_container = items_container.child(item_el);
        }

        rail = rail.child(items_container);

        if needs_bottom_spacer {
            rail = rail.child(div().flex_1());
        }

        // Bottom padding
        rail = rail.child(div().h(px(TOP_PADDING)));

        rail.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Renders a static (non-interactive) demo of the NavigationRail for
/// component showcase purposes.
///
/// Shows two variants side-by-side: standard with FAB and compact.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::navigation_rail;
///
/// let demo = navigation_rail::navigation_rail_demo(true); // dark mode
/// ```
pub fn navigation_rail_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        // Standard rail with FAB
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("STANDARD WITH FAB"),
                )
                .child(
                    div()
                        .h(px(360.0))
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            NavigationRail::new(theme)
                                .menu_icon("☰", |_, _, _| {})
                                .fab("✏️", |_, _, _| {})
                                .item("🏠", "Home", true, |_, _, _| {})
                                .item_with_badge("🔍", "Search", false, "", |_, _, _| {})
                                .item_with_badge("📚", "Library", false, "3", |_, _, _| {})
                                .item("👤", "Profile", false, |_, _, _| {}),
                        ),
                ),
        )
        // Compact rail
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("COMPACT (NO LABELS)"),
                )
                .child(
                    div()
                        .h(px(300.0))
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            NavigationRail::new(theme)
                                .compact(true)
                                .item("🏠", "Home", true, |_, _, _| {})
                                .item("🔍", "Search", false, |_, _, _| {})
                                .item("📚", "Library", false, |_, _, _| {})
                                .item("👤", "Profile", false, |_, _, _| {}),
                        ),
                ),
        )
        // Selected labels only
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SELECTED LABEL ONLY"),
                )
                .child(
                    div()
                        .h(px(300.0))
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            NavigationRail::new(theme)
                                .label_mode(RailLabelMode::Selected)
                                .item("🏠", "Home", false, |_, _, _| {})
                                .item("🔍", "Search", true, |_, _, _| {})
                                .item("📚", "Library", false, |_, _, _| {})
                                .item("👤", "Profile", false, |_, _, _| {}),
                        ),
                ),
        )
}
