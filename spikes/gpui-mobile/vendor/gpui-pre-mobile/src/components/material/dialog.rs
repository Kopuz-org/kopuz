//! Material Design 3 Dialog components.
//!
//! Dialogs inform users about a task and can contain critical information,
//! require decisions, or involve multiple tasks. MD3 defines three dialog
//! types:
//!
//! - **Basic dialog** — title, content, and action buttons
//! - **Full-screen dialog** — edge-to-edge with a top app bar
//! - **Simple dialog** — a list of choices (no explicit action buttons)
//!
//! ## Architecture
//!
//! All dialog structs use a **builder pattern** and implement `IntoElement`,
//! making them composable with GPUI's standard `.child(...)` API. Click
//! handlers use GPUI's `on_mouse_down` signature, so `cx.listener(...)`
//! works directly.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::dialog::{BasicDialog, FullScreenDialog, SimpleDialog};
//! use gpui_mobile::components::material::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Basic confirmation dialog
//! let dialog = BasicDialog::new(theme)
//!     .title("Discard draft?")
//!     .content("Your unsaved changes will be lost.")
//!     .dismiss_button("Cancel", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .confirm_button("Discard", cx.listener(|this, _, _, cx| { /* ... */ }));
//!
//! // Full-screen dialog
//! let fs_dialog = FullScreenDialog::new(theme)
//!     .title("New Event")
//!     .close_button(cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .save_button("Save", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .child(some_form_content);
//!
//! // Simple dialog with choices
//! let simple = SimpleDialog::new(theme)
//!     .title("Choose ringtone")
//!     .item("None", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .item("Callisto", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .item("Luna", cx.listener(|this, _, _, cx| { /* ... */ }));
//! ```

use gpui::{div, prelude::*, px, AnyElement, Hsla, MouseButton, MouseDownEvent, Stateful, Window};

use super::theme::{color, MaterialTheme, ShapeScale};

// ── Constants ────────────────────────────────────────────────────────────────

/// Dialog container corner radius (MD3: extra-large = 28dp).
const DIALOG_RADIUS: f32 = ShapeScale::EXTRA_LARGE;

/// Padding inside the dialog container.
const DIALOG_PADDING: f32 = 24.0;

/// Gap between the title and the content.
const TITLE_CONTENT_GAP: f32 = 16.0;

/// Gap between the content and the action buttons.
const CONTENT_ACTIONS_GAP: f32 = 24.0;

/// Gap between individual action buttons.
const ACTION_GAP: f32 = 8.0;

/// Minimum dialog width.
const MIN_DIALOG_WIDTH: f32 = 280.0;

/// Maximum dialog width.
const MAX_DIALOG_WIDTH: f32 = 560.0;

/// Scrim (overlay backdrop) opacity.
const SCRIM_OPACITY: f32 = 0.32;

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ═══════════════════════════════════════════════════════════════════════════════
//  Basic Dialog
// ═══════════════════════════════════════════════════════════════════════════════

/// Descriptor for a dialog action button.
struct DialogAction {
    label: String,
    on_click: ClickHandler,
    filled: bool,
}

/// A Material Design 3 **basic dialog**.
///
/// Basic dialogs present a title, supporting text, and 1–2 action buttons.
/// They interrupt the user flow and should be used for critical confirmations.
///
/// # MD3 Specification
///
/// - Container: surface-container-high, extra-large shape (28dp radius)
/// - Title: headline-small, on-surface
/// - Supporting text: body-medium, on-surface-variant
/// - Actions: aligned to the trailing edge (right in LTR)
/// - Scrim: scrim colour at 32% opacity behind the dialog
///
/// # Example
///
/// ```rust,ignore
/// BasicDialog::new(theme)
///     .title("Delete item?")
///     .content("This action cannot be undone.")
///     .dismiss_button("Cancel", cx.listener(|this, _, _, cx| { ... }))
///     .confirm_button("Delete", cx.listener(|this, _, _, cx| { ... }))
/// ```
pub struct BasicDialog {
    theme: MaterialTheme,
    icon: Option<String>,
    title: Option<String>,
    content: Option<String>,
    content_element: Option<AnyElement>,
    actions: Vec<DialogAction>,
    on_scrim_tap: Option<ClickHandler>,
    show_scrim: bool,
    id: Option<gpui::ElementId>,
}

