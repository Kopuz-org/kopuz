//! Material Design 3 Navigation Drawer component.
//!
#![allow(unused_imports)]
//! Navigation drawers provide access to destinations and app functionality,
//! such as switching accounts. They can either be permanently on-screen
//! (standard drawer) or controlled by a navigation menu icon (modal drawer).
//!
//! MD3 defines two drawer types:
//!
//! - **Standard** — always visible, sits alongside content (for large screens)
//! - **Modal** — overlays content with a scrim, toggled open/closed
//!
//! # Features
//!
//! - Configurable header area (e.g. account info, branding)
//! - Grouped destination items with optional section headers (dividers)
//! - Active indicator on the selected destination
//! - Badges on destination items (dot or text)
//! - Leading and trailing elements on items (icons, counts)
//!
//! # Examples
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::navigation_drawer::*;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Standard drawer
//! let drawer = NavigationDrawer::new(theme)
//!     .header(my_header_element)
//!     .section("Mail")
//!     .item("📥", "Inbox", true, cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .item_with_badge("📤", "Outbox", false, "3", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .section("Labels")
//!     .item("⭐", "Starred", false, cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .item("🗑️", "Trash", false, cx.listener(|this, _, _, cx| { /* ... */ }));
//!
//! // Modal drawer (same API, wrapped in a modal overlay by the Scaffold)
//! let modal = ModalNavigationDrawer::new(theme)
//!     .drawer(drawer)
//!     .open(is_drawer_open)
//!     .on_scrim_tap(cx.listener(|this, _, _, cx| {
//!         this.close_drawer();
//!         cx.notify();
//!     }));
//! ```

use gpui::{div, prelude::*, px, rgb, AnyElement, MouseButton, MouseDownEvent, Stateful, Window};

use super::theme::{MaterialTheme, ShapeScale};

// ── Constants ────────────────────────────────────────────────────────────────

/// Standard drawer width (360 dp per MD3 spec).
const DRAWER_WIDTH: f32 = 360.0;

/// Minimum drawer width for compact layouts.
const DRAWER_MIN_WIDTH: f32 = 256.0;

/// Height of each drawer destination item.
const ITEM_HEIGHT: f32 = 56.0;

/// Horizontal padding inside the drawer.
const DRAWER_PADDING_H: f32 = 12.0;

/// Vertical padding between sections.
const SECTION_GAP: f32 = 16.0;

/// Active indicator corner radius.
const INDICATOR_RADIUS: f32 = 28.0;

/// Item internal horizontal padding.
const ITEM_PADDING_H: f32 = 16.0;

/// Gap between the leading icon and the label.
const ICON_LABEL_GAP: f32 = 12.0;

/// Section header (divider label) height.
const SECTION_HEADER_HEIGHT: f32 = 36.0;

/// Scrim overlay opacity (modal drawer).
const SCRIM_OPACITY: f32 = 0.32;

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ── Internal entry types ─────────────────────────────────────────────────────

/// Internal enum representing a single element in the drawer's item list.
enum DrawerEntry {
    /// A navigable destination item.
    Item(DrawerItemEntry),
    /// A section divider with an optional label.
    Section(Option<String>),
}

