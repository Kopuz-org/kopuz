#![allow(unused_imports)]
//! Material Design 3 Tab Bar component.
//!
//! Tabs organize content across different screens, data sets, and other
//! interactions. MD3 defines two tab variants:
//!
//! - **Primary tabs** — high-emphasis tabs placed at the top of the content
//!   area, with an underline active indicator and optional icons
//! - **Secondary tabs** — used within a content area for sub-navigation,
//!   with a simpler underline indicator (no icons)
//!
//! # Features
//!
//! - Configurable tab items with labels and optional icons
//! - Active underline indicator with customizable color
//! - Primary and secondary visual variants
//! - Scrollable mode for many tabs (horizontal scroll)
//! - Fixed mode where all tabs share equal width
//! - Badge support on tab items (dot or text)
//!
//! # Examples
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::tab_bar::*;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Primary tabs with icons
//! let tabs = TabBar::primary(theme)
//!     .tab("✈️", "Flights", true, cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .tab("🏨", "Hotels", false, cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .tab("🚗", "Cars", false, cx.listener(|this, _, _, cx| { /* ... */ }));
//!
//! // Secondary tabs (no icons)
//! let tabs = TabBar::secondary(theme)
//!     .text_tab("All", true, cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .text_tab("Unread", false, cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .text_tab("Starred", false, cx.listener(|this, _, _, cx| { /* ... */ }));
//!
//! // Scrollable tabs
//! let tabs = TabBar::primary(theme)
//!     .scrollable(true)
//!     .tab("", "Tab 1", true, |_, _, _| {})
//!     .tab("", "Tab 2", false, |_, _, _| {})
//!     .tab("", "Tab 3", false, |_, _, _| {})
//!     .tab("", "Tab 4", false, |_, _, _| {})
//!     .tab("", "Tab 5", false, |_, _, _| {});
//! ```

use gpui::{div, prelude::*, px, rgb, AnyElement, MouseButton, MouseDownEvent, Stateful, Window};

use super::theme::MaterialTheme;

// ── Constants ────────────────────────────────────────────────────────────────

/// Height of primary tabs (with icons: 64dp, without: 48dp).
const PRIMARY_TAB_HEIGHT_WITH_ICON: f32 = 64.0;
const PRIMARY_TAB_HEIGHT_TEXT_ONLY: f32 = 48.0;

/// Height of secondary tabs (always 48dp).
const SECONDARY_TAB_HEIGHT: f32 = 48.0;

/// Minimum tab width in fixed mode (90dp per MD3 spec).
const MIN_TAB_WIDTH: f32 = 90.0;

/// Horizontal padding within a scrollable tab.
const TAB_PADDING_H: f32 = 16.0;

/// Active indicator (underline) height.
const INDICATOR_HEIGHT: f32 = 3.0;

/// Active indicator corner radius (top corners only for primary tabs).
const INDICATOR_RADIUS: f32 = 3.0;

/// Gap between icon and label in primary tabs.
const ICON_LABEL_GAP: f32 = 2.0;

/// Badge dot size.
const BADGE_DOT_SIZE: f32 = 6.0;

/// Badge pill height.
const BADGE_PILL_HEIGHT: f32 = 16.0;

/// Badge pill minimum width.
const BADGE_PILL_MIN_WIDTH: f32 = 16.0;

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ── Tab variant ──────────────────────────────────────────────────────────────

/// The visual variant of a tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarVariant {
    /// Primary tabs — high emphasis, support icons, active indicator is
    /// a rounded underline in the primary color.
    Primary,
    /// Secondary tabs — lower emphasis, text-only, active indicator is
    /// a simple underline in the primary color.
    Secondary,
}

// ── Internal tab entry ───────────────────────────────────────────────────────

/// Internal descriptor for a single tab.
struct TabEntry {
    /// Optional icon text — emoji or short string. Empty string = no icon.
    icon: String,
    /// Label text.
    label: String,
    /// Whether this tab is currently selected/active.
    active: bool,
    /// Click handler invoked when the tab is tapped.
    on_click: ClickHandler,
    /// Optional badge text. `Some("")` = dot badge, `Some("3")` = text badge.
    badge: Option<String>,
    /// Whether this tab is disabled (non-interactive, muted colors).
    disabled: bool,
}

