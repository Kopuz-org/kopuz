//! Material Design 3 Scaffold component.
//!
//! A Scaffold provides the basic Material Design visual layout structure.
//! It combines a top app bar, body content, an optional bottom bar,
//! an optional FAB, and optional drawer/sheet overlays into a single
//! cohesive layout.
//!
//! # Example
//!
//! ```rust,ignore
//! use gpui_mobile::components::material::scaffold::Scaffold;
//! use gpui_mobile::components::material::app_bar::TopAppBar;
//! use gpui_mobile::components::material::navigation_bar::NavigationBarBuilder;
//! use gpui_mobile::components::material::theme::MaterialTheme;
//!
//! let theme = MaterialTheme::dark();
//!
//! Scaffold::new(theme)
//!     .top_bar(TopAppBar::small("My App", theme).build())
//!     .body(my_content)
//!     .bottom_bar(
//!         NavigationBarBuilder::new(theme.is_dark)
//!             .item("🏠", "Home", true, |_, _, _| {})
//!             .build(),
//!     )
//!     .floating_action_button(my_fab)
//! ```

use gpui::{div, prelude::*, px, rgb, AnyElement};

use super::theme::MaterialTheme;

// ── Scaffold ─────────────────────────────────────────────────────────────────

/// A Material Design 3 Scaffold — the top-level visual layout structure.
///
/// The Scaffold provides slots for:
/// - **top_bar** — typically a [`TopAppBar`](super::app_bar::TopAppBar)
/// - **body** — the main scrollable content area
/// - **bottom_bar** — typically a [`NavigationBar`](super::navigation_bar::NavigationBarBuilder)
///   or [`BottomAppBar`](super::bottom_app_bar::BottomAppBar)
/// - **floating_action_button** — an optional FAB positioned above the bottom bar
/// - **drawer** — an optional side navigation drawer overlay
/// - **snackbar** — an optional snackbar message shown above the bottom bar
///
/// The Scaffold does **not** own any state; it is a purely layout-driven
/// component. State management (current screen, drawer open/closed, etc.)
/// is the responsibility of the parent view.
pub struct Scaffold {
    theme: MaterialTheme,
    top_bar: Option<AnyElement>,
    body: Option<AnyElement>,
    bottom_bar: Option<AnyElement>,
    fab: Option<AnyElement>,
    drawer: Option<AnyElement>,
    snackbar_element: Option<AnyElement>,
    banner: Option<AnyElement>,
    body_scrollable: bool,
    safe_area_top: f32,
    safe_area_bottom: f32,
}

impl Scaffold {
    /// Create a new Scaffold with the given theme.
    pub fn new(theme: MaterialTheme) -> Self {
        Self {
            theme,
            top_bar: None,
            body: None,
            bottom_bar: None,
            fab: None,
            drawer: None,
            snackbar_element: None,
            banner: None,
            body_scrollable: true,
            safe_area_top: 0.0,
            safe_area_bottom: 0.0,
        }
    }

    /// Set the top app bar element.
    ///
    /// Typically built with [`TopAppBar`](super::app_bar::TopAppBar).
    pub fn top_bar(mut self, element: impl IntoElement) -> Self {
        self.top_bar = Some(element.into_any_element());
        self
    }

    /// Set the main body content.
    ///
    /// By default the body is placed in a scrollable container. Use
    /// [`body_scrollable(false)`](Self::body_scrollable) to disable scrolling.
    pub fn body(mut self, element: impl IntoElement) -> Self {
        self.body = Some(element.into_any_element());
        self
    }

    /// Set the bottom bar element.
    ///
    /// Typically built with [`NavigationBarBuilder`](super::navigation_bar::NavigationBarBuilder)
    /// or [`BottomAppBar`](super::bottom_app_bar::BottomAppBar).
    pub fn bottom_bar(mut self, element: impl IntoElement) -> Self {
        self.bottom_bar = Some(element.into_any_element());
        self
    }