/// A single drawer destination item.
struct DrawerItemEntry {
    /// Leading icon — emoji or short string.
    icon: String,
    /// Label text.
    label: String,
    /// Whether this item is the currently active destination.
    active: bool,
    /// Click handler.
    on_click: ClickHandler,
    /// Optional badge text. Empty string = dot badge, non-empty = text badge.
    badge: Option<String>,
    /// Optional trailing element text (e.g. a count like "24").
    trailing: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  NavigationDrawer
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Navigation Drawer**.
///
/// The drawer is a vertical panel containing destination items grouped
/// into optional sections. Each item has a leading icon, label, and
/// optional badge or trailing text.
///
/// The drawer itself does not manage open/closed state — that is the
/// responsibility of the parent. For modal behavior, wrap the drawer
/// in a [`ModalNavigationDrawer`].
///
/// # Layout
///
/// ```text
/// ╭──────────────────────────────╮
/// │        [Header Area]         │
/// │──────────────────────────────│
/// │ ┌──────────────────────────┐ │
/// │ │ 📥  Inbox            24  │ │  ← active item with trailing count
/// │ └──────────────────────────┘ │
/// │   📤  Outbox           ●3   │  ← with badge
/// │   📑  Drafts                │
/// │──────── Labels ─────────────│  ← section divider
/// │   ⭐  Starred               │
/// │   🗑️  Trash                 │
/// ╰──────────────────────────────╯
/// ```
pub struct NavigationDrawer {
    theme: MaterialTheme,
    entries: Vec<DrawerEntry>,
    header: Option<AnyElement>,
    width: f32,
    /// Internal item index counter for generating unique element IDs.
    item_count: usize,
}

impl NavigationDrawer {
    /// Create a new navigation drawer with the given theme.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            entries: Vec::new(),
            header: None,
            width: DRAWER_WIDTH,
            item_count: 0,
        }
    }

    /// Set a custom header element at the top of the drawer.
    ///
    /// Typically used for account switchers, branding, or user info.
    pub fn header(mut self, element: impl IntoElement) -> Self {
        self.header = Some(element.into_any_element());
        self
    }

    /// Add a section divider with an optional label.
    ///
    /// Sections create visual separation between groups of destinations.
    /// Pass an empty string or use [`divider`](Self::divider) for a
    /// label-less divider line.
    pub fn section(mut self, label: impl Into<String>) -> Self {
        let label_str = label.into();
        self.entries
            .push(DrawerEntry::Section(if label_str.is_empty() {
                None
            } else {
                Some(label_str)
            }));
        self
    }

    /// Add a plain divider (no label).
    pub fn divider(mut self) -> Self {
        self.entries.push(DrawerEntry::Section(None));
        self
    }

    /// Add a destination item with an icon, label, active state, and click handler.
    ///
    /// The `on_click` handler matches GPUI's `on_mouse_down` signature,
    /// so `cx.listener(...)` can be used directly.
    pub fn item(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.item_count += 1;
        self.entries.push(DrawerEntry::Item(DrawerItemEntry {
            icon: icon.into(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
            badge: None,
            trailing: None,
        }));
        self
    }

    /// Add a destination item with a badge.
    ///
    /// - `badge` = `""` → dot badge
    /// - `badge` = `"3"`, `"99+"` etc. → text badge pill
    pub fn item_with_badge(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        badge: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.item_count += 1;
        self.entries.push(DrawerEntry::Item(DrawerItemEntry {
            icon: icon.into(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
            badge: Some(badge.into()),
            trailing: None,
        }));
        self
    }

    /// Add a destination item with trailing text (e.g. an unread count).
    pub fn item_with_trailing(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        trailing: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.item_count += 1;
        self.entries.push(DrawerEntry::Item(DrawerItemEntry {
            icon: icon.into(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
            badge: None,
            trailing: Some(trailing.into()),
        }));
        self
    }

    /// Override the drawer width (default is 360 dp).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(DRAWER_MIN_WIDTH);
        self
    }

    // ── Internal builder helpers ──────────────────────────────────────

    /// Build a single drawer item element.
    fn build_item(theme: &MaterialTheme, index: usize, entry: DrawerItemEntry) -> AnyElement {
        let active_bg = theme.secondary_container;
        let active_fg = theme.on_secondary_container;
        let inactive_fg = theme.on_surface_variant;

        let (bg, fg) = if entry.active {
            (Some(active_bg), active_fg)
        } else {
            (None, inactive_fg)
        };

        let mut item_row = div()
            .id(gpui::ElementId::Name(format!("drawer-item-{index}").into()))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ICON_LABEL_GAP))
            .w_full()
            .h(px(ITEM_HEIGHT))
            .px(px(ITEM_PADDING_H))
            .rounded(px(INDICATOR_RADIUS))
            .cursor_pointer();

        // Background (active indicator)
        if let Some(bg_color) = bg {
            item_row = item_row.bg(rgb(bg_color));
        }

        // Leading icon
        item_row = item_row.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.0))
                .text_xl()
                .text_color(rgb(fg))
                .child(entry.icon),
        );

        // Label (takes remaining space)
        item_row = item_row.child(
            div()
                .flex_1()
                .text_sm()
                .text_color(rgb(if entry.active {
                    active_fg
                } else {
                    theme.on_surface
                }))
                .child(entry.label),
        );

        // Badge (if present)
        if let Some(badge_text) = entry.badge {
            if badge_text.is_empty() {
                // Dot badge
                item_row = item_row.child(div().size(px(6.0)).rounded_full().bg(rgb(theme.error)));
            } else {
                // Text badge pill
                item_row = item_row.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .min_w(px(22.0))
                        .h(px(22.0))
                        .px(px(6.0))
                        .rounded_full()
                        .bg(rgb(theme.error))
                        .text_color(rgb(theme.on_error))
                        .text_size(px(11.0))
                        .line_height(px(16.0))
                        .child(badge_text),
                );
            }
        }

        // Trailing text (if present)
        if let Some(trailing_text) = entry.trailing {
            item_row = item_row.child(
                div()
                    .text_sm()
                    .text_color(rgb(theme.on_surface_variant))
                    .child(trailing_text),
            );
        }

        // Click handler
        item_row = item_row.on_mouse_down(MouseButton::Left, entry.on_click);

        item_row.into_any_element()
    }

    /// Build a section divider element.
    fn build_section(theme: &MaterialTheme, label: Option<String>) -> AnyElement {
        let mut section = div().flex().flex_col().w_full().py(px(SECTION_GAP / 2.0));

        // Divider line
        section = section.child(div().w_full().h(px(1.0)).bg(rgb(theme.outline_variant)));

        // Optional section label
        if let Some(label_text) = label {
            section = section.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(px(SECTION_HEADER_HEIGHT))
                    .px(px(ITEM_PADDING_H))
                    .text_size(px(14.0))
                    .line_height(px(20.0))
                    .text_color(rgb(theme.on_surface_variant))
                    .child(label_text),
            );
        }

        section.into_any_element()
    }
}

