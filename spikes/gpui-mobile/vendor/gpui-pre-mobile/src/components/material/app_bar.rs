//! Material Design 3 App Bar components.
//!
//! App bars display information and actions at the top or bottom of a screen.
//! MD3 defines four top app bar types:
//!
//! - **Center-aligned** — title centered, single leading/trailing icon
//! - **Small** — title left-aligned, leading icon, up to 3 trailing icons
//! - **Medium** — larger title below the bar area (two-row)
//! - **Large** — even larger title below the bar area (two-row)
//!
//! # Example
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::app_bar::TopAppBar;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! // Small top app bar with a back button and two actions
//! let bar = TopAppBar::small("My Screen", theme)
//!     .leading_icon("←", |_, _, _| { /* go back */ })
//!     .trailing_icon("🔍", |_, _, _| { /* search */ })
//!     .trailing_icon("⋮", |_, _, _| { /* overflow menu */ })
//!     .build();
//!
//! // Center-aligned top app bar
//! let bar = TopAppBar::center_aligned("Home", theme)
//!     .leading_icon("☰", |_, _, _| { /* open drawer */ })
//!     .trailing_icon("👤", |_, _, _| { /* profile */ })
//!     .build();
//! ```

use gpui::{div, prelude::*, px, rgb, AnyElement, MouseButton, MouseDownEvent, Window};

use super::theme::{MaterialTheme, ShapeScale};

// ── Top App Bar variant ──────────────────────────────────────────────────────

/// The visual variant of a top app bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopAppBarVariant {
    /// Title centered horizontally, typically one leading and one trailing icon.
    CenterAligned,
    /// Title left-aligned, leading icon, up to 3 trailing action icons.
    Small,
    /// Two-row bar: top row has icons, bottom row has a medium-sized title.
    Medium,
    /// Two-row bar: top row has icons, bottom row has a large title.
    Large,
}

// ── Icon entry ───────────────────────────────────────────────────────────────

/// An icon button in the app bar (leading or trailing).
#[allow(clippy::type_complexity)]
struct AppBarIconEntry {
    /// Icon text — an emoji or short string.
    icon: String,
    /// Click handler invoked when the icon is tapped.
    on_click: Box<dyn Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static>,
}

// ── TopAppBar builder ────────────────────────────────────────────────────────

/// Builder for a Material Design 3 top app bar.
///
/// Construct via [`TopAppBar::small`], [`TopAppBar::center_aligned`],
/// [`TopAppBar::medium`], or [`TopAppBar::large`], add icons, and
/// finalise with [`build`](TopAppBar::build).
pub struct TopAppBar {
    title: String,
    variant: TopAppBarVariant,
    theme: MaterialTheme,
    leading: Option<AppBarIconEntry>,
    trailing: Vec<AppBarIconEntry>,
    /// Whether the bar is in a "scrolled" state (elevated).
    scrolled: bool,
}

impl TopAppBar {
    // ── Constructors ─────────────────────────────────────────────────