// ═══════════════════════════════════════════════════════════════════════════════
//  TabBar
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Tab Bar**.
///
/// Organizes content across different screens or data sets. Supports
/// primary (icon + label) and secondary (label-only) variants.
///
/// In **fixed** mode (default), all tabs share equal width and fill the
/// available horizontal space. In **scrollable** mode, each tab sizes to
/// its content and the bar scrolls horizontally.
///
/// # Layout (Primary, fixed)
///
/// ```text
/// ┌─────────────┬─────────────┬─────────────┐
/// │     ✈️       │     🏨       │     🚗       │
/// │   Flights   │   Hotels    │    Cars     │
/// │ ━━━━━━━━━━━ │             │             │  ← active indicator
/// └─────────────┴─────────────┴─────────────┘
/// ```
///
/// # Layout (Secondary, fixed)
///
/// ```text
/// ┌──────────┬──────────┬──────────┐
/// │   All    │  Unread  │ Starred  │
/// │ ━━━━━━━━ │          │          │  ← active indicator
/// └──────────┴──────────┴──────────┘
/// ```
pub struct TabBar {
    theme: MaterialTheme,
    variant: TabBarVariant,
    tabs: Vec<TabEntry>,
    /// Whether the tab bar is scrollable (false = fixed equal-width tabs).
    scrollable: bool,
    /// Whether to show a divider line below the tab bar.
    show_divider: bool,
}

impl TabBar {
    // ── Constructors ─────────────────────────────────────────────────

    /// Create a **primary** tab bar.
    ///
    /// Primary tabs support both icons and labels, and use a rounded
    /// underline indicator for the active tab.
    pub fn primary(theme: MaterialTheme) -> Self {
        Self {
            theme,
            variant: TabBarVariant::Primary,
            tabs: Vec::new(),
            scrollable: false,
            show_divider: true,
        }
    }

    /// Create a **secondary** tab bar.
    ///
    /// Secondary tabs are text-only and use a simple underline indicator.
    pub fn secondary(theme: MaterialTheme) -> Self {
        Self {
            theme,
            variant: TabBarVariant::Secondary,
            tabs: Vec::new(),
            scrollable: false,
            show_divider: true,
        }
    }

    // ── Tab items ────────────────────────────────────────────────────