impl IntoElement for NavigationDrawer {
    type Element = <gpui::Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let mut drawer = div()
            .id("navigation-drawer")
            .flex()
            .flex_col()
            .w(px(self.width))
            .h_full()
            .bg(rgb(t.surface_container_low))
            .rounded_tr(px(ShapeScale::LARGE))
            .rounded_br(px(ShapeScale::LARGE))
            .overflow_hidden();

        // ── Header ───────────────────────────────────────────────────
        if let Some(header_el) = self.header {
            drawer = drawer.child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .px(px(DRAWER_PADDING_H))
                    .pt(px(16.0))
                    .pb(px(8.0))
                    .child(header_el),
            );
        }

        // ── Items area (scrollable) ──────────────────────────────────
        let mut items_container = div()
            .id("drawer-items-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .px(px(DRAWER_PADDING_H))
            .gap(px(2.0))
            .overflow_y_scroll();

        let mut item_index: usize = 0;
        for entry in self.entries {
            match entry {
                DrawerEntry::Item(item_entry) => {
                    let el = Self::build_item(&t, item_index, item_entry);
                    items_container = items_container.child(el);
                    item_index += 1;
                }
                DrawerEntry::Section(label) => {
                    let el = Self::build_section(&t, label);
                    items_container = items_container.child(el);
                }
            }
        }

        drawer = drawer.child(items_container);

        // Bottom padding
        drawer = drawer.child(div().h(px(DRAWER_PADDING_H)));

        drawer.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ModalNavigationDrawer
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Modal Navigation Drawer**.
///
/// Wraps a [`NavigationDrawer`] in a full-screen overlay with a scrim
/// (semi-transparent backdrop). When `open` is `true`, the drawer is
/// visible and the scrim is rendered; when `false`, nothing is rendered.
///
/// The caller is responsible for managing the open/closed state and
/// providing an `on_scrim_tap` handler to close the drawer.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::navigation_drawer::*;
/// use gpui_mobile::components::material::theme::MaterialTheme;
///
/// let theme = MaterialTheme::dark();
///
/// ModalNavigationDrawer::new(theme)
///     .drawer(
///         NavigationDrawer::new(theme)
///             .item("📥", "Inbox", true, |_, _, _| {})
///             .item("📤", "Outbox", false, |_, _, _| {}),
///     )
///     .open(self.drawer_open)
///     .on_scrim_tap(cx.listener(|this, _, _, cx| {
///         this.drawer_open = false;
///         cx.notify();
///     }))
/// ```
#[allow(dead_code)]
pub struct ModalNavigationDrawer {
    theme: MaterialTheme,
    drawer: Option<NavigationDrawer>,
    open: bool,
    on_scrim_tap: Option<ClickHandler>,
}

impl ModalNavigationDrawer {
    /// Create a new modal drawer wrapper.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            drawer: None,
            open: false,
            on_scrim_tap: None,
        }
    }

    /// Set the inner navigation drawer.
    pub fn drawer(mut self, drawer: NavigationDrawer) -> Self {
        self.drawer = Some(drawer);
        self
    }

    /// Set whether the modal drawer is currently open.
    ///
    /// When `false`, the modal renders nothing (zero-size).
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Set the handler invoked when the scrim (backdrop) is tapped.
    ///
    /// This should close the drawer by updating the parent's state.
    pub fn on_scrim_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_scrim_tap = Some(Box::new(handler));
        self
    }
}

