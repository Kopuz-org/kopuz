//! Material Design 3 Search Bar and Search View components.
//!
//! Search allows users to enter a keyword or phrase to get relevant
//! information. MD3 defines two search patterns:
//!
//! - **Search bar** — a persistent bar (usually at the top of the screen)
//!   that displays the current query and can expand into a search view.
//! - **Search view** — an expanded overlay showing the input field,
//!   suggestions, and search results.
//!
//! ## Architecture
//!
//! All components use a **builder pattern** and implement `IntoElement`,
//! making them composable with GPUI's standard `.child(...)` API. Click
//! handlers use GPUI's `on_mouse_down` signature, so `cx.listener(...)`
//! works directly.
//!
//! Since GPUI does not yet provide a native text-input element from the
//! `div()` API, the search bar renders a **visual representation** of the
//! search field (placeholder text, leading/trailing icons, query text).
//! Actual keyboard input must be wired externally by the consumer.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::search_bar::{SearchBar, SearchView};
//! use gpui_mobile::components::material::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Collapsed search bar
//! let bar = SearchBar::new(theme)
//!     .placeholder("Search emails")
//!     .leading_icon("🔍")
//!     .trailing_icon("🎤")
//!     .on_tap(cx.listener(|this, _, _, cx| { /* expand to search view */ }));
//!
//! // Search bar with a current query
//! let bar_with_query = SearchBar::new(theme)
//!     .query("Material Design")
//!     .leading_icon("←")
//!     .trailing_icon("✕")
//!     .on_leading_tap(cx.listener(|this, _, _, cx| { /* go back */ }))
//!     .on_trailing_tap(cx.listener(|this, _, _, cx| { /* clear query */ }));
//!
//! // Search view with suggestion items
//! let view = SearchView::new(theme)
//!     .query("flutter")
//!     .leading_icon("←")
//!     .suggestion("🕐", "flutter widgets", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .suggestion("🔍", "flutter animation", cx.listener(|this, _, _, cx| { /* ... */ }))
//!     .suggestion("🔍", "flutter material 3", cx.listener(|this, _, _, cx| { /* ... */ }));
//! ```
//!
//! ## MD3 Specification Reference
//!
//! - Search bar height: 56dp
//! - Container: surface-container-high, full shape (28dp radius)
//! - Leading icon: on-surface, 24dp
//! - Placeholder/input: on-surface-variant (placeholder) / on-surface (input)
//! - Trailing icon(s): on-surface-variant
//! - Search view: surface-container-high, full width, with divider below input

use gpui::{div, prelude::*, px, AnyElement, Hsla, MouseButton, MouseDownEvent, Stateful, Window};

use super::theme::{color, MaterialTheme};

// ── Constants ────────────────────────────────────────────────────────────────

/// Search bar height in dp (MD3 spec: 56dp).
const SEARCH_BAR_HEIGHT: f32 = 56.0;

/// Search bar corner radius (MD3: full shape = 28dp).
const SEARCH_BAR_RADIUS: f32 = 28.0;

/// Horizontal padding inside the search bar.
const SEARCH_BAR_PADDING_H: f32 = 16.0;

/// Icon container size.
const ICON_SIZE: f32 = 40.0;

/// Gap between icon and text content.
const ICON_TEXT_GAP: f32 = 8.0;

/// Suggestion item vertical padding.
const SUGGESTION_PADDING_V: f32 = 12.0;

/// Suggestion item horizontal padding.
const SUGGESTION_PADDING_H: f32 = 16.0;

/// Suggestion icon-label gap.
const SUGGESTION_GAP: f32 = 16.0;

/// Divider height.
const DIVIDER_HEIGHT: f32 = 1.0;

// ── Click handler alias ──────────────────────────────────────────────────────

type ClickHandler = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>;

// ═══════════════════════════════════════════════════════════════════════════════
//  Search Bar
// ═══════════════════════════════════════════════════════════════════════════════