impl BasicDialog {
    /// Create a new basic dialog builder with the given theme.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            icon: None,
            title: None,
            content: None,
            content_element: None,
            actions: Vec::new(),
            on_scrim_tap: None,
            show_scrim: true,
            id: None,
        }
    }

    /// Set an optional hero icon displayed above the title (emoji or short text).
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the dialog title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the supporting text content.
    pub fn content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Set a custom element as the dialog body (replaces text content).
    pub fn content_element(mut self, element: impl IntoElement) -> Self {
        self.content_element = Some(element.into_any_element());
        self
    }

    /// Add a text-style dismiss button (e.g. "Cancel").
    pub fn dismiss_button(
        mut self,
        label: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.actions.push(DialogAction {
            label: label.into(),
            on_click: Box::new(on_click),
            filled: false,
        });
        self
    }

    /// Add a filled-tonal confirm button (e.g. "Delete", "Accept").
    pub fn confirm_button(
        mut self,
        label: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.actions.push(DialogAction {
            label: label.into(),
            on_click: Box::new(on_click),
            filled: true,
        });
        self
    }

    /// Add a custom action button.
    pub fn action(
        mut self,
        label: impl Into<String>,
        filled: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.actions.push(DialogAction {
            label: label.into(),
            on_click: Box::new(on_click),
            filled,
        });
        self
    }

    /// Set a handler for tapping the scrim (backdrop) to dismiss.
    pub fn on_scrim_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_scrim_tap = Some(Box::new(handler));
        self
    }

    /// Whether to show the scrim backdrop. Default: `true`.
    pub fn show_scrim(mut self, show: bool) -> Self {
        self.show_scrim = show;
        self
    }

    /// Set a custom element ID for the dialog container.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for BasicDialog {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let scrim_color: Hsla = {
            let base = color(t.scrim);
            gpui::hsla(base.h, base.s, base.l, SCRIM_OPACITY)
        };

        let container_bg = color(t.surface_container_high);
        let title_color = color(t.on_surface);
        let content_color = color(t.on_surface_variant);
        let primary_color = color(t.primary);
        let on_primary = color(t.on_primary);

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("basic-dialog".into()));

        // ── Scrim layer ──────────────────────────────────────────────────

        let mut scrim = div()
            .id(elem_id)
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .flex()
            .items_center()
            .justify_center();

        if self.show_scrim {
            scrim = scrim.bg(scrim_color);

            if let Some(handler) = self.on_scrim_tap {
                scrim = scrim.on_mouse_down(MouseButton::Left, handler);
            }
        }

        // ── Dialog container ─────────────────────────────────────────────

        let mut dialog = div()
            .flex()
            .flex_col()
            .min_w(px(MIN_DIALOG_WIDTH))
            .max_w(px(MAX_DIALOG_WIDTH))
            .p(px(DIALOG_PADDING))
            .rounded(px(DIALOG_RADIUS))
            .bg(container_bg)
            .mx_6(); // horizontal margin from screen edges

        // Icon (centered above title)
        let has_icon = self.icon.is_some();
        if let Some(icon_text) = self.icon {
            dialog = dialog.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w_full()
                    .pb(px(TITLE_CONTENT_GAP))
                    .text_2xl()
                    .text_color(primary_color)
                    .child(icon_text),
            );
        }

        // Title
        if let Some(title_text) = self.title {
            dialog = dialog.child(
                div()
                    .w_full()
                    .text_color(title_color)
                    .text_xl() // headline-small
                    .line_height(px(32.0))
                    .when(has_icon, |d| d.text_center())
                    .child(title_text),
            );
        }

        // Content
        if let Some(custom_element) = self.content_element {
            dialog = dialog.child(
                div()
                    .w_full()
                    .pt(px(TITLE_CONTENT_GAP))
                    .child(custom_element),
            );
        } else if let Some(content_text) = self.content {
            dialog = dialog.child(
                div()
                    .w_full()
                    .pt(px(TITLE_CONTENT_GAP))
                    .text_sm() // body-medium
                    .line_height(px(20.0))
                    .text_color(content_color)
                    .child(content_text),
            );
        }

        // Actions row
        if !self.actions.is_empty() {
            let mut actions_row = div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(px(ACTION_GAP))
                .pt(px(CONTENT_ACTIONS_GAP))
                .w_full();

            for (i, action) in self.actions.into_iter().enumerate() {
                let btn = if action.filled {
                    div()
                        .id(gpui::ElementId::Name(format!("dialog-action-{i}").into()))
                        .px(px(24.0))
                        .py(px(10.0))
                        .rounded(px(ShapeScale::FULL))
                        .bg(primary_color)
                        .text_color(on_primary)
                        .text_sm()
                        .line_height(px(20.0))
                        .cursor_pointer()
                        .child(action.label)
                        .on_mouse_down(MouseButton::Left, action.on_click)
                } else {
                    div()
                        .id(gpui::ElementId::Name(format!("dialog-action-{i}").into()))
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(ShapeScale::FULL))
                        .text_color(primary_color)
                        .text_sm()
                        .line_height(px(20.0))
                        .cursor_pointer()
                        .child(action.label)
                        .on_mouse_down(MouseButton::Left, action.on_click)
                };

                actions_row = actions_row.child(btn);
            }

            dialog = dialog.child(actions_row);
        }

        // Stop propagation on the dialog container itself so that tapping
        // inside the dialog doesn't trigger the scrim handler.
        dialog = dialog.on_mouse_down(MouseButton::Left, |_, _, _| {
            // Intentionally empty — just captures the event.
        });

        scrim = scrim.child(dialog);
        scrim.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Full-Screen Dialog
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **full-screen dialog**.
///
/// Full-screen dialogs fill the entire screen, containing a top app bar
/// with close and save actions, and a scrollable body. They are used for
/// complex forms or content creation tasks on mobile.
///
/// # MD3 Specification
///
/// - Container: surface, full viewport
/// - Top app bar: close icon (leading), title (center), save action (trailing)
/// - Body: scrollable content area
///
/// # Example
///
/// ```rust,ignore
/// FullScreenDialog::new(theme)
///     .title("New Event")
///     .close_button(cx.listener(|this, _, _, cx| { ... }))
///     .save_button("Save", cx.listener(|this, _, _, cx| { ... }))
///     .child(my_form)
/// ```
pub struct FullScreenDialog {
    theme: MaterialTheme,
    title: Option<String>,
    close_handler: Option<ClickHandler>,
    save_label: Option<String>,
    save_handler: Option<ClickHandler>,
    children: Vec<AnyElement>,
    id: Option<gpui::ElementId>,
}