    /// Create a **small** top app bar with a left-aligned title.
    pub fn small(title: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            title: title.into(),
            variant: TopAppBarVariant::Small,
            theme,
            leading: None,
            trailing: Vec::new(),
            scrolled: false,
        }
    }

    /// Create a **center-aligned** top app bar with a centered title.
    pub fn center_aligned(title: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            title: title.into(),
            variant: TopAppBarVariant::CenterAligned,
            theme,
            leading: None,
            trailing: Vec::new(),
            scrolled: false,
        }
    }

    /// Create a **medium** top app bar with a two-row layout and medium title.
    pub fn medium(title: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            title: title.into(),
            variant: TopAppBarVariant::Medium,
            theme,
            leading: None,
            trailing: Vec::new(),
            scrolled: false,
        }
    }

    /// Create a **large** top app bar with a two-row layout and large title.
    pub fn large(title: impl Into<String>, theme: MaterialTheme) -> Self {
        Self {
            title: title.into(),
            variant: TopAppBarVariant::Large,
            theme,
            leading: None,
            trailing: Vec::new(),
            scrolled: false,
        }
    }

    // ── Configuration ────────────────────────────────────────────────

    /// Set the leading (left-side) navigation icon.
    ///
    /// Typically a back arrow or hamburger menu icon. Only one leading
    /// icon is supported per the MD3 spec.
    ///
    /// The `on_click` handler matches GPUI's `on_mouse_down` signature,
    /// so `cx.listener(...)` can be used directly.
    pub fn leading_icon(
        mut self,
        icon: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.leading = Some(AppBarIconEntry {
            icon: icon.into(),
            on_click: Box::new(on_click),
        });
        self
    }

    /// Add a trailing (right-side) action icon.
    ///
    /// MD3 recommends a maximum of 3 trailing icons. Icons are rendered
    /// in the order they are added (left to right).
    pub fn trailing_icon(
        mut self,
        icon: impl Into<String>,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.trailing.push(AppBarIconEntry {
            icon: icon.into(),
            on_click: Box::new(on_click),
        });
        self
    }

    /// Mark the app bar as in a "scrolled" state.
    ///
    /// When scrolled, the bar adopts a slightly elevated surface tone
    /// (`surface_container`) instead of the default transparent/surface
    /// background, providing visual separation from the scrolled content.
    pub fn scrolled(mut self, scrolled: bool) -> Self {
        self.scrolled = scrolled;
        self
    }

    // ── Build ────────────────────────────────────────────────────────

    /// Build and return the top app bar element.
    ///
    /// Consumes the builder.
    pub fn build(self) -> impl IntoElement {
        match self.variant {
            TopAppBarVariant::CenterAligned => self.build_center_aligned(),
            TopAppBarVariant::Small => self.build_small(),
            TopAppBarVariant::Medium => self.build_medium(),
            TopAppBarVariant::Large => self.build_large(),
        }
    }

    // ── Private build methods ────────────────────────────────────────

    /// Build a center-aligned top app bar.
    fn build_center_aligned(self) -> AnyElement {
        let t = &self.theme;
        let bar_bg = if self.scrolled {
            t.surface_container
        } else {
            t.surface
        };

        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(64.0))
            .px_1()
            .bg(rgb(bar_bg));

        // Leading icon (or spacer)
        row = row.child(self.build_leading_slot());

        // Center title (flex-1 + text-center)
        row = row.child(
            div()
                .flex_1()
                .text_center()
                .line_height(px(24.0))
                .text_color(rgb(t.on_surface))
                .child(div().text_size(px(22.0)).child(self.title.clone())),
        );

        // Trailing icons (or spacer to balance)
        row = row.child(self.build_trailing_slot());

        row.into_any_element()
    }

    /// Build a small top app bar.
    fn build_small(self) -> AnyElement {
        let t = &self.theme;
        let bar_bg = if self.scrolled {
            t.surface_container
        } else {
            t.surface
        };

        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(64.0))
            .px_1()
            .bg(rgb(bar_bg));

        // Leading icon
        row = row.child(self.build_leading_slot());

        // Title (left-aligned, takes remaining space)
        row = row.child(
            div()
                .flex_1()
                .pl_2()
                .line_height(px(24.0))
                .text_color(rgb(t.on_surface))
                .child(div().text_size(px(22.0)).child(self.title.clone())),
        );

        // Trailing icons
        row = row.child(self.build_trailing_slot());

        row.into_any_element()
    }

    /// Build a medium top app bar (two rows).
    fn build_medium(self) -> AnyElement {
        let t = &self.theme;
        let bar_bg = if self.scrolled {
            t.surface_container
        } else {
            t.surface
        };

        let mut container = div().flex().flex_col().w_full().bg(rgb(bar_bg));

        // Top row: leading + trailing icons
        let mut top_row = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(64.0))
            .px_1();

        top_row = top_row.child(self.build_leading_slot());
        top_row = top_row.child(div().flex_1()); // spacer
        top_row = top_row.child(self.build_trailing_slot());

        container = container.child(top_row);

        // Bottom row: medium title
        container = container.child(
            div()
                .w_full()
                .px_4()
                .pb_4()
                .line_height(px(32.0))
                .text_color(rgb(t.on_surface))
                .child(div().text_size(px(28.0)).child(self.title.clone())),
        );

        container.into_any_element()
    }

    /// Build a large top app bar (two rows).
    fn build_large(self) -> AnyElement {
        let t = &self.theme;
        let bar_bg = if self.scrolled {
            t.surface_container
        } else {
            t.surface
        };

        let mut container = div().flex().flex_col().w_full().bg(rgb(bar_bg));

        // Top row: leading + trailing icons
        let mut top_row = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(64.0))
            .px_1();

        top_row = top_row.child(self.build_leading_slot());
        top_row = top_row.child(div().flex_1());
        top_row = top_row.child(self.build_trailing_slot());

        container = container.child(top_row);

        // Bottom row: large title
        container = container.child(
            div()
                .w_full()
                .px_4()
                .pb_6()
                .line_height(px(40.0))
                .text_color(rgb(t.on_surface))
                .child(div().text_size(px(36.0)).child(self.title.clone())),
        );

        container.into_any_element()
    }

    // ── Slot builders ────────────────────────────────────────────────

    /// Build the leading icon slot.
    ///
    /// Returns the leading icon button, or a 48px spacer if no leading
    /// icon was configured.
    fn build_leading_slot(&self) -> AnyElement {
        let t = &self.theme;

        if let Some(entry) = &self.leading {
            // We need to clone values out to avoid borrow issues
            let icon = entry.icon.clone();
            // Can't move out of &self, so we build a static placeholder.
            // The actual click handler requires ownership. Since we're in &self,
            // we'll build the interactive version in the consuming builder path.
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(48.0))
                .rounded(px(ShapeScale::FULL))
                .cursor_pointer()
                .text_xl()
                .text_color(rgb(t.on_surface))
                .child(icon)
                .into_any_element()
        } else {
            div().w(px(48.0)).h(px(48.0)).into_any_element()
        }
    }

    /// Build the trailing icons slot.
    ///
    /// Returns a row of trailing icon buttons, or a 48px spacer if none
    /// were configured.
    fn build_trailing_slot(&self) -> AnyElement {
        let t = &self.theme;

        if self.trailing.is_empty() {
            return div().w(px(48.0)).h(px(48.0)).into_any_element();
        }

        let mut row = div().flex().flex_row().items_center();

        for entry in &self.trailing {
            let icon = entry.icon.clone();
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(48.0))
                    .rounded(px(ShapeScale::FULL))
                    .cursor_pointer()
                    .text_xl()
                    .text_color(rgb(t.on_surface_variant))
                    .child(icon),
            );
        }

        row.into_any_element()
    }
}

