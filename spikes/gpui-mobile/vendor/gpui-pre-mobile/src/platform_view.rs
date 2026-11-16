//! Platform Views — embedding native views in the GPUI render tree.
//!
//! Platform views allow native UI components (video players, maps, camera
//! previews, web views) to be embedded alongside GPUI-rendered content.
//!
//! ## Architecture
//!
//! This follows Flutter's platform view approach:
//!
//! 1. **`PlatformView`** — trait representing a live native view instance.
//! 2. **`PlatformViewFactory`** — creates `PlatformView` instances by type.
//! 3. **`PlatformViewRegistry`** — global registry of view factories.
//! 4. **`PlatformViewHandle`** — opaque handle for managing a platform view.
//!
//! ## Composition Modes
//!
//! - **Hybrid Composition** (default): The native view is placed in the
//!   platform's view hierarchy and positioned to match the GPUI element's
//!   screen coordinates. Best compatibility, slight performance overhead.
//!
//! - **Texture-based** (future): The native view renders to an offscreen
//!   texture that GPUI composites. Better performance but some input and
//!   accessibility trade-offs.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use gpui_mobile::platform_view::{PlatformViewRegistry, PlatformViewParams};
//!
//! // Register a factory (typically in package init)
//! PlatformViewRegistry::global().register("video_player", Box::new(MyVideoFactory));
//!
//! // Create a view
//! let handle = PlatformViewRegistry::global()
//!     .create_view("video_player", PlatformViewParams::default())
//!     .unwrap();
//! ```

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Unique identifier for a platform view instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlatformViewId(pub u64);

impl PlatformViewId {
    pub fn next() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for PlatformViewId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PlatformView({})", self.0)
    }
}

/// Bounds for positioning a platform view, in logical pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlatformViewBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Parameters for creating a platform view.
#[derive(Debug, Clone, Default)]
pub struct PlatformViewParams {
    /// Initial bounds in logical pixels.
    pub bounds: PlatformViewBounds,
    /// Creation arguments (type-specific, e.g. video URL, map coordinates).
    pub creation_params: HashMap<String, String>,
}

/// Trait representing a live native platform view.
///
/// Implementors wrap a platform-specific native view (Android `View`,
/// iOS `UIView`) and manage its lifecycle.
pub trait PlatformView: Send + Sync {
    /// Returns the unique identifier for this view.
    fn id(&self) -> PlatformViewId;

    /// Returns the view type string (e.g. "video_player", "map").
    fn view_type(&self) -> &str;

    /// Update the view's position and size to match GPUI layout.
    ///
    /// Called on every frame where the element's bounds have changed.
    /// Coordinates are in logical pixels relative to the window origin.
    fn set_bounds(&self, bounds: PlatformViewBounds);

    /// Show or hide the native view.
    ///
    /// Called when the GPUI element enters/leaves the visible area or
    /// when the view is explicitly hidden.
    fn set_visible(&self, visible: bool);

    /// Set the z-order of the native view.
    ///
    /// Higher values are drawn on top. GPUI content rendered after the
    /// platform view element can overlay it using this mechanism.
    fn set_z_index(&self, z_index: i32);

    /// Dispose of the native view and release all resources.
    ///
    /// After this call, the view must not be used. The native view is
    /// removed from the view hierarchy.
    fn dispose(&self);

    /// Whether the view is currently disposed.
    fn is_disposed(&self) -> bool;
}

/// Factory for creating platform views of a specific type.
///
/// Registered with `PlatformViewRegistry` to handle creation of views
/// matching a particular `view_type` string.
pub trait PlatformViewFactory: Send + Sync {
    /// Create a new platform view with the given parameters.
    ///
    /// Returns `Ok(view)` on success or `Err(message)` if creation fails.
    fn create(&self, params: &PlatformViewParams) -> Result<Box<dyn PlatformView>, String>;

    /// The view type this factory handles (e.g. "video_player").
    fn view_type(&self) -> &str;
}

/// Handle for managing a platform view's lifecycle.
///
/// Wraps a `PlatformView` with convenience methods and automatic
/// disposal on drop.
pub struct PlatformViewHandle {
    view: Box<dyn PlatformView>,
}

impl std::fmt::Debug for PlatformViewHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformViewHandle")
            .field("id", &self.view.id())
            .field("view_type", &self.view.view_type())
            .finish()
    }
}

impl PlatformViewHandle {
    /// Create a new handle wrapping the given view.
    pub fn new(view: Box<dyn PlatformView>) -> Self {
        Self { view }
    }

    /// Get the view's unique identifier.
    pub fn id(&self) -> PlatformViewId {
        self.view.id()
    }

    /// Get the view type string.
    pub fn view_type(&self) -> &str {
        self.view.view_type()
    }

    /// Update the view's position and size.
    ///
    /// Also updates the global registry's bounds tracking so that
    /// hit-testing reflects the current position.
    pub fn set_bounds(&self, bounds: PlatformViewBounds) {
        self.view.set_bounds(bounds);
        PlatformViewRegistry::global().update_view_bounds(self.view.id(), bounds);
    }

