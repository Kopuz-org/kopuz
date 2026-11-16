//! Material Design 3 Menu components.
//!
//! Menus display a list of choices on a temporary surface. They appear
//! when users interact with a button, action, or other control. MD3
//! defines two menu patterns:
//!
//! - **Dropdown menu** — attached to an anchor element (button, icon),
//!   positioned below or beside it
//! - **Context menu** — appears at the touch/click position
//!
//! Both variants share the same visual style and item structure.
//!
//! ## Architecture
//!
//! All menu structs use a **builder pattern** and implement `IntoElement`,
//! making them composable with GPUI's standard `.child(...)` API. Click
//! handlers use GPUI's `on_mouse_down` signature, so `cx.listener(...)`
//! works directly.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::menu::{Menu, MenuDivider};
//! use gpui_mobile::components::material::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Basic dropdown menu
//! let menu = Menu::new(theme)
//!     .item("Cut", "✂️", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .item("Copy", "📋", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .item("Paste", "📄", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .divider()
//!     .item("Select All", "", cx.listener(|this, _, _, cx| { /* ... */ }));
//!
//! // Menu with disabled and checked items
//! let menu = Menu::new(theme)
//!     .item_checked("Bold", "B", true, cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .item_checked("Italic", "I", false, cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .divider()
//!     .item_disabled("Strikethrough", "S");
//!
//! // Cascading / sub-menu (visual only — expand behaviour is consumer-managed)
//! let menu = Menu::new(theme)
//!     .item("Open", "📂", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .item("Open Recent", "▸", cx.listener(|this, _, _, cx| { /* show sub-menu */ }))
//!     .divider()
//!     .item("Settings", "⚙️", cx.listener(|this, _, _, cx| { /* ... */ }));
//! ```
//!
//! ## MD3 Specification Reference
//!
//! - Container: surface-container, medium shape (12dp radius), elevation level 2
//! - Min width: 112dp, max width: 280dp
//! - Item height: 48dp
//! - Item padding: 12dp vertical, 16dp horizontal (trailing 24dp for shortcut)
//! - Leading icon: on-surface-variant, 24dp
//! - Label: body-large, on-surface
//! - Trailing text/icon: body-large, on-surface-variant
//! - Divider: outline-variant, 1dp height, 0dp horizontal margin
//! - Disabled items: 38% opacity

use gpui::{div, prelude::*, px, AnyElement, Hsla, MouseButton, MouseDownEvent, Stateful, Window};

use super::theme::{color, MaterialTheme, ShapeScale, TRANSPARENT};

// ── Constants ────────────────────────────────────────────────────────────────

/// Menu container corner radius (MD3: medium = 12dp).
const MENU_RADIUS: f32 = ShapeScale::MEDIUM;

/// Minimum menu width in dp (MD3 spec: 112dp).
const MIN_MENU_WIDTH: f32 = 112.0;

/// Maximum menu width in dp (MD3 spec: 280dp).
const MAX_MENU_WIDTH: f32 = 280.0;

/// Menu item height in dp (MD3 spec: 48dp).
const ITEM_HEIGHT: f32 = 48.0;

/// Menu vertical padding (top/bottom of the list).
const MENU_PADDING_V: f32 = 8.0;

/// Menu item horizontal padding (leading).
const ITEM_PADDING_H: f32 = 16.0;

/// Trailing area padding (for shortcut/icon).
const ITEM_TRAILING_PADDING: f32 = 24.0;

/// Gap between leading icon and label.
const ICON_LABEL_GAP: f32 = 12.0;

/// Divider height.
const DIVIDER_HEIGHT: f32 = 1.0;

/// Divider vertical margin.
const DIVIDER_MARGIN_V: f32 = 8.0;

/// Disabled content opacity (MD3 spec: 0.38).
const DISABLED_OPACITY: f32 = 0.38;

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ═══════════════════════════════════════════════════════════════════════════════
//  Menu Entry (internal)
// ═══════════════════════════════════════════════════════════════════════════════

