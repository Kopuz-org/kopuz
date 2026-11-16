//! Material navigation bar component — bottom navigation with icon + label items.
//!
//! The navigation bar provides access to primary destinations in an app.
//! Material Design 3 navigation bars feature:
//!
//! - 3–5 icon + label destinations arranged horizontally
//! - An **active indicator** pill behind the selected item's icon
//! - Distinct colours for active and inactive items
//!
//! ## Functional API
//!
//! The primary API uses [`NavigationBarBuilder`] which accepts
//! [`NavigationItem`]s, each carrying its own on-click handler. This
//! design is fully compatible with GPUI's `cx.listener` pattern — each
//! item's handler can be created via `cx.listener(move |this, ...| { ... })`.
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::NavigationBarBuilder;
//!
//! let bar = NavigationBarBuilder::new(dark)
//!     .item("🏠", "Home", current == Screen::Home, cx.listener(move |this, _, _, cx| {
//!         this.navigate_to(Screen::Home);
//!         cx.notify();
//!     }))
//!     .item("⚙️", "Settings", current == Screen::Settings, cx.listener(move |this, _, _, cx| {
//!         this.navigate_to(Screen::Settings);
//!         cx.notify();
//!     }))
//!     .build();
//! ```
//!
//! For showcase / demo purposes, [`navigation_bar_demo`] renders a
//! static (non-interactive) bar with four hardcoded items.

use gpui::{div, prelude::*, px, rgb, AnyElement, MouseButton, MouseDownEvent, Window};

// ── Navigation item descriptor ───────────────────────────────────────────────

/// Describes a single destination in a material navigation bar.
///
/// Each item has an icon, a label, an active flag, and an on-click handler.
/// The handler signature matches what GPUI's `on_mouse_down` expects, so
/// you can use `cx.listener(...)` directly.
#[allow(clippy::type_complexity)]
struct NavigationItemEntry {
    /// Icon text — an emoji or short string displayed above the label.
    icon: String,
    /// Label text — a short description displayed below the icon.
    label: String,
    /// Whether this item is the currently active destination.
    active: bool,
    /// Click handler invoked when the item is tapped.
    on_click: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>,
}

// ── Public NavigationItem for non-interactive use ────────────────────────────

/// A simple descriptor for a navigation bar item (icon + label).
///
/// Used with [`navigation_bar_demo`] and as a convenience type. For the
/// interactive builder API, use [`NavigationBarBuilder::item`] directly.
#[derive(Debug, Clone)]
pub struct NavigationItem {
    /// Icon text — an emoji or short string displayed above the label.
    pub icon: String,
    /// Label text — a short description displayed below the icon.
    pub label: String,
}

impl NavigationItem {
    /// Create a new navigation item with the given icon and label.
    pub fn new(icon: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            icon: icon.into(),
            label: label.into(),
        }
    }
}

// ── Builder ──────────────────────────────────────────────────────────────────

/// Builder for a Material Design 3 bottom navigation bar.
///
/// Construct with [`NavigationBarBuilder::new`], add items with
/// [`item`](NavigationBarBuilder::item), and finalise with
/// [`build`](NavigationBarBuilder::build).
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::NavigationBarBuilder;
///
/// let bar = NavigationBarBuilder::new(true) // dark mode
///     .item("🏠", "Home", true, cx.listener(move |this, _, _, cx| {
///         this.navigate_to(Screen::Home);
///         cx.notify();
///     }))
///     .item("🔢", "Counter", false, cx.listener(move |this, _, _, cx| {
///         this.navigate_to(Screen::Counter);
///         cx.notify();
///     }))
///     .build();
/// ```
pub struct NavigationBarBuilder {
    dark: bool,
    items: Vec<NavigationItemEntry>,
}

impl NavigationBarBuilder {
    /// Create a new navigation bar builder.
    ///
    /// - `dark` — whether to use dark-mode colours (MD3 colour system)
    pub fn new(dark: bool) -> Self {
        Self {
            dark,
            items: Vec::new(),
        }
    }

    /// Add a navigation item with an icon, label, active state, and
    /// click handler.
    ///
    /// The `on_click` parameter accepts anything that satisfies
    /// `Fn(&MouseDownEvent, &mut Window, &mut App) + 'static`, which
    /// is exactly what `cx.listener(...)` returns. You can also pass
    /// a plain closure.
    ///
    /// # Parameters
    ///
    /// - `icon` — emoji or short text displayed as the item's icon
    /// - `label` — text displayed below the icon
    /// - `active` — whether this item is the currently selected destination
    /// - `on_click` — handler invoked when the item is tapped
    pub fn item(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.items.push(NavigationItemEntry {
            icon: icon.into(),
            label: label.into(),
            active,
            on_click: Box::new(on_click),
        });
        self
    }

    /// Build the navigation bar element.
    ///
    /// Consumes the builder and returns an element ready to be added as
    /// a child of a GPUI div.
    pub fn build(self) -> impl IntoElement {
        let dark = self.dark;
        let surface = if dark { 0x211f26_u32 } else { 0xf3edf7 };
        let active_indicator = if dark { 0x4a4458_u32 } else { 0xe8def8 };
        let active_color = if dark { 0xe6e1e5_u32 } else { 0x1c1b1f };
        let inactive_color = if dark { 0x938f99_u32 } else { 0x49454f };

        let mut children: Vec<AnyElement> = Vec::with_capacity(self.items.len());

        for (index, entry) in self.items.into_iter().enumerate() {
            let fg = if entry.active {
                active_color
            } else {
                inactive_color
            };

            let el = div()
                .id(gpui::ElementId::Name(format!("nav-item-{index}").into()))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.0))
                .flex_1()
                .cursor_pointer()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_5()
                        .py(px(4.0))
                        .rounded(px(16.0))
                        .when(entry.active, |d| d.bg(rgb(active_indicator)))
                        .text_xl()
                        .text_color(rgb(fg))
                        .child(entry.icon),
                )
                .child(div().text_xs().text_color(rgb(fg)).child(entry.label))
                .on_mouse_down(MouseButton::Left, entry.on_click);

            children.push(el.into_any_element());
        }

        let mut bar = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .py_3()
            .bg(rgb(surface));

        for child in children {
            bar = bar.child(child);
        }

        bar
    }
}

// ── Static demo bar (for component showcase) ─────────────────────────────────

/// Renders a static (non-interactive) Material Design 3 navigation bar
/// with four hardcoded items for showcase / demo purposes.
///
/// Use [`NavigationBarBuilder`] for the functional version with click handling.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material;
///
/// let demo = material::navigation_bar_demo(true); // dark mode
/// ```
pub fn navigation_bar_demo(dark: bool) -> impl IntoElement {
    NavigationBarBuilder::new(dark)
        .item("🏠", "Home", true, |_, _, _| {})
        .item("🔍", "Explore", false, |_, _, _| {})
        .item("📚", "Library", false, |_, _, _| {})
        .item("👤", "Profile", false, |_, _, _| {})
        .build()
}