/// A Material Design 3 **Search Bar**.
///
/// Search bars sit at the top of the screen and display a search field
/// with optional leading and trailing icons. They can be tapped to expand
/// into a full [`SearchView`].
///
/// # MD3 Specification
///
/// - Height: 56dp
/// - Container: surface-container-high, full shape (28dp radius)
/// - Leading icon: on-surface colour, typically a search or menu icon
/// - Placeholder: on-surface-variant, body-large
/// - Trailing icon(s): on-surface-variant, typically voice/avatar
/// - Elevation: level 2 (tonal)
///
/// # Example
///
/// ```rust,ignore
/// SearchBar::new(theme)
///     .placeholder("Hinted search text")
///     .leading_icon("🔍")
///     .trailing_icon("🎤")
///     .on_tap(cx.listener(|this, _, _, cx| {
///         this.show_search_view = true;
///         cx.notify();
///     }))
/// ```
pub struct SearchBar {
    theme: MaterialTheme,
    /// Placeholder text shown when there is no query.
    placeholder: Option<String>,
    /// Current query text (if any).
    query: Option<String>,
    /// Leading icon text (emoji or short string, e.g. "🔍" or "←").
    leading_icon: Option<String>,
    /// Trailing icon text (emoji or short string, e.g. "🎤" or "✕").
    trailing_icon: Option<String>,
    /// Second trailing icon (e.g. avatar).
    trailing_icon2: Option<String>,
    /// Handler for tapping the search bar body (expand to search view).
    on_tap: Option<ClickHandler>,
    /// Handler for tapping the leading icon.
    on_leading_tap: Option<ClickHandler>,
    /// Handler for tapping the trailing icon.
    on_trailing_tap: Option<ClickHandler>,
    /// Handler for tapping the second trailing icon.
    on_trailing2_tap: Option<ClickHandler>,
    /// Whether the search bar is elevated (shadow simulation).
    elevated: bool,
    /// Full width (stretch to fill container). Default: true.
    full_width: bool,
    /// Custom element ID.
    id: Option<gpui::ElementId>,
}