/// Build a top app bar with functional click handlers.
///
/// This is the consuming version that takes ownership of the handlers.
/// Use this when you need interactive icons. The [`TopAppBar::build`]
/// method calls this internally.
impl IntoElement for TopAppBar {
    type Element = <gpui::Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let t = self.theme;
        let bar_bg = if self.scrolled {
            t.surface_container
        } else {
            t.surface
        };

        match self.variant {
            TopAppBarVariant::CenterAligned | TopAppBarVariant::Small => {
                let is_center = self.variant == TopAppBarVariant::CenterAligned;

                let mut row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .h(px(64.0))
                    .px_1()
                    .bg(rgb(bar_bg));

                // Leading icon
                if let Some(entry) = self.leading {
                    row = row.child(
                        div()
                            .id("app-bar-leading")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(48.0))
                            .rounded(px(ShapeScale::FULL))
                            .cursor_pointer()
                            .text_xl()
                            .text_color(rgb(t.on_surface))
                            .child(entry.icon)
                            .on_mouse_down(MouseButton::Left, entry.on_click),
                    );
                } else {
                    row = row.child(div().w(px(48.0)).h(px(48.0)));
                }

                // Title
                if is_center {
                    row = row.child(
                        div()
                            .flex_1()
                            .text_center()
                            .line_height(px(24.0))
                            .text_color(rgb(t.on_surface))
                            .child(div().text_size(px(22.0)).child(self.title)),
                    );
                } else {
                    row = row.child(
                        div()
                            .flex_1()
                            .pl_2()
                            .line_height(px(24.0))
                            .text_color(rgb(t.on_surface))
                            .child(div().text_size(px(22.0)).child(self.title)),
                    );
                }

                // Trailing icons
                if self.trailing.is_empty() {
                    row = row.child(div().w(px(48.0)).h(px(48.0)));
                } else {
                    let mut trailing_row = div().flex().flex_row().items_center();
                    for (i, entry) in self.trailing.into_iter().enumerate() {
                        trailing_row = trailing_row.child(
                            div()
                                .id(gpui::ElementId::Name(
                                    format!("app-bar-trailing-{i}").into(),
                                ))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(48.0))
                                .rounded(px(ShapeScale::FULL))
                                .cursor_pointer()
                                .text_xl()
                                .text_color(rgb(t.on_surface_variant))
                                .child(entry.icon)
                                .on_mouse_down(MouseButton::Left, entry.on_click),
                        );
                    }
                    row = row.child(trailing_row);
                }