impl FullScreenDialog {
    /// Create a new full-screen dialog builder.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            title: None,
            close_handler: None,
            save_label: None,
            save_handler: None,
            children: Vec::new(),
            id: None,
        }
    }

    /// Set the dialog title shown in the top app bar.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the close button handler (typically navigates back or dismisses).
    pub fn close_button(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.close_handler = Some(Box::new(handler));
        self
    }

    /// Set the save/confirm action button in the top app bar.
    pub fn save_button(
        mut self,
        label: impl Into<String>,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.save_label = Some(label.into());
        self.save_handler = Some(Box::new(handler));
        self
    }

    /// Add a child element to the dialog body.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    /// Add multiple children to the dialog body.
    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        for child in children {
            self.children.push(child.into_any_element());
        }
        self
    }

    /// Set a custom element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for FullScreenDialog {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let surface = color(t.surface);
        let on_surface = color(t.on_surface);
        let primary_color = color(t.primary);
        let surface_container = color(t.surface_container);

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("fullscreen-dialog".into()));

        // ── Top app bar ──────────────────────────────────────────────────

        let mut top_bar = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(56.0))
            .px(px(16.0))
            .bg(surface_container);

        // Close button (leading)
        if let Some(close_handler) = self.close_handler {
            top_bar = top_bar.child(
                div()
                    .id("fs-dialog-close")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(40.0))
                    .h(px(40.0))
                    .rounded(px(20.0))
                    .cursor_pointer()
                    .text_lg()
                    .text_color(on_surface)
                    .child("✕")
                    .on_mouse_down(MouseButton::Left, close_handler),
            );
        }

        // Title (center)
        top_bar = top_bar.child(
            div()
                .flex_1()
                .text_lg()
                .text_color(on_surface)
                .pl(px(16.0))
                .child(self.title.unwrap_or_default()),
        );

        // Save action (trailing)
        if let Some(save_handler) = self.save_handler {
            let label = self.save_label.unwrap_or_else(|| "Save".into());
            top_bar = top_bar.child(
                div()
                    .id("fs-dialog-save")
                    .px(px(24.0))
                    .py(px(10.0))
                    .rounded(px(ShapeScale::FULL))
                    .bg(primary_color)
                    .text_color(color(t.on_primary))
                    .text_sm()
                    .cursor_pointer()
                    .child(label)
                    .on_mouse_down(MouseButton::Left, save_handler),
            );
        }

        // ── Body ─────────────────────────────────────────────────────────

        let mut body = div()
            .id("fs-dialog-body")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .p(px(24.0));

        for child in self.children {
            body = body.child(child);
        }

        // ── Container ────────────────────────────────────────────────────

        div()
            .id(elem_id)
            .size_full()
            .flex()
            .flex_col()
            .bg(surface)
            .child(top_bar)
            .child(body)
            .into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Simple Dialog