    /// Add a tab with an icon and label.
    ///
    /// For secondary tabs, the icon is ignored. Use [`text_tab`](Self::text_tab)
    /// for clarity when adding text-only tabs.
    ///
    /// # Parameters
    ///
    /// - `icon` — emoji or short icon string (pass `""` for no icon)
    /// - `label` — tab text
    /// - `active` — whether this tab is currently selected
    /// - `on_click` — handler invoked when the tab is tapped
    pub fn tab(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.tabs.push(TabEntry {
            icon: icon.into(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
            badge: None,
            disabled: false,
        });
        self
    }

    /// Add a text-only tab (no icon).
    ///
    /// Convenience method equivalent to `.tab("", label, active, on_click)`.
    pub fn text_tab(
        mut self,
        label: impl Into<String>,
        active: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.tabs.push(TabEntry {
            icon: String::new(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
            badge: None,
            disabled: false,
        });
        self
    }

    /// Add a tab with a badge.
    ///
    /// - `badge` = `""` → dot badge
    /// - `badge` = `"3"` → text badge pill
    pub fn tab_with_badge(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        badge: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.tabs.push(TabEntry {
            icon: icon.into(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
            badge: Some(badge.into()),
            disabled: false,
        });
        self
    }

    /// Add a disabled (non-interactive) tab.
    ///
    /// Disabled tabs are rendered with reduced opacity and do not respond
    /// to clicks.
    pub fn disabled_tab(mut self, icon: impl Into<String>, label: impl Into<String>) -> Self {
        self.tabs.push(TabEntry {
            icon: icon.into(),
            label: label.into(),
            active: false,
            on_click: Box::new(|_, _, _| {}),
            badge: None,
            disabled: true,
        });
        self
    }

    // ── Configuration ────────────────────────────────────────────────

    /// Set whether the tab bar is scrollable.
    ///
    /// When `true`, tabs size to their content and the bar scrolls
    /// horizontally. When `false` (default), tabs share equal width.
    pub fn scrollable(mut self, scrollable: bool) -> Self {
        self.scrollable = scrollable;
        self
    }

    /// Set whether to show a divider line below the tab bar.
    ///
    /// Defaults to `true`. Set to `false` if the tab bar sits directly
    /// above content with its own top border.
    pub fn show_divider(mut self, show: bool) -> Self {
        self.show_divider = show;
        self
    }

    // ── Internal helpers ─────────────────────────────────────────────

    /// Determine whether any tab has an icon (affects primary tab height).
    fn has_any_icon(&self) -> bool {
        self.variant == TabBarVariant::Primary && self.tabs.iter().any(|t| !t.icon.is_empty())
    }

    /// Build a single primary tab element.
    fn build_primary_tab(
        theme: &MaterialTheme,
        index: usize,
        entry: TabEntry,
        has_icons: bool,
        fixed: bool,
    ) -> AnyElement {
        let active_fg = theme.primary;
        let inactive_fg = theme.on_surface_variant;
        let fg = if entry.disabled {
            theme.on_surface
        } else if entry.active {
            active_fg
        } else {
            inactive_fg
        };

        let tab_height = if has_icons {
            PRIMARY_TAB_HEIGHT_WITH_ICON
        } else {
            PRIMARY_TAB_HEIGHT_TEXT_ONLY
        };

        let mut tab = div()
            .id(gpui::ElementId::Name(format!("tab-{index}").into()))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .h(px(tab_height))
            .cursor_pointer()
            .relative();

        if fixed {
            tab = tab.flex_1().min_w(px(MIN_TAB_WIDTH));
        } else {
            tab = tab.px(px(TAB_PADDING_H));
        }

        // Disabled opacity
        if entry.disabled {
            tab = tab.opacity(0.38);
        }

        // Content column: icon (optional) + label
        let mut content = div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(ICON_LABEL_GAP));

        // Icon with optional badge
        if !entry.icon.is_empty() {
            let mut icon_wrapper = div()
                .relative()
                .text_xl()
                .text_color(rgb(fg))
                .child(entry.icon.clone());

            // Badge overlay on icon
            if let Some(ref badge_text) = entry.badge {
                icon_wrapper = Self::apply_badge(icon_wrapper, theme, badge_text);
            }

            content = content.child(icon_wrapper);
        } else if let Some(ref badge_text) = entry.badge {
            // Badge next to label when there is no icon (rendered later in label row)
            // We'll handle this as a relative wrapper on the label.
            let _ = badge_text;
        }

        // Label
        let mut label_wrapper = div()
            .relative()
            .text_sm()
            .line_height(px(20.0))
            .text_color(rgb(fg));

        // If there is no icon and a badge exists, attach badge to label
        if entry.icon.is_empty() {
            if let Some(ref badge_text) = entry.badge {
                label_wrapper = Self::apply_badge(label_wrapper, theme, badge_text);
            }
        }

        label_wrapper = label_wrapper.child(entry.label);
        content = content.child(label_wrapper);

        tab = tab.child(content);

        // Active indicator (underline at the bottom)
        if entry.active && !entry.disabled {
            tab = tab.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left(px(if fixed { 0.0 } else { TAB_PADDING_H / 2.0 }))
                    .right(px(if fixed { 0.0 } else { TAB_PADDING_H / 2.0 }))
                    .h(px(INDICATOR_HEIGHT))
                    .rounded_tl(px(INDICATOR_RADIUS))
                    .rounded_tr(px(INDICATOR_RADIUS))
                    .bg(rgb(active_fg)),
            );
        }

        // Click handler
        if !entry.disabled {
            tab = tab.on_mouse_down(MouseButton::Left, entry.on_click);
        }

        tab.into_any_element()
    }

    /// Build a single secondary tab element.
    fn build_secondary_tab(
        theme: &MaterialTheme,
        index: usize,
        entry: TabEntry,
        fixed: bool,
    ) -> AnyElement {
        let active_fg = theme.on_surface;
        let inactive_fg = theme.on_surface_variant;
        let fg = if entry.disabled {
            theme.on_surface
        } else if entry.active {
            active_fg
        } else {
            inactive_fg
        };

        let mut tab = div()
            .id(gpui::ElementId::Name(format!("sec-tab-{index}").into()))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .h(px(SECONDARY_TAB_HEIGHT))
            .cursor_pointer()
            .relative();

        if fixed {
            tab = tab.flex_1().min_w(px(MIN_TAB_WIDTH));
        } else {
            tab = tab.px(px(TAB_PADDING_H));
        }

        if entry.disabled {
            tab = tab.opacity(0.38);
        }

        // Label with optional badge
        let mut label_wrapper = div()
            .relative()
            .text_sm()
            .line_height(px(20.0))
            .text_color(rgb(fg));

        if let Some(ref badge_text) = entry.badge {
            label_wrapper = Self::apply_badge(label_wrapper, theme, badge_text);
        }

        label_wrapper = label_wrapper.child(entry.label);
        tab = tab.child(label_wrapper);

        // Active indicator (simple full-width underline)
        if entry.active && !entry.disabled {
            tab = tab.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(2.0))
                    .bg(rgb(theme.primary)),
            );
        }