impl SearchBar {
    /// Create a new search bar builder with the given theme.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            placeholder: None,
            query: None,
            leading_icon: None,
            trailing_icon: None,
            trailing_icon2: None,
            on_tap: None,
            on_leading_tap: None,
            on_trailing_tap: None,
            on_trailing2_tap: None,
            elevated: false,
            full_width: true,
            id: None,
        }
    }

    /// Set the placeholder text shown when there is no query.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Set the current query text. When set, the placeholder is hidden
    /// and the query text is displayed instead.
    pub fn query(mut self, text: impl Into<String>) -> Self {
        let t = text.into();
        if t.is_empty() {
            self.query = None;
        } else {
            self.query = Some(t);
        }
        self
    }

    /// Set the leading icon (e.g. "🔍", "☰", or "←").
    pub fn leading_icon(mut self, icon: impl Into<String>) -> Self {
        self.leading_icon = Some(icon.into());
        self
    }

    /// Set the primary trailing icon (e.g. "🎤", "✕").
    pub fn trailing_icon(mut self, icon: impl Into<String>) -> Self {
        self.trailing_icon = Some(icon.into());
        self
    }

    /// Set a second trailing icon (e.g. avatar or additional action).
    pub fn trailing_icon2(mut self, icon: impl Into<String>) -> Self {
        self.trailing_icon2 = Some(icon.into());
        self
    }

    /// Set a handler for tapping the search bar body.
    ///
    /// This is typically used to expand the bar into a full search view.
    pub fn on_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_tap = Some(Box::new(handler));
        self
    }

    /// Set a handler for tapping the leading icon.
    pub fn on_leading_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_leading_tap = Some(Box::new(handler));
        self
    }

    /// Set a handler for tapping the primary trailing icon.
    pub fn on_trailing_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_trailing_tap = Some(Box::new(handler));
        self
    }

    /// Set a handler for tapping the second trailing icon.
    pub fn on_trailing2_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_trailing2_tap = Some(Box::new(handler));
        self
    }

    /// Enable or disable the elevated (shadow) appearance. Default: false.
    pub fn elevated(mut self, elevated: bool) -> Self {
        self.elevated = elevated;
        self
    }

    /// Whether the search bar stretches to fill its container. Default: true.
    pub fn full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    /// Set a custom element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for SearchBar {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let container_bg = color(t.surface_container_high);
        let on_surface = color(t.on_surface);
        let on_surface_variant = color(t.on_surface_variant);

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("search-bar".into()));

        // ── Outer container ──────────────────────────────────────────────

        let mut bar = div()
            .id(elem_id)
            .flex()
            .flex_row()
            .items_center()
            .h(px(SEARCH_BAR_HEIGHT))
            .px(px(SEARCH_BAR_PADDING_H))
            .gap(px(ICON_TEXT_GAP))
            .rounded(px(SEARCH_BAR_RADIUS))
            .bg(container_bg)
            .cursor_pointer();

        if self.full_width {
            bar = bar.w_full();
        }

        // Elevated shadow simulation
        if self.elevated {
            // Simulate MD3 elevation level 2 with a subtle border + opacity trick
            let shadow_color: Hsla = {
                let base = color(t.shadow);
                gpui::hsla(base.h, base.s, base.l, 0.15)
            };
            bar = bar.border_1().border_color(shadow_color);
        }

        // Tap handler for the entire bar
        if let Some(tap_handler) = self.on_tap {
            bar = bar.on_mouse_down(MouseButton::Left, tap_handler);
        }

        // ── Leading icon ─────────────────────────────────────────────────

        if let Some(icon) = self.leading_icon {
            if let Some(handler) = self.on_leading_tap {
                let icon_el = div()
                    .id("search-bar-leading")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface)
                    .flex_shrink_0()
                    .cursor_pointer()
                    .child(icon)
                    .on_mouse_down(MouseButton::Left, handler);
                bar = bar.child(icon_el);
            } else {
                let icon_el = div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface)
                    .flex_shrink_0()
                    .child(icon);
                bar = bar.child(icon_el);
            }
        }

        // ── Text area (placeholder or query) ─────────────────────────────

        let text_content = if let Some(ref query_text) = self.query {
            div()
                .flex_1()
                .text_base()
                .line_height(px(24.0))
                .text_color(on_surface)
                .overflow_hidden()
                .child(query_text.clone())
        } else {
            let placeholder_text = self.placeholder.as_deref().unwrap_or("Search").to_string();
            div()
                .flex_1()
                .text_base()
                .line_height(px(24.0))
                .text_color(on_surface_variant)
                .overflow_hidden()
                .child(placeholder_text)
        };

        bar = bar.child(text_content);

        // ── Trailing icon ────────────────────────────────────────────────

        if let Some(icon) = self.trailing_icon {
            if let Some(handler) = self.on_trailing_tap {
                let icon_el = div()
                    .id("search-bar-trailing")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface_variant)
                    .flex_shrink_0()
                    .cursor_pointer()
                    .child(icon)
                    .on_mouse_down(MouseButton::Left, handler);
                bar = bar.child(icon_el);
            } else {
                let icon_el = div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface_variant)
                    .flex_shrink_0()
                    .child(icon);
                bar = bar.child(icon_el);
            }
        }

        // ── Second trailing icon ─────────────────────────────────────────

        if let Some(icon) = self.trailing_icon2 {
            if let Some(handler) = self.on_trailing2_tap {
                let icon_el = div()
                    .id("search-bar-trailing2")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface_variant)
                    .flex_shrink_0()
                    .cursor_pointer()
                    .child(icon)
                    .on_mouse_down(MouseButton::Left, handler);
                bar = bar.child(icon_el);
            } else {
                let icon_el = div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface_variant)
                    .flex_shrink_0()
                    .child(icon);
                bar = bar.child(icon_el);
            }
        }

        bar.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Search View
// ═══════════════════════════════════════════════════════════════════════════════

/// A single suggestion entry in a [`SearchView`].
struct SearchSuggestion {
    icon: String,
    text: String,
    on_click: ClickHandler,
}

/// A Material Design 3 **Search View**.
///
/// Search views expand from a search bar to fill the screen, showing the
/// input field at the top with a list of suggestions or results below.
///
/// # MD3 Specification
///
/// - Container: surface-container-high, full width
/// - Input area: same height as search bar (56dp), with leading icon and
///   clear/trailing icons
/// - Divider: outline-variant, 1dp below the input row
/// - Suggestions: list tiles with leading icon, body-large text
///
/// # Example
///
/// ```rust,ignore
/// SearchView::new(theme)
///     .query("material")
///     .leading_icon("←")
///     .on_leading_tap(cx.listener(|this, _, _, cx| { /* collapse */ }))
///     .suggestion("🕐", "material design", cx.listener(|this, _, _, cx| { ... }))
///     .suggestion("🔍", "material you", cx.listener(|this, _, _, cx| { ... }))
///     .suggestion("🔍", "material 3 components", cx.listener(|this, _, _, cx| { ... }))
///     .child(some_result_content) // optional body below suggestions
/// ```
pub struct SearchView {
    theme: MaterialTheme,
    /// Current query text displayed in the input area.
    query: Option<String>,
    /// Placeholder text when query is empty.
    placeholder: Option<String>,
    /// Leading icon (typically "←" for back navigation).
    leading_icon: Option<String>,
    /// Trailing icon (typically "✕" for clearing the query).
    trailing_icon: Option<String>,
    /// Handler for tapping the leading icon.
    on_leading_tap: Option<ClickHandler>,
    /// Handler for tapping the trailing icon.
    on_trailing_tap: Option<ClickHandler>,
    /// Suggestion items.
    suggestions: Vec<SearchSuggestion>,
    /// Additional body content (rendered below the suggestions).
    body_children: Vec<AnyElement>,
    /// Custom element ID.
    id: Option<gpui::ElementId>,
}