    /// Set the floating action button element.
    ///
    /// The FAB is positioned in the bottom-right corner of the body area,
    /// above the bottom bar.
    pub fn floating_action_button(mut self, element: impl IntoElement) -> Self {
        self.fab = Some(element.into_any_element());
        self
    }

    /// Set a navigation drawer overlay element.
    ///
    /// When present, the drawer is rendered on top of the scaffold with a
    /// scrim overlay behind it. The caller is responsible for toggling
    /// visibility and handling the scrim tap to close.
    pub fn drawer(mut self, element: impl IntoElement) -> Self {
        self.drawer = Some(element.into_any_element());
        self
    }

    /// Set a snackbar element to display above the bottom bar.
    pub fn snackbar(mut self, element: impl IntoElement) -> Self {
        self.snackbar_element = Some(element.into_any_element());
        self
    }

    /// Set a Material banner element to display below the top bar.
    pub fn banner_element(mut self, element: impl IntoElement) -> Self {
        self.banner = Some(element.into_any_element());
        self
    }

    /// Control whether the body content area is scrollable.
    ///
    /// Defaults to `true`. Set to `false` if the body manages its own
    /// scrolling (e.g. a list view) or if the content is guaranteed to fit.
    pub fn body_scrollable(mut self, scrollable: bool) -> Self {
        self.body_scrollable = scrollable;
        self
    }

    /// Set the top safe area inset in logical pixels.
    ///
    /// This adds a spacer above the top bar to account for the device's
    /// status bar, notch, or dynamic island.
    pub fn safe_area_top(mut self, px: f32) -> Self {
        self.safe_area_top = px;
        self
    }

    /// Set the bottom safe area inset in logical pixels.
    ///
    /// This adds a spacer below the bottom bar to account for the device's
    /// home indicator or gesture bar.
    pub fn safe_area_bottom(mut self, px_val: f32) -> Self {
        self.safe_area_bottom = px_val;
        self
    }
}

impl IntoElement for Scaffold {
    type Element = <gpui::Div as IntoElement>::Element;

    fn into_element(self) -> Self::Element {
        let bg = self.theme.surface;
        let safe_bg = self.theme.surface_container;

        let mut root = div().flex().flex_col().size_full().bg(rgb(bg));

        // ── Top safe area spacer ─────────────────────────────────────
        if self.safe_area_top > 0.0 {
            root = root.child(div().w_full().h(px(self.safe_area_top)).bg(rgb(safe_bg)));
        }

        // ── Top app bar ──────────────────────────────────────────────
        if let Some(top_bar) = self.top_bar {
            root = root.child(top_bar);
        }

        // ── Banner (below top bar, above body) ───────────────────────
        if let Some(banner) = self.banner {
            root = root.child(banner);
        }

        // ── Body content area (flex-1, optionally scrollable) ────────
        {
            let body_content = self.body.unwrap_or_else(|| div().into_any_element());
            let has_fab = self.fab.is_some();

            if self.body_scrollable {
                // Wrap in a scrollable container with relative positioning
                // so we can layer the FAB on top.
                let mut body_wrapper = div()
                    .id("scaffold-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .relative();

                body_wrapper = body_wrapper.child(body_content);

                // ── FAB (overlaid in bottom-right of body area) ──────
                if let Some(fab) = self.fab {
                    body_wrapper = body_wrapper
                        .child(div().absolute().bottom(px(16.0)).right(px(16.0)).child(fab));
                }

                // ── Snackbar (overlaid above bottom of body area) ────
                if let Some(snack) = self.snackbar_element {
                    body_wrapper = body_wrapper.child(
                        div()
                            .absolute()
                            .bottom(px(if has_fab { 80.0 } else { 16.0 }))
                            .left(px(16.0))
                            .right(px(16.0))
                            .child(snack),
                    );
                }

                root = root.child(body_wrapper);
            } else {
                // Non-scrollable: use a simple flex-1 container
                let mut body_wrapper = div().id("scaffold-body-ns").flex_1().relative();
                body_wrapper = body_wrapper.child(body_content);

                if let Some(fab) = self.fab {
                    body_wrapper = body_wrapper
                        .child(div().absolute().bottom(px(16.0)).right(px(16.0)).child(fab));
                }

                if let Some(snack) = self.snackbar_element {
                    body_wrapper = body_wrapper.child(
                        div()
                            .absolute()
                            .bottom(px(80.0))
                            .left(px(16.0))
                            .right(px(16.0))
                            .child(snack),
                    );
                }

                root = root.child(body_wrapper);
            }
        }

        // ── Bottom bar ───────────────────────────────────────────────
        if let Some(bottom_bar) = self.bottom_bar {
            root = root.child(bottom_bar);
        }

        // ── Bottom safe area spacer ──────────────────────────────────
        if self.safe_area_bottom > 0.0 {
            root = root.child(div().w_full().h(px(self.safe_area_bottom)).bg(rgb(safe_bg)));
        }

        // ── Drawer overlay ───────────────────────────────────────────
        if let Some(drawer_el) = self.drawer {
            root = root.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    // Scrim overlay
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.32)),
                    )
                    // Drawer content
                    .child(drawer_el),
            );
        }

        root.into_element()
    }
}