/// Internal representation of a single menu entry (item, divider, header, etc.)
enum MenuEntry {
    /// A standard menu item.
    Item {
        label: String,
        leading_icon: Option<String>,
        trailing_text: Option<String>,
        trailing_icon: Option<String>,
        on_click: Option<ClickHandler>,
        disabled: bool,
        checked: Option<bool>,
    },
    /// A visual divider between groups of items.
    Divider,
    /// A section header label (non-interactive).
    Header { text: String },
    /// A custom element rendered as a menu row.
    Custom { element: AnyElement },
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Menu
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Menu** (dropdown / context menu).
///
/// Menus display a list of choices on a temporary, elevated surface.
/// They appear when the user taps a trigger element (button, icon, long-press)
/// and disappear once a selection is made or the menu is dismissed.
///
/// # Builder API
///
/// Build a menu by chaining item methods:
///
/// | Method | Description |
/// |--------|-------------|
/// | `.item(label, icon, handler)` | Standard item with leading icon |
/// | `.item_with_trailing(label, icon, trailing, handler)` | Item with trailing text |
/// | `.item_checked(label, icon, checked, handler)` | Item with a check mark |
/// | `.item_disabled(label, icon)` | Disabled (non-interactive) item |
/// | `.divider()` | Visual separator between item groups |
/// | `.header(text)` | Non-interactive section label |
/// | `.custom(element)` | Custom element rendered as a menu row |
///
/// # Example
///
/// ```rust,ignore
/// Menu::new(theme)
///     .item("Edit", "✏️", cx.listener(|this, _, _, cx| { ... }))
///     .item("Share", "🔗", cx.listener(|this, _, _, cx| { ... }))
///     .divider()
///     .item("Delete", "🗑️", cx.listener(|this, _, _, cx| { ... }))
/// ```
pub struct Menu {
    theme: MaterialTheme,
    entries: Vec<MenuEntry>,
    /// Optional width override (defaults to content-sized within min/max).
    width: Option<f32>,
    /// Whether to show an elevation shadow. Default: true.
    elevated: bool,
    /// Custom element ID.
    id: Option<gpui::ElementId>,
}

impl Menu {
    /// Create a new menu builder with the given theme.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            entries: Vec::new(),
            width: None,
            elevated: true,
            id: None,
        }
    }

    /// Add a standard menu item with a label, optional leading icon, and
    /// click handler.
    ///
    /// Pass an empty string for `icon` to omit the leading icon.
    pub fn item(
        mut self,
        label: impl Into<String>,
        icon: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        let icon_str = icon.into();
        self.entries.push(MenuEntry::Item {
            label: label.into(),
            leading_icon: if icon_str.is_empty() {
                None
            } else {
                Some(icon_str)
            },
            trailing_text: None,
            trailing_icon: None,
            on_click: Some(Box::new(on_click)),
            disabled: false,
            checked: None,
        });
        self
    }

    /// Add a menu item with trailing text (e.g. keyboard shortcut or sub-menu arrow).
    pub fn item_with_trailing(
        mut self,
        label: impl Into<String>,
        icon: impl Into<String>,
        trailing: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        let icon_str = icon.into();
        self.entries.push(MenuEntry::Item {
            label: label.into(),
            leading_icon: if icon_str.is_empty() {
                None
            } else {
                Some(icon_str)
            },
            trailing_text: Some(trailing.into()),
            trailing_icon: None,
            on_click: Some(Box::new(on_click)),
            disabled: false,
            checked: None,
        });
        self
    }

    /// Add a menu item with a trailing icon (e.g. sub-menu chevron).
    pub fn item_with_trailing_icon(
        mut self,
        label: impl Into<String>,
        leading_icon: impl Into<String>,
        trailing_icon: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        let leading = leading_icon.into();
        let trailing = trailing_icon.into();
        self.entries.push(MenuEntry::Item {
            label: label.into(),
            leading_icon: if leading.is_empty() {
                None
            } else {
                Some(leading)
            },
            trailing_text: None,
            trailing_icon: if trailing.is_empty() {
                None
            } else {
                Some(trailing)
            },
            on_click: Some(Box::new(on_click)),
            disabled: false,
            checked: None,
        });
        self
    }

    /// Add a menu item with a check mark state.
    ///
    /// When `checked` is `true`, a "✓" indicator is shown at the leading
    /// position. When `false`, the leading icon is used (or space is reserved).
    pub fn item_checked(
        mut self,
        label: impl Into<String>,
        icon: impl Into<String>,
        checked: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        let icon_str = icon.into();
        self.entries.push(MenuEntry::Item {
            label: label.into(),
            leading_icon: if icon_str.is_empty() {
                None
            } else {
                Some(icon_str)
            },
            trailing_text: None,
            trailing_icon: None,
            on_click: Some(Box::new(on_click)),
            disabled: false,
            checked: Some(checked),
        });
        self
    }

    /// Add a disabled (non-interactive) menu item.
    ///
    /// Disabled items are rendered at 38% opacity and do not respond to taps.
    pub fn item_disabled(mut self, label: impl Into<String>, icon: impl Into<String>) -> Self {
        let icon_str = icon.into();
        self.entries.push(MenuEntry::Item {
            label: label.into(),
            leading_icon: if icon_str.is_empty() {
                None
            } else {
                Some(icon_str)
            },
            trailing_text: None,
            trailing_icon: None,
            on_click: None,
            disabled: true,
            checked: None,
        });
        self
    }

    /// Add a visual divider between groups of items.
    pub fn divider(mut self) -> Self {
        self.entries.push(MenuEntry::Divider);
        self
    }

    /// Add a non-interactive section header label.
    pub fn header(mut self, text: impl Into<String>) -> Self {
        self.entries.push(MenuEntry::Header { text: text.into() });
        self
    }

    /// Add a custom element rendered as a menu row.
    pub fn custom(mut self, element: impl IntoElement) -> Self {
        self.entries.push(MenuEntry::Custom {
            element: element.into_any_element(),
        });
        self
    }

    /// Set a fixed width for the menu (overrides content-based sizing).
    /// The value is clamped to the MD3 min/max range (112–280dp).
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width.clamp(MIN_MENU_WIDTH, MAX_MENU_WIDTH));
        self
    }

    /// Enable or disable the elevation shadow. Default: true.
    pub fn elevated(mut self, elevated: bool) -> Self {
        self.elevated = elevated;
        self
    }

    /// Set a custom element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for Menu {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let container_bg = color(t.surface_container);
        let on_surface: Hsla = color(t.on_surface);
        let on_surface_variant: Hsla = color(t.on_surface_variant);
        let divider_color = color(t.outline_variant);
        let primary_color = color(t.primary);

        // Shadow for elevation
        let shadow_color: Hsla = if self.elevated {
            let base = color(t.shadow);
            gpui::hsla(base.h, base.s, base.l, 0.15)
        } else {
            TRANSPARENT
        };

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("menu".into()));

        // ── Menu container ───────────────────────────────────────────────

        let mut menu = div()
            .id(elem_id)
            .flex()
            .flex_col()
            .min_w(px(MIN_MENU_WIDTH))
            .max_w(px(MAX_MENU_WIDTH))
            .py(px(MENU_PADDING_V))
            .rounded(px(MENU_RADIUS))
            .bg(container_bg)
            .overflow_hidden();

        if let Some(w) = self.width {
            menu = menu.w(px(w));
        }

        // Elevation shadow simulation via border
        if self.elevated {
            menu = menu.border_1().border_color(shadow_color);
        }

        // ── Render entries ───────────────────────────────────────────────

        for (i, entry) in self.entries.into_iter().enumerate() {
            match entry {
                MenuEntry::Item {
                    label,
                    leading_icon,
                    trailing_text,
                    trailing_icon,
                    on_click,
                    disabled,
                    checked,
                } => {
                    let is_checked = checked.unwrap_or(false);
                    let has_check = checked.is_some();

                    let item_opacity = if disabled { DISABLED_OPACITY } else { 1.0 };

                    // Build item content as children collected into a vec
                    let mut children: Vec<AnyElement> = Vec::new();

                    // Leading area: check mark or icon
                    if has_check {
                        if is_checked {
                            children.push(
                                div()
                                    .w(px(24.0))
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(primary_color)
                                    .flex_shrink_0()
                                    .child("✓")
                                    .into_any_element(),
                            );
                        } else {
                            // Reserve space for the check mark
                            children.push(
                                div()
                                    .w(px(24.0))
                                    .h(px(24.0))
                                    .flex_shrink_0()
                                    .into_any_element(),
                            );
                        }
                    } else if let Some(icon_text) = leading_icon {
                        children.push(
                            div()
                                .w(px(24.0))
                                .h(px(24.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_base()
                                .text_color(on_surface_variant)
                                .flex_shrink_0()
                                .child(icon_text)
                                .into_any_element(),
                        );
                    }

                    // Label
                    children.push(
                        div()
                            .flex_1()
                            .text_base()
                            .line_height(px(24.0))
                            .text_color(on_surface)
                            .child(label)
                            .into_any_element(),
                    );

                    // Trailing text (e.g. keyboard shortcut)
                    if let Some(trailing) = trailing_text {
                        children.push(
                            div()
                                .text_sm()
                                .text_color(on_surface_variant)
                                .flex_shrink_0()
                                .child(trailing)
                                .into_any_element(),
                        );
                    }

                    // Trailing icon (e.g. sub-menu chevron)
                    if let Some(t_icon) = trailing_icon {
                        children.push(
                            div()
                                .w(px(24.0))
                                .h(px(24.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_sm()
                                .text_color(on_surface_variant)
                                .flex_shrink_0()
                                .child(t_icon)
                                .into_any_element(),
                        );
                    }

                    // Build the item row — always use an id so we get a
                    // Stateful<Div> that supports on_mouse_down uniformly.
                    let mut item = div()
                        .id(gpui::ElementId::Name(format!("menu-item-{i}").into()))
                        .flex()
                        .flex_row()
                        .items_center()
                        .w_full()
                        .h(px(ITEM_HEIGHT))
                        .pl(px(ITEM_PADDING_H))
                        .pr(px(ITEM_TRAILING_PADDING))
                        .gap(px(ICON_LABEL_GAP))
                        .opacity(item_opacity);

                    if !disabled {
                        item = item.cursor_pointer();
                    }

                    for child in children {
                        item = item.child(child);
                    }

                    // Click handler (only for enabled items)
                    if let Some(handler) = on_click {
                        if !disabled {
                            item = item.on_mouse_down(MouseButton::Left, handler);
                        }
                    }

                    menu = menu.child(item);
                }

                MenuEntry::Divider => {
                    menu = menu.child(
                        div()
                            .w_full()
                            .h(px(DIVIDER_HEIGHT))
                            .my(px(DIVIDER_MARGIN_V))
                            .bg(divider_color),
                    );
                }

                MenuEntry::Header { text } => {
                    menu = menu.child(
                        div()
                            .w_full()
                            .px(px(ITEM_PADDING_H))
                            .py(px(8.0))
                            .text_xs()
                            .line_height(px(16.0))
                            .text_color(on_surface_variant)
                            .child(text),
                    );
                }

                MenuEntry::Custom { element } => {
                    menu = menu.child(element);
                }
            }
        }

        menu.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  MenuAnchor — wraps a trigger + menu for positioning
// ═══════════════════════════════════════════════════════════════════════════════

/// A wrapper that positions a [`Menu`] relative to an anchor element.
///
/// `MenuAnchor` renders its anchor child normally and, when `open` is `true`,
/// overlays the menu below the anchor using absolute positioning.
///
/// # Example
///
/// ```rust,ignore
/// MenuAnchor::new(theme)
///     .anchor(my_button_element)
///     .menu(
///         Menu::new(theme)
///             .item("Option A", "", cx.listener(|this, _, _, cx| { ... }))
///             .item("Option B", "", cx.listener(|this, _, _, cx| { ... })),
///     )
///     .open(self.menu_visible)
/// ```
#[allow(dead_code)]
pub struct MenuAnchor {
    theme: MaterialTheme,
    anchor: Option<AnyElement>,
    menu: Option<AnyElement>,
    open: bool,
    /// Custom element ID.
    id: Option<gpui::ElementId>,
}

impl MenuAnchor {
    /// Create a new menu anchor wrapper.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            anchor: None,
            menu: None,
            open: false,
            id: None,
        }
    }

    /// Set the anchor element (the trigger, e.g. a button).
    pub fn anchor(mut self, element: impl IntoElement) -> Self {
        self.anchor = Some(element.into_any_element());
        self
    }

    /// Set the menu element to display when open.
    pub fn menu(mut self, menu: impl IntoElement) -> Self {
        self.menu = Some(menu.into_any_element());
        self
    }

    /// Set whether the menu is currently visible.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Set a custom element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for MenuAnchor {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("menu-anchor".into()));

        let mut container = div().id(elem_id).relative();

        // Render the anchor element
        if let Some(anchor_el) = self.anchor {
            container = container.child(anchor_el);
        }

        // Overlay the menu when open
        if self.open {
            if let Some(menu_el) = self.menu {
                let overlay = div()
                    .absolute()
                    .top_full() // position below the anchor
                    .left_0()
                    .child(menu_el);

                container = container.child(overlay);
            }
        }

        container.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / Showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Render a static (non-interactive) demo of menu variants for the
/// component showcase gallery.
pub fn menu_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);
    let label_color = color(theme.on_surface_variant);

    div()
        .flex()
        .flex_col()
        .gap_6()
        .w_full()
        .p_4()
        // ── Basic menu ───────────────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().text_color(label_color).child("Basic Menu"))
                .child(
                    Menu::new(theme)
                        .item("Cut", "✂️", |_, _, _| {})
                        .item("Copy", "📋", |_, _, _| {})
                        .item("Paste", "📄", |_, _, _| {})
                        .width(200.0),
                ),
        )
        // ── Menu with divider and sections ───────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Menu with Divider & Header"),
                )
                .child(
                    Menu::new(theme)
                        .header("Edit")
                        .item("Undo", "↩️", |_, _, _| {})
                        .item("Redo", "↪️", |_, _, _| {})
                        .divider()
                        .header("Clipboard")
                        .item("Cut", "✂️", |_, _, _| {})
                        .item("Copy", "📋", |_, _, _| {})
                        .item("Paste", "📄", |_, _, _| {})
                        .width(220.0),
                ),
        )
        // ── Menu with trailing text (shortcuts) ──────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Menu with Shortcuts"),
                )
                .child(
                    Menu::new(theme)
                        .item_with_trailing("New File", "📝", "⌘N", |_, _, _| {})
                        .item_with_trailing("Open", "📂", "⌘O", |_, _, _| {})
                        .item_with_trailing("Save", "💾", "⌘S", |_, _, _| {})
                        .divider()
                        .item_with_trailing("Close", "", "⌘W", |_, _, _| {})
                        .width(260.0),
                ),
        )
        // ── Menu with check marks ────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Menu with Check Marks"),
                )
                .child(
                    Menu::new(theme)
                        .item_checked("Bold", "B", true, |_, _, _| {})
                        .item_checked("Italic", "I", false, |_, _, _| {})
                        .item_checked("Underline", "U", true, |_, _, _| {})
                        .divider()
                        .item_disabled("Strikethrough", "S")
                        .width(220.0),
                ),
        )
        // ── Menu with disabled items ─────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Menu with Disabled Items"),
                )
                .child(
                    Menu::new(theme)
                        .item("Edit", "✏️", |_, _, _| {})
                        .item("Share", "🔗", |_, _, _| {})
                        .divider()
                        .item_disabled("Delete", "🗑️")
                        .item_disabled("Archive", "📦")
                        .width(200.0),
                ),
        )
        // ── Menu with sub-menu indicator ─────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Menu with Sub-menu Indicators"),
                )
                .child(
                    Menu::new(theme)
                        .item("New", "📝", |_, _, _| {})
                        .item_with_trailing_icon("Open Recent", "📂", "▸", |_, _, _| {})
                        .divider()
                        .item_with_trailing_icon("Export As", "📤", "▸", |_, _, _| {})
                        .item("Print", "🖨️", |_, _, _| {})
                        .width(240.0),
                ),
        )
        // ── Compact menu (no icons) ──────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Compact Menu (no icons)"),
                )
                .child(
                    Menu::new(theme)
                        .item("Small", "", |_, _, _| {})
                        .item("Medium", "", |_, _, _| {})
                        .item("Large", "", |_, _, _| {})
                        .item("Extra Large", "", |_, _, _| {})
                        .elevated(false),
                ),
        )
}