impl IntoElement for ModalNavigationDrawer {
    type Element = <gpui::Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        if !self.open {
            // Render nothing when closed
            return div().id("modal-drawer-empty").into_element();
        }

        let mut overlay = div()
            .id("modal-drawer-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full();

        // Scrim (semi-transparent backdrop)
        let mut scrim = div()
            .id("drawer-scrim")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(gpui::hsla(0.0, 0.0, 0.0, SCRIM_OPACITY))
            .cursor_pointer();

        if let Some(handler) = self.on_scrim_tap {
            scrim = scrim.on_mouse_down(MouseButton::Left, handler);
        }

        overlay = overlay.child(scrim);

        // Drawer (slides in from the left)
        if let Some(drawer) = self.drawer {
            overlay = overlay.child(div().absolute().top_0().left_0().h_full().child(drawer));
        }

        overlay.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Renders a static (non-interactive) demo of the NavigationDrawer for
/// component showcase purposes.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::navigation_drawer;
///
/// let demo = navigation_drawer::navigation_drawer_demo(true); // dark mode
/// ```
pub fn navigation_drawer_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        // Standard drawer demo
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("STANDARD DRAWER"),
                )
                .child(
                    div()
                        .h(px(400.0))
                        .w(px(320.0))
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            NavigationDrawer::new(theme)
                                .width(320.0)
                                .header(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(22.0))
                                                .text_color(rgb(theme.on_surface))
                                                .child("Mail"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(theme.on_surface_variant))
                                                .child("user@example.com"),
                                        ),
                                )
                                .item_with_trailing("📥", "Inbox", true, "24", |_, _, _| {})
                                .item_with_badge("📤", "Outbox", false, "3", |_, _, _| {})
                                .item("📑", "Drafts", false, |_, _, _| {})
                                .item_with_badge("🔔", "Updates", false, "", |_, _, _| {})
                                .section("Labels")
                                .item("⭐", "Starred", false, |_, _, _| {})
                                .item("⏰", "Snoozed", false, |_, _, _| {})
                                .item("📁", "Important", false, |_, _, _| {})
                                .item("📨", "Sent", false, |_, _, _| {})
                                .section("")
                                .item("🗑️", "Trash", false, |_, _, _| {})
                                .item("⚠️", "Spam", false, |_, _, _| {}),
                        ),
                ),
        )
        // Info text
        .child(
            div()
                .text_xs()
                .text_color(rgb(theme.on_surface_variant))
                .child(
                    "Navigation drawers can also be displayed as modal overlays \
                     using ModalNavigationDrawer with a scrim backdrop.",
                ),
        )
}