        if !entry.disabled {
            tab = tab.on_mouse_down(MouseButton::Left, entry.on_click);
        }

        tab.into_any_element()
    }

    /// Apply a badge overlay to a parent div (either a dot or text pill).
    fn apply_badge(parent: gpui::Div, theme: &MaterialTheme, badge_text: &str) -> gpui::Div {
        if badge_text.is_empty() {
            // Dot badge
            parent.child(
                div()
                    .absolute()
                    .top(px(-2.0))
                    .right(px(-4.0))
                    .size(px(BADGE_DOT_SIZE))
                    .rounded_full()
                    .bg(rgb(theme.error)),
            )
        } else {
            // Text badge pill
            parent.child(
                div()
                    .absolute()
                    .top(px(-4.0))
                    .right(px(-10.0))
                    .min_w(px(BADGE_PILL_MIN_WIDTH))
                    .h(px(BADGE_PILL_HEIGHT))
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
                            .child(badge_text.to_string()),
                    ),
            )
        }
    }
}

impl IntoElement for TabBar {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let is_fixed = !self.scrollable;
        let has_icons = self.has_any_icon();
        let is_primary = self.variant == TabBarVariant::Primary;

        let mut container = div()
            .id("tab-bar-container")
            .flex()
            .flex_col()
            .w_full()
            .bg(rgb(t.surface));

        // Tab row — always call .id() upfront so the type is consistently Stateful<Div>
        let mut tab_row = div().id("tab-bar-row").flex().flex_row().w_full();

        if self.scrollable {
            // In scrollable mode, enable horizontal scrolling
            tab_row = tab_row.overflow_x_scroll();
        }

        for (index, entry) in self.tabs.into_iter().enumerate() {
            let tab_el = if is_primary {
                Self::build_primary_tab(&t, index, entry, has_icons, is_fixed)
            } else {
                Self::build_secondary_tab(&t, index, entry, is_fixed)
            };
            tab_row = tab_row.child(tab_el);
        }

        container = container.child(tab_row);

        // Divider line below the tabs
        if self.show_divider {
            container = container.child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .bg(rgb(t.surface_container_highest)),
            );
        }

        container.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Renders a comprehensive showcase of all MD3 tab bar variants.
///
/// Shows primary tabs (with and without icons), secondary tabs,
/// scrollable tabs, and tabs with badges.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::tab_bar;
///
/// let demo = tab_bar::tab_bar_demo(true); // dark mode
/// ```
pub fn tab_bar_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        // Primary tabs with icons
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("PRIMARY TABS (WITH ICONS)"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TabBar::primary(theme)
                                .tab("✈️", "Flights", true, |_, _, _| {})
                                .tab("🏨", "Hotels", false, |_, _, _| {})
                                .tab("🚗", "Cars", false, |_, _, _| {}),
                        ),
                ),
        )
        // Primary tabs without icons
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("PRIMARY TABS (TEXT ONLY)"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TabBar::primary(theme)
                                .text_tab("All", true, |_, _, _| {})
                                .text_tab("Unread", false, |_, _, _| {})
                                .text_tab("Starred", false, |_, _, _| {})
                                .text_tab("Archived", false, |_, _, _| {}),
                        ),
                ),
        )
        // Secondary tabs
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SECONDARY TABS"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TabBar::secondary(theme)
                                .text_tab("For You", false, |_, _, _| {})
                                .text_tab("Following", true, |_, _, _| {})
                                .text_tab("Trending", false, |_, _, _| {}),
                        ),
                ),
        )
        // Tabs with badges
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("TABS WITH BADGES"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TabBar::primary(theme)
                                .tab_with_badge("📥", "Inbox", true, "12", |_, _, _| {})
                                .tab_with_badge("💬", "Chat", false, "", |_, _, _| {})
                                .tab("📞", "Calls", false, |_, _, _| {})
                                .disabled_tab("🎵", "Music"),
                        ),
                ),
        )
        // Scrollable tabs
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SCROLLABLE TABS"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TabBar::primary(theme)
                                .scrollable(true)
                                .text_tab("Popular", true, |_, _, _| {})
                                .text_tab("Recent", false, |_, _, _| {})
                                .text_tab("Recommended", false, |_, _, _| {})
                                .text_tab("Trending", false, |_, _, _| {})
                                .text_tab("New Releases", false, |_, _, _| {})
                                .text_tab("Coming Soon", false, |_, _, _| {})
                                .text_tab("Top Rated", false, |_, _, _| {}),
                        ),
                ),
        )
}