impl SearchView {
    /// Create a new search view builder.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            query: None,
            placeholder: None,
            leading_icon: None,
            trailing_icon: None,
            on_leading_tap: None,
            on_trailing_tap: None,
            suggestions: Vec::new(),
            body_children: Vec::new(),
            id: None,
        }
    }

    /// Set the current query text.
    pub fn query(mut self, text: impl Into<String>) -> Self {
        let t = text.into();
        if t.is_empty() {
            self.query = None;
        } else {
            self.query = Some(t);
        }
        self
    }

    /// Set the placeholder text when query is empty.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Set the leading icon (e.g. "←" for back).
    pub fn leading_icon(mut self, icon: impl Into<String>) -> Self {
        self.leading_icon = Some(icon.into());
        self
    }

    /// Set the trailing icon (e.g. "✕" for clear).
    pub fn trailing_icon(mut self, icon: impl Into<String>) -> Self {
        self.trailing_icon = Some(icon.into());
        self
    }

    /// Set a handler for tapping the leading icon.
    pub fn on_leading_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_leading_tap = Some(Box::new(handler));
        self
    }

    /// Set a handler for tapping the trailing icon.
    pub fn on_trailing_tap(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_trailing_tap = Some(Box::new(handler));
        self
    }

    /// Add a suggestion item with a leading icon, text, and click handler.
    pub fn suggestion(
        mut self,
        icon: impl Into<String>,
        text: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.suggestions.push(SearchSuggestion {
            icon: icon.into(),
            text: text.into(),
            on_click: Box::new(on_click),
        });
        self
    }

    /// Add a child element to the body area (rendered below suggestions).
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.body_children.push(child.into_any_element());
        self
    }

    /// Set a custom element ID.
    pub fn id(mut self, id: impl Into<gpui::ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl IntoElement for SearchView {
    type Element = <Stateful<gpui::Div> as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;

        let container_bg = color(t.surface_container_high);
        let on_surface = color(t.on_surface);
        let on_surface_variant = color(t.on_surface_variant);
        let divider_color = color(t.outline_variant);

        let elem_id = self
            .id
            .unwrap_or_else(|| gpui::ElementId::Name("search-view".into()));

        // ── Main container ───────────────────────────────────────────────

        let mut view = div()
            .id(elem_id)
            .flex()
            .flex_col()
            .w_full()
            .bg(container_bg);

        // ── Input row ────────────────────────────────────────────────────

        let mut input_row = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(SEARCH_BAR_HEIGHT))
            .px(px(SEARCH_BAR_PADDING_H))
            .gap(px(ICON_TEXT_GAP));

        // Leading icon
        if let Some(icon) = self.leading_icon {
            if let Some(handler) = self.on_leading_tap {
                let icon_el = div()
                    .id("search-view-leading")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface)
                    .flex_shrink_0()
                    .cursor_pointer()
                    .child(icon)
                    .on_mouse_down(MouseButton::Left, handler);
                input_row = input_row.child(icon_el);
            } else {
                let icon_el = div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface)
                    .flex_shrink_0()
                    .child(icon);
                input_row = input_row.child(icon_el);
            }
        }

        // Query / placeholder text
        let text_el = if let Some(ref query_text) = self.query {
            div()
                .flex_1()
                .text_base()
                .line_height(px(24.0))
                .text_color(on_surface)
                .overflow_hidden()
                .child(query_text.clone())
        } else {
            let placeholder_text = self.placeholder.as_deref().unwrap_or("Search").to_string();
            div()
                .flex_1()
                .text_base()
                .line_height(px(24.0))
                .text_color(on_surface_variant)
                .overflow_hidden()
                .child(placeholder_text)
        };

        input_row = input_row.child(text_el);

        // Trailing icon (clear)
        if let Some(icon) = self.trailing_icon {
            if let Some(handler) = self.on_trailing_tap {
                let icon_el = div()
                    .id("search-view-trailing")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface_variant)
                    .flex_shrink_0()
                    .cursor_pointer()
                    .child(icon)
                    .on_mouse_down(MouseButton::Left, handler);
                input_row = input_row.child(icon_el);
            } else {
                let icon_el = div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(ICON_SIZE))
                    .h(px(ICON_SIZE))
                    .rounded(px(ICON_SIZE / 2.0))
                    .text_lg()
                    .text_color(on_surface_variant)
                    .flex_shrink_0()
                    .child(icon);
                input_row = input_row.child(icon_el);
            }
        }

        view = view.child(input_row);

        // ── Divider ──────────────────────────────────────────────────────

        view = view.child(div().w_full().h(px(DIVIDER_HEIGHT)).bg(divider_color));

        // ── Suggestions list ─────────────────────────────────────────────

        for (i, suggestion) in self.suggestions.into_iter().enumerate() {
            let row = div()
                .id(gpui::ElementId::Name(
                    format!("search-suggestion-{i}").into(),
                ))
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .px(px(SUGGESTION_PADDING_H))
                .py(px(SUGGESTION_PADDING_V))
                .gap(px(SUGGESTION_GAP))
                .cursor_pointer()
                .child(
                    div()
                        .text_lg()
                        .text_color(on_surface_variant)
                        .flex_shrink_0()
                        .child(suggestion.icon),
                )
                .child(
                    div()
                        .flex_1()
                        .text_base()
                        .line_height(px(24.0))
                        .text_color(on_surface)
                        .child(suggestion.text),
                )
                .on_mouse_down(MouseButton::Left, suggestion.on_click);

            view = view.child(row);
        }

        // ── Body children ────────────────────────────────────────────────

        for child in self.body_children {
            view = view.child(child);
        }

        view.into_element()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Demo / Showcase
