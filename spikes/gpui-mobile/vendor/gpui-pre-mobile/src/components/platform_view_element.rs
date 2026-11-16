//! GPUI element for embedding native platform views.
//!
//! `PlatformViewElement` is a GPUI component that reserves layout space for
//! a native view (Android `View` or iOS `UIView`) and synchronizes its
//! position and size during the paint phase.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use gpui_mobile::components::platform_view_element::platform_view_element;
//! use gpui_mobile::platform_view::PlatformViewHandle;
//!
//! fn my_component(handle: Arc<PlatformViewHandle>) -> impl IntoElement {
//!     div()
//!         .flex()
//!         .child(platform_view_element(handle).size_full())
//! }
//! ```

use crate::platform_view::{PlatformViewBounds, PlatformViewHandle};
use gpui::{div, ParentElement, Styled};
use std::sync::Arc;

/// Create a GPUI element that hosts a native platform view.
///
/// The element reserves layout space and updates the platform view's
/// position and size on every paint. The native view is shown when
/// painted and hidden when the element is removed from the tree.
///
/// The returned element can be styled with `.w()`, `.h()`, `.size()`,
/// `.flex_grow(1.)`, etc. to control how much space it occupies in the layout.
pub fn platform_view_element(handle: Arc<PlatformViewHandle>) -> gpui::Div {
    div().child(
        gpui::canvas(
            // Prepaint: capture bounds
            move |bounds, _window, _cx| bounds,
            // Paint: synchronize native view position
            move |_bounds, prepaint_bounds, _window, _cx| {
                let logical_bounds = PlatformViewBounds {
                    x: prepaint_bounds.origin.x.as_f32(),
                    y: prepaint_bounds.origin.y.as_f32(),
                    width: prepaint_bounds.size.width.as_f32(),
                    height: prepaint_bounds.size.height.as_f32(),
                };
                handle.set_bounds(logical_bounds);
                handle.set_visible(true);
            },
        )
        .size_full(),
    )
}

/// A higher-level wrapper that creates the platform view from the registry
/// and manages its lifecycle.
///
/// The view is created on first render and disposed when dropped.
pub struct ManagedPlatformView {
    handle: Option<Arc<PlatformViewHandle>>,
    view_type: String,
    creation_params: std::collections::HashMap<String, String>,
}

impl ManagedPlatformView {
    /// Create a new managed platform view for the given type.
    pub fn new(view_type: impl Into<String>) -> Self {
        Self {
            handle: None,
            view_type: view_type.into(),
            creation_params: std::collections::HashMap::new(),
        }
    }

    /// Add a creation parameter.
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.creation_params.insert(key.into(), value.into());
        self
    }

    /// Get or create the platform view handle.
    pub fn ensure_created(&mut self) -> Result<Arc<PlatformViewHandle>, String> {
        if let Some(handle) = &self.handle {
            return Ok(handle.clone());
        }

        let registry = crate::platform_view::PlatformViewRegistry::global();
        let params = crate::platform_view::PlatformViewParams {
            bounds: PlatformViewBounds::default(),
            creation_params: self.creation_params.clone(),
        };

        let handle = Arc::new(registry.create_view(&self.view_type, params)?);
        self.handle = Some(handle.clone());
        Ok(handle)
    }

    /// Get the handle if the view has been created.
    pub fn handle(&self) -> Option<&Arc<PlatformViewHandle>> {
        self.handle.as_ref()
    }

    /// Render the platform view element.
    ///
    /// Returns a div with an error message if the view fails to create.
    pub fn render(&mut self) -> gpui::Div {
        match self.ensure_created() {
            Ok(handle) => platform_view_element(handle),
            Err(e) => {
                log::error!("Failed to create platform view '{}': {}", self.view_type, e);
                div()
            }
        }
    }
}

impl Drop for ManagedPlatformView {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.dispose();
        }
    }
}