    /// Show or hide the view.
    pub fn set_visible(&self, visible: bool) {
        self.view.set_visible(visible);
    }

    /// Set the z-order.
    pub fn set_z_index(&self, z_index: i32) {
        self.view.set_z_index(z_index);
    }

    /// Access the underlying `PlatformView` trait object.
    pub fn inner(&self) -> &dyn PlatformView {
        &*self.view
    }

    /// Dispose of the view explicitly.
    ///
    /// Also removes the view from the global registry's bounds tracking.
    pub fn dispose(&self) {
        PlatformViewRegistry::global().remove_view(self.view.id());
        self.view.dispose();
    }
}

impl Drop for PlatformViewHandle {
    fn drop(&mut self) {
        if !self.view.is_disposed() {
            PlatformViewRegistry::global().remove_view(self.view.id());
            self.view.dispose();
        }
    }
}

/// Global registry of platform view factories.
///
/// Packages register their factories here during initialization.
/// When a GPUI element needs a native view, it looks up the factory
/// by type string and creates an instance.
pub struct PlatformViewRegistry {
    factories: Mutex<HashMap<String, Box<dyn PlatformViewFactory>>>,
    views: Mutex<HashMap<PlatformViewId, PlatformViewBounds>>,
}

impl PlatformViewRegistry {
    /// Get the global registry singleton.
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<PlatformViewRegistry> = OnceLock::new();
        INSTANCE.get_or_init(|| Self {
            factories: Mutex::new(HashMap::new()),
            views: Mutex::new(HashMap::new()),
        })
    }

    /// Register a factory for a view type.
    ///
    /// If a factory for this type already exists, it is replaced.
    pub fn register(&self, view_type: &str, factory: Box<dyn PlatformViewFactory>) {
        log::info!(
            "PlatformViewRegistry: registered factory for '{}'",
            view_type
        );
        self.factories
            .lock()
            .unwrap()
            .insert(view_type.to_string(), factory);
    }

    /// Unregister a factory for a view type.
    pub fn unregister(&self, view_type: &str) {
        self.factories.lock().unwrap().remove(view_type);
    }

    /// Check if a factory is registered for the given view type.
    pub fn has_factory(&self, view_type: &str) -> bool {
        self.factories.lock().unwrap().contains_key(view_type)
    }

    /// List all registered view types.
    pub fn registered_types(&self) -> Vec<String> {
        self.factories.lock().unwrap().keys().cloned().collect()
    }

    /// Create a platform view of the specified type.
    ///
    /// Returns a `PlatformViewHandle` that manages the view's lifecycle.
    pub fn create_view(
        &self,
        view_type: &str,
        params: PlatformViewParams,
    ) -> Result<PlatformViewHandle, String> {
        let factories = self.factories.lock().unwrap();
        let factory = factories
            .get(view_type)
            .ok_or_else(|| format!("No factory registered for view type '{}'", view_type))?;

        let view = factory.create(&params)?;
        let id = view.id();
        let initial_bounds = params.bounds;
        self.views.lock().unwrap().insert(id, initial_bounds);
        log::debug!(
            "PlatformViewRegistry: created view {} of type '{}'",
            id,
            view_type
        );
        Ok(PlatformViewHandle::new(view))
    }

    /// Update the bounds of a tracked platform view.
    ///
    /// Called whenever a platform view's position or size changes. This keeps
    /// the registry's bounds map in sync for hit-testing purposes.
    pub fn update_view_bounds(&self, id: PlatformViewId, bounds: PlatformViewBounds) {
        if let Some(entry) = self.views.lock().unwrap().get_mut(&id) {
            *entry = bounds;
        }
    }

    /// Remove a platform view from the registry's tracking.
    ///
    /// Called when a platform view is disposed. After removal the view's
    /// bounds will no longer participate in hit-testing.
    pub fn remove_view(&self, id: PlatformViewId) {
        self.views.lock().unwrap().remove(&id);
    }

    /// Check if a point hits any active platform view.
    ///
    /// Returns `true` if the point (in logical pixels, relative to the
    /// window origin) falls within any registered platform view's bounds.
    ///
    /// On Android with `NativeActivity`, all touch events go to the native
    /// surface first rather than to the Java view hierarchy. This method
    /// lets the input handler detect touches on platform views so it can
    /// skip GPUI dispatch and let the native views handle them instead.
    ///
    /// On iOS the OS's `UIView` hit-testing handles this natively, but
    /// this method is available as a consistent cross-platform check.
    pub fn hit_test(&self, x: f32, y: f32) -> bool {
        let views = self.views.lock().unwrap();
        for bounds in views.values() {
            if x >= bounds.x
                && x <= bounds.x + bounds.width
                && y >= bounds.y
                && y <= bounds.y + bounds.height
            {
                return true;
            }
        }
        false
    }

    /// Returns the number of currently tracked views.
    ///
    /// Useful for quick checks — if zero, hit-testing can be skipped entirely.
    pub fn active_view_count(&self) -> usize {
        self.views.lock().unwrap().len()
    }
}