// ═══════════════════════════════════════════════════════════════════════════════

/// A single item in a [`SimpleDialog`].
struct SimpleDialogItem {
    label: String,
    icon: Option<String>,
    on_click: ClickHandler,
    selected: bool,
}

/// A Material Design 3 **simple dialog**.
///
/// Simple dialogs present a list of items (choices) and an optional title.
/// Selecting an item immediately performs the action and closes the dialog
/// (no explicit confirm/cancel buttons).
///
/// # MD3 Specification
///
/// - Container: surface-container-high, extra-large shape
/// - Title: headline-small, on-surface
/// - Items: body-large, on-surface-variant, full-width tap targets
/// - Scrim behind the dialog
///
/// # Example
///
/// ```rust,ignore
/// SimpleDialog::new(theme)
///     .title("Set backup account")
///     .item_with_icon("✉️", "user@example.com", false, cx.listener(...))
///     .item_with_icon("✉️", "user2@example.com", true, cx.listener(...))
///     .item_with_icon("➕", "Add account", false, cx.listener(...))
/// ```
pub struct SimpleDialog {
    theme: MaterialTheme,
    title: Option<String>,
    items: Vec<SimpleDialogItem>,
    on_scrim_tap: Option<ClickHandler>,
    show_scrim: bool,
    id: Option<gpui::ElementId>,
}