// ── Demo / showcase ──────────────────────────────────────────────────────────

/// Renders a static demo of the Scaffold layout for showcase purposes.
///
/// This shows the scaffold structure with a mock top bar, body content,
/// bottom bar, and a FAB.
///
/// # Example
///
/// ```rust,ignore
/// use gpui_mobile::components::material::scaffold;
///
/// let demo = scaffold::scaffold_demo(true); // dark mode
/// ```
pub fn scaffold_demo(dark: bool) -> impl IntoElement {
    let theme = MaterialTheme::from_appearance(dark);

    let mock_top_bar = div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .px_4()
        .py_3()
        .bg(rgb(theme.surface_container))
        .child(
            div()
                .text_lg()
                .text_color(rgb(theme.on_surface))
                .child("Scaffold Demo"),
        );

    let mock_body = div()
        .flex()
        .flex_col()
        .gap_4()
        .p_4()
        .child(
            div()
                .text_color(rgb(theme.on_surface))
                .child("This is the scaffold body content area."),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(theme.on_surface_variant))
                .child("The scaffold provides top bar, body, bottom bar, FAB, and drawer slots."),
        )
        .child(
            div()
                .p_4()
                .rounded(px(12.0))
                .bg(rgb(theme.surface_container))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(theme.on_surface_variant))
                        .child("Content cards, lists, and other elements go here."),
                ),
        );

    let mock_fab = div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(56.0))
        .rounded(px(16.0))
        .bg(rgb(theme.primary_container))
        .text_2xl()
        .text_color(rgb(theme.on_primary_container))
        .child("+");

    let mock_bottom_bar = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_around()
        .w_full()
        .py_3()
        .bg(rgb(theme.surface_container))
        .child(
            div()
                .text_color(rgb(theme.on_surface))
                .text_sm()
                .child("🏠 Home"),
        )
        .child(
            div()
                .text_color(rgb(theme.on_surface_variant))
                .text_sm()
                .child("🔍 Search"),
        )
        .child(
            div()
                .text_color(rgb(theme.on_surface_variant))
                .text_sm()
                .child("👤 Profile"),
        );

    // For the demo, we render a contained version (not full-screen)
    div()
        .flex()
        .flex_col()
        .h(px(400.0))
        .w_full()
        .rounded(px(12.0))
        .overflow_hidden()
        .border_1()
        .border_color(rgb(theme.outline_variant))
        .child(
            Scaffold::new(theme)
                .top_bar(mock_top_bar)
                .body(mock_body)
                .bottom_bar(mock_bottom_bar)
                .floating_action_button(mock_fab)
                .body_scrollable(false),
        )
}