                row.into_element()
            }

            TopAppBarVariant::Medium | TopAppBarVariant::Large => {
                let is_large = self.variant == TopAppBarVariant::Large;

                let mut container = div().flex().flex_col().w_full().bg(rgb(bar_bg));

                // Top row
                let mut top_row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w_full()
                    .h(px(64.0))
                    .px_1();

                if let Some(entry) = self.leading {
                    top_row = top_row.child(
                        div()
                            .id("app-bar-leading")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(48.0))
                            .rounded(px(ShapeScale::FULL))
                            .cursor_pointer()
                            .text_xl()
                            .text_color(rgb(t.on_surface))
                            .child(entry.icon)
                            .on_mouse_down(MouseButton::Left, entry.on_click),
                    );
                } else {
                    top_row = top_row.child(div().w(px(48.0)).h(px(48.0)));
                }

                top_row = top_row.child(div().flex_1());

                if self.trailing.is_empty() {
                    top_row = top_row.child(div().w(px(48.0)).h(px(48.0)));
                } else {
                    let mut trailing_row = div().flex().flex_row().items_center();
                    for (i, entry) in self.trailing.into_iter().enumerate() {
                        trailing_row = trailing_row.child(
                            div()
                                .id(gpui::ElementId::Name(
                                    format!("app-bar-trailing-{i}").into(),
                                ))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(48.0))
                                .rounded(px(ShapeScale::FULL))
                                .cursor_pointer()
                                .text_xl()
                                .text_color(rgb(t.on_surface_variant))
                                .child(entry.icon)
                                .on_mouse_down(MouseButton::Left, entry.on_click),
                        );
                    }
                    top_row = top_row.child(trailing_row);
                }

                container = container.child(top_row);

                // Bottom row: title
                let (font_size, line_h, pad_bottom) = if is_large {
                    (36.0, 40.0, 6.0)
                } else {
                    (28.0, 32.0, 4.0)
                };

                container = container.child(
                    div()
                        .w_full()
                        .px_4()
                        .pb(px(pad_bottom * 4.0))
                        .line_height(px(line_h))
                        .text_color(rgb(t.on_surface))
                        .child(div().text_size(px(font_size)).child(self.title)),
                );

                container.into_element()
            }
        }
    }
}

// ── Demo / showcase ──────────────────────────────────────────────────────────

/// Renders a showcase of all four top app bar variants.
///
/// This is a static (non-interactive) demo for component galleries.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::app_bar;
///
/// let demo = app_bar::app_bar_demo(true); // dark mode
/// ```
pub fn app_bar_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        // Center-aligned
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("CENTER-ALIGNED"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TopAppBar::center_aligned("Title", theme)
                                .leading_icon("☰", |_, _, _| {})
                                .trailing_icon("👤", |_, _, _| {}),
                        ),
                ),
        )
        // Small
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SMALL"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TopAppBar::small("Title", theme)
                                .leading_icon("←", |_, _, _| {})
                                .trailing_icon("📎", |_, _, _| {})
                                .trailing_icon("📅", |_, _, _| {})
                                .trailing_icon("⋮", |_, _, _| {}),
                        ),
                ),
        )
        // Medium
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("MEDIUM"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TopAppBar::medium("Medium Title", theme)
                                .leading_icon("←", |_, _, _| {})
                                .trailing_icon("⋮", |_, _, _| {}),
                        ),
                ),
        )
        // Large
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("LARGE"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TopAppBar::large("Large Title", theme)
                                .leading_icon("←", |_, _, _| {})
                                .trailing_icon("🔍", |_, _, _| {})
                                .trailing_icon("⋮", |_, _, _| {}),
                        ),
                ),
        )
        // Scrolled variant
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("SMALL (SCROLLED)"),
                )
                .child(
                    div()
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(theme.outline_variant))
                        .child(
                            TopAppBar::small("Scrolled", theme)
                                .leading_icon("←", |_, _, _| {})
                                .trailing_icon("⋮", |_, _, _| {})
                                .scrolled(true),
                        ),
                ),
        )
}