impl SimpleDialog {
    /// Create a new simple dialog builder.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            title: None,
            items: Vec::new(),
            on_scrim_tap: None,
            show_scrim: true,
            id: None,
        }
    }

    /// Set the dialog title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Add a text-only choice item.
    pub fn item(
        mut self,
        label: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.items.push(SimpleDialogItem {
            label: label.into(),
            icon: None,
            on_click: Box::new(on_click),
            selected: false,
        });
        self
    }

    /// Add a choice item with a leading icon and optional selected state.
    pub fn item_with_icon(
        mut self,
        icon: impl Into<String>,
        label: impl Into<String>,
        selected: bool,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.items.push(SimpleDialogItem {
            label: label.into(),
            icon: Some(icon.into()),
            on_click: Box::new(on_click),
            selected,
        });
        self
    }

    /// Set a handler for tapping the scrim to dismiss.
    pub fn on_scrim_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_scrim_tap = Some(Box::new(handler));
        self
    }

    /// Whether to show the scrim backdrop. Default: `true`.
    pub fn show_scrim(mut self, show: bool) -> Self {
        self.show_scrim = show;
        self
    }

    /// Set a custom element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for SimpleDialog {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let scrim_color: Hsla = {
            let base = color(t.scrim);
            gpui::hsla(base.h, base.s, base.l, SCRIM_OPACITY)
        };

        let container_bg = color(t.surface_container_high);
        let title_color = color(t.on_surface);
        let item_color = color(t.on_surface_variant);
        let selected_color = color(t.primary);
        let selected_bg = color(t.secondary_container);

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("simple-dialog".into()));

        // ── Scrim ────────────────────────────────────────────────────────

        let mut scrim = div()
            .id(elem_id)
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .flex()
            .items_center()
            .justify_center();

        if self.show_scrim {
            scrim = scrim.bg(scrim_color);

            if let Some(handler) = self.on_scrim_tap {
                scrim = scrim.on_mouse_down(MouseButton::Left, handler);
            }
        }

        // ── Dialog container ─────────────────────────────────────────────

        let mut dialog = div()
            .flex()
            .flex_col()
            .min_w(px(MIN_DIALOG_WIDTH))
            .max_w(px(MAX_DIALOG_WIDTH))
            .py(px(DIALOG_PADDING))
            .rounded(px(DIALOG_RADIUS))
            .bg(container_bg)
            .mx_6();

        // Title
        if let Some(title_text) = self.title {
            dialog = dialog.child(
                div()
                    .w_full()
                    .px(px(DIALOG_PADDING))
                    .pb(px(TITLE_CONTENT_GAP))
                    .text_xl()
                    .line_height(px(32.0))
                    .text_color(title_color)
                    .child(title_text),
            );
        }

        // Items
        for (i, item) in self.items.into_iter().enumerate() {
            let text_col = if item.selected {
                selected_color
            } else {
                item_color
            };

            let mut row = div()
                .id(gpui::ElementId::Name(
                    format!("simple-dialog-item-{i}").into(),
                ))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(16.0))
                .w_full()
                .px(px(DIALOG_PADDING))
                .py(px(12.0))
                .cursor_pointer();

            if item.selected {
                row = row.bg(selected_bg);
            }

            // Leading icon
            if let Some(icon_text) = item.icon {
                row = row.child(div().text_lg().text_color(text_col).child(icon_text));
            }

            // Label
            row = row.child(
                div()
                    .flex_1()
                    .text_base()
                    .line_height(px(24.0))
                    .text_color(text_col)
                    .child(item.label),
            );

            // Selected indicator
            if item.selected {
                row = row.child(div().text_sm().text_color(selected_color).child("✓"));
            }

            row = row.on_mouse_down(MouseButton::Left, item.on_click);

            dialog = dialog.child(row);
        }

        // Stop propagation inside dialog
        dialog = dialog.on_mouse_down(MouseButton::Left, |_, _, _| {});

        scrim = scrim.child(dialog);
        scrim.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Render a static (non-interactive) demo of all dialog variants.
///
/// Used in the component showcase gallery. Items do not have functional
/// handlers — they only demonstrate the visual layout.
pub fn dialog_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_6()
        .w_full()
        .p_4()
        // ── Basic dialog ─────────────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(color(theme.on_surface_variant))
                        .child("Basic Dialog"),
                )
                .child(
                    BasicDialog::new(theme)
                        .show_scrim(false)
                        .icon("🗑️")
                        .title("Delete item?")
                        .content(
                            "This item will be permanently removed from your library. \
                             This action cannot be undone.",
                        )
                        .dismiss_button("Cancel", |_, _, _| {})
                        .confirm_button("Delete", |_, _, _| {}),
                ),
        )
        // ── Basic dialog without icon ────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(color(theme.on_surface_variant))
                        .child("Basic Dialog (no icon)"),
                )
                .child(
                    BasicDialog::new(theme)
                        .show_scrim(false)
                        .title("Discard draft?")
                        .content("Your unsaved changes will be lost.")
                        .dismiss_button("Cancel", |_, _, _| {})
                        .confirm_button("Discard", |_, _, _| {}),
                ),
        )
        // ── Simple dialog ────────────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(color(theme.on_surface_variant))
                        .child("Simple Dialog"),
                )
                .child(
                    SimpleDialog::new(theme)
                        .show_scrim(false)
                        .title("Set backup account")
                        .item_with_icon("✉️", "user@example.com", false, |_, _, _| {})
                        .item_with_icon("✉️", "work@example.com", true, |_, _, _| {})
                        .item_with_icon("➕", "Add account", false, |_, _, _| {}),
                ),
        )
}