// ═══════════════════════════════════════════════════════════════════════════════

/// Render a static (non-interactive) demo of search bar and search view
/// variants for the component showcase gallery.
pub fn search_bar_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);
    let label_color = color(theme.on_surface_variant);

    div()
        .flex()
        .flex_col()
        .gap_6()
        .w_full()
        .p_4()
        // ── Search bar (empty) ───────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Search Bar — empty"),
                )
                .child(
                    SearchBar::new(theme)
                        .placeholder("Hinted search text")
                        .leading_icon("🔍")
                        .trailing_icon("🎤"),
                ),
        )
        // ── Search bar (with query) ──────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Search Bar — with query"),
                )
                .child(
                    SearchBar::new(theme)
                        .query("Material Design")
                        .leading_icon("←")
                        .trailing_icon("✕"),
                ),
        )
        // ── Search bar (elevated) ────────────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Search Bar — elevated"),
                )
                .child(
                    SearchBar::new(theme)
                        .placeholder("Search files")
                        .leading_icon("☰")
                        .trailing_icon("🎤")
                        .trailing_icon2("👤")
                        .elevated(true),
                ),
        )
        // ── Search view with suggestions ─────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Search View — with suggestions"),
                )
                .child(
                    div()
                        .rounded(px(SEARCH_BAR_RADIUS))
                        .overflow_hidden()
                        .child(
                            SearchView::new(theme)
                                .query("flutter")
                                .leading_icon("←")
                                .trailing_icon("✕")
                                .suggestion("🕐", "flutter widgets", |_, _, _| {})
                                .suggestion("🔍", "flutter animation", |_, _, _| {})
                                .suggestion("🔍", "flutter material 3", |_, _, _| {})
                                .suggestion("🕐", "flutter state management", |_, _, _| {}),
                        ),
                ),
        )
        // ── Search view with placeholder ─────────────────────────────────
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(label_color)
                        .child("Search View — empty with placeholder"),
                )
                .child(
                    div()
                        .rounded(px(SEARCH_BAR_RADIUS))
                        .overflow_hidden()
                        .child(
                            SearchView::new(theme)
                                .placeholder("Search messages")
                                .leading_icon("←")
                                .suggestion("🕐", "recent search 1", |_, _, _| {})
                                .suggestion("🕐", "recent search 2", |_, _, _| {}),
                        ),
                ),
        )
}
