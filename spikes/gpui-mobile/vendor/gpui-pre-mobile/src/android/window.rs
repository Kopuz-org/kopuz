//! Android window implementation.
//!
//! `AndroidWindow` wraps an `ANativeWindow *` (handed to us by the system via
//! `APP_CMD_INIT_WINDOW`) and owns the `WgpuRenderer` that draws into it.
//!
//! ## Lifecycle
//!
//! Android windows can be destroyed and recreated at any time (e.g. when the
//! app goes to the background and returns).  The window must handle:
//!
//! * `APP_CMD_INIT_WINDOW`  — create the wgpu surface and renderer.
//! * `APP_CMD_TERM_WINDOW`  — destroy the renderer, keep the window struct.
//! * `APP_CMD_WINDOW_RESIZED` — call `update_drawable_size`.
//!
//! ## Thread safety
//!
//! `AndroidWindow` is `Send + Sync`.  All renderer access is serialised through
//! a `Mutex<Option<WgpuRenderer>>`; the `Option` is `None` while the window
//! surface is unavailable.
//!
//! ## Input events
//!
//! Touch points and key events arrive via the NDK input queue and are
//! translated into the `TouchPoint` / `AndroidKeyEvent` types defined in
//! `mod.rs` before being forwarded to registered callbacks.
//!
//! ## GPUI integration
//!
//! `AndroidPlatformWindow` wraps an `Arc<AndroidWindow>` and implements
//! `gpui::PlatformWindow`, `HasWindowHandle`, and `HasDisplayHandle` so it
//! can be returned from `Platform::open_window`.

#![allow(unsafe_code)]

use anyhow::{Context as _, Result};
use futures::channel::oneshot;
use gpui::{
    self, AtlasKey, AtlasTile, Capslock, DispatchEventResult, GpuSpecs, Modifiers, PlatformAtlas,
    PlatformDisplay, PlatformInputHandler, PlatformWindow, PromptButton, PromptLevel,
    RequestFrameOptions, WindowBackgroundAppearance, WindowBounds, WindowControlArea,
    WindowVisibility,
};
use gpui_wgpu::{wgpu, GpuContext, WgpuRenderer, WgpuSurfaceConfig};
use parking_lot::Mutex;
use raw_window_handle::{
    AndroidDisplayHandle, AndroidNdkWindowHandle, HasDisplayHandle, HasWindowHandle,
    RawDisplayHandle, RawWindowHandle,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use super::{AndroidKeyEvent, Bounds, DevicePixels, Pixels, Point, Size, TouchPoint};

/// Lightweight, owned window handle for wgpu surface creation.
/// Stores the raw ANativeWindow pointer and implements the traits
/// required by `WgpuRenderer::new` (`Clone + Debug + Send + Sync + 'static`).
#[derive(Debug, Clone, Copy)]
struct RawAndroidWindow {
    nw_ptr: *mut std::ffi::c_void,
}

unsafe impl Send for RawAndroidWindow {}
unsafe impl Sync for RawAndroidWindow {}

impl HasWindowHandle for RawAndroidWindow {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
    {
        let ptr = NonNull::new(self.nw_ptr).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let handle = AndroidNdkWindowHandle::new(ptr);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(handle.into()) })
    }
}

impl HasDisplayHandle for RawAndroidWindow {
    fn display_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError>
    {
        Ok(unsafe {
            raw_window_handle::DisplayHandle::borrow_raw(RawDisplayHandle::Android(
                AndroidDisplayHandle::new(),
            ))
        })
    }
}

// Re-export for use with raw-window-handle and the frame-rate helper.
use ndk::native_window::NativeWindow;

type SetFrameRateFn = unsafe extern "C" fn(*mut ndk_sys::ANativeWindow, f32, i8) -> i32;
type SetFrameRateWithChangeStrategyFn =
    unsafe extern "C" fn(*mut ndk_sys::ANativeWindow, f32, i8, i8) -> i32;

const DEFAULT_FRAME_RATE_COMPATIBILITY: i8 = 0;
const ONLY_IF_SEAMLESS: i8 = 0;
const HIGH_FRAME_RATE: f32 = 120.0;

// Resolve an optional libandroid symbol at runtime.
//
// Linking API 30/31 frame-rate entry points directly would make the shared
// library fail to `dlopen` on older devices before our own code runs, so we
// always go through `dlsym`. Keep this dynamic lookup even if `ndk` grows
// nicer wrappers — reintroducing a static link would crash on pre-API-30.
fn load_libandroid_symbol(name: &[u8]) -> *mut c_void {
    debug_assert_eq!(name.last().copied(), Some(0));
    unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr().cast()) }
}

fn load_set_frame_rate() -> Option<SetFrameRateFn> {
    static SYMBOL: OnceLock<Option<SetFrameRateFn>> = OnceLock::new();
    *SYMBOL.get_or_init(|| {
        let symbol = load_libandroid_symbol(b"ANativeWindow_setFrameRate\0");
        if symbol.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute::<*mut c_void, SetFrameRateFn>(symbol) })
        }
    })
}

fn load_set_frame_rate_with_change_strategy() -> Option<SetFrameRateWithChangeStrategyFn> {
    static SYMBOL: OnceLock<Option<SetFrameRateWithChangeStrategyFn>> = OnceLock::new();
    *SYMBOL.get_or_init(|| {
        let symbol = load_libandroid_symbol(b"ANativeWindow_setFrameRateWithChangeStrategy\0");
        if symbol.is_null() {
            None
        } else {
            Some(unsafe {
                std::mem::transmute::<*mut c_void, SetFrameRateWithChangeStrategyFn>(symbol)
            })
        }
    })
}

/// Request 120 Hz refresh rate from the native window when the platform exposes the API.
///
/// The NDK entry points are resolved with `dlsym` so this library still loads
/// on pre-API-30/31 devices where the symbols do not exist.
fn request_high_frame_rate(window: &NativeWindow) {
    let api_level = unsafe { ndk_sys::android_get_device_api_level() };
    let native_window = window.ptr().as_ptr();

    let status = if api_level >= 31 {
        load_set_frame_rate_with_change_strategy().map(|set_frame_rate| unsafe {
            set_frame_rate(
                native_window,
                HIGH_FRAME_RATE,
                DEFAULT_FRAME_RATE_COMPATIBILITY,
                ONLY_IF_SEAMLESS,
            )
        })
    } else if api_level >= 30 {
        load_set_frame_rate().map(|set_frame_rate| unsafe {
            set_frame_rate(
                native_window,
                HIGH_FRAME_RATE,
                DEFAULT_FRAME_RATE_COMPATIBILITY,
            )
        })
    } else {
        None
    };

    match status {
        Some(0) => log::info!("Requested 120 Hz frame rate"),
        Some(status) => log::warn!("ANativeWindow_setFrameRate* failed with status {status}"),
        None => log::debug!("Skipping high frame rate request on API {api_level}"),
    }
}

// ── callback type aliases ─────────────────────────────────────────────────────

/// Called once per VSync tick to produce the next `Scene`.
pub type RequestFrameCallback = Box<dyn FnMut() + Send + 'static>;

/// Called when a touch event arrives.
pub type TouchCallback = Box<dyn FnMut(TouchPoint) + Send + 'static>;

/// Called when the window's active status changes (foreground/background).
pub type ActiveStatusCallback = Box<dyn FnMut(bool) + Send + 'static>;

/// Called when the window becomes visible or hidden.
pub type VisibilityCallback = Box<dyn FnMut(bool) + Send + 'static>;

/// Called when a key event arrives.
pub type KeyCallback = Box<dyn FnMut(AndroidKeyEvent) + Send + 'static>;

/// Called when the window is resized.
pub type ResizeCallback = Box<dyn FnMut(Size<DevicePixels>, f32) + Send + 'static>;

/// Called when the window is destroyed (surface lost).
pub type CloseCallback = Box<dyn FnOnce() + Send + 'static>;

/// Called when the window appearance (light/dark) changes.
pub type AppearanceCallback = Box<dyn FnMut(WindowAppearance) + Send + 'static>;

// ── appearance ────────────────────────────────────────────────────────────────

/// Light / dark / high-contrast appearance of the window.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum WindowAppearance {
    #[default]
    Light,
    Dark,
    HighContrastLight,
    HighContrastDark,
}

/// kopuz: what a hardware key types. GPUI leaves inserting it to the platform, as macOS's
/// `insertText` and Linux's XIM do, once no binding has taken the key; this platform never did,
/// so a physical keyboard could not type at all.
fn typed_text(event: &gpui::PlatformInput) -> Option<String> {
    let gpui::PlatformInput::KeyDown(down) = event else {
        return None;
    };
    let modifiers = &down.keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    down.keystroke
        .key_char
        .clone()
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

/// Inserts `text` at the focused input's selection; no `RefCell` borrow spans the handler's update.
fn insert_typed(slot: &Rc<RefCell<Option<PlatformInputHandler>>>, text: &str) {
    let Some(mut handler) = slot.borrow_mut().take() else {
        return;
    };
    handler.replace_text_in_range(None, text);
    let mut current = slot.borrow_mut();
    if current.is_none() {
        *current = Some(handler);
    }
}

// ── window state (interior-mutable) ──────────────────────────────────────────

/// Safe area insets in physical (device) pixels.
///
/// These represent the areas of the screen occupied by system UI elements
/// (status bar, navigation bar, camera notch, etc.) that the app content
/// should avoid drawing into — or at least pad/account for.
#[derive(Debug, Clone, Copy, Default)]
pub struct SafeAreaInsets {
    /// Top inset in device pixels (status bar / camera notch).
    pub top: f32,
    /// Bottom inset in device pixels (navigation bar / gesture indicator).
    pub bottom: f32,
    /// Left inset in device pixels (e.g. display cutout on landscape).
    pub left: f32,
    /// Right inset in device pixels (e.g. display cutout on landscape).
    pub right: f32,
}

impl SafeAreaInsets {
    /// Convert physical-pixel insets to logical pixels using the given scale factor.
    pub fn to_logical(&self, scale_factor: f32) -> SafeAreaInsets {
        SafeAreaInsets {
            top: self.top / scale_factor,
            bottom: self.bottom / scale_factor,
            left: self.left / scale_factor,
            right: self.right / scale_factor,
        }
    }
}

struct WindowState {
    /// The `NativeWindow` handle.  Reference-counted via `ndk::NativeWindow`
    /// (Clone acquires, Drop releases).  `None` while the surface is
    /// unavailable (between `term_window` and the next `init_window`).
    native_window: Option<NativeWindow>,

    /// Shared GPU context (instance + adapter + device + queue).
    /// Created once and reused across window re-creations.
    gpu_context: GpuContext,

    /// The wgpu renderer from `gpui_wgpu`.  `None` while the surface is unavailable.
    renderer: Option<WgpuRenderer>,

    /// Cached display geometry.
    width: i32,
    height: i32,
    scale_factor: f32,

    /// Safe area insets in physical (device) pixels.
    ///
    /// Updated from `AndroidApp::content_rect()` during `InitWindow` and
    /// `WindowResized`.  The top inset accounts for the status bar / notch,
    /// and the bottom inset accounts for the navigation bar / gesture area.
    safe_area_insets: SafeAreaInsets,

    /// Current appearance.
    appearance: WindowAppearance,

    /// Whether the window is currently visible / active.
    is_active: bool,

    /// Whether the window background should be transparent.
    transparent: bool,

    // ── callbacks ─────────────────────────────────────────────────────────
    request_frame_callback: Option<RequestFrameCallback>,
    touch_callback: Option<TouchCallback>,
    key_callback: Option<KeyCallback>,
    resize_callback: Option<ResizeCallback>,
    close_callback: Option<CloseCallback>,
    appearance_callback: Option<AppearanceCallback>,
    active_status_callback: Option<ActiveStatusCallback>,
    visibility_callback: Option<VisibilityCallback>,
}

// SAFETY: `WindowState` is only ever accessed while holding the
// `Mutex<WindowState>` lock, and all GPU work (including any use of
// `GpuContext = Rc<RefCell<Option<WgpuContext>>>`) happens exclusively on the
// Android main thread.  The `Rc` never escapes to another thread.
unsafe impl Send for WindowState {}

// ── AndroidWindow ─────────────────────────────────────────────────────────────

/// A GPUI window on Android.
///
/// Each `AndroidWindow` corresponds to one `ANativeWindow *` provided by the
/// system.  On a typical Android device there is exactly one window per app,
/// but foldable / multi-display devices may have two.
pub struct AndroidWindow {
    state: Arc<Mutex<WindowState>>,
    /// A stable numeric ID derived from the initial native-window pointer.
    id: u64,
    /// Whether the window is currently active (foregrounded).
    ///
    /// This is an `AtomicBool` separate from `WindowState` so that
    /// lifecycle handlers can set it without acquiring the state lock
    /// (which may be held by a background render thread).
    active: Arc<std::sync::atomic::AtomicBool>,
}

// SAFETY: `WindowState` is protected by a `Mutex`.
unsafe impl Send for AndroidWindow {}
unsafe impl Sync for AndroidWindow {}

impl AndroidWindow {
    // ── constructors ─────────────────────────────────────────────────────────

    /// Create an `AndroidWindow` from an `ndk::NativeWindow`.
    ///
    /// Initialises the wgpu surface and renderer immediately.
    ///
    /// `gpu_context` — shared wgpu device/queue/instance.  Pass `None` on the
    /// first window; it will be initialised and stored.  Subsequent windows
    /// should pass the same `Option<WgpuContext>` so that the GPU context is
    /// shared.
    ///
    /// `scale_factor` — the display density relative to 160 dpi (e.g. 3.0 for
    /// a 480 dpi device).
    ///
    /// `transparent` — whether to request a pre-multiplied alpha surface.
    pub fn new(
        native_window: NativeWindow,
        gpu_context: GpuContext,
        scale_factor: f32,
        transparent: bool,
    ) -> Result<Arc<Self>> {
        request_high_frame_rate(&native_window);

        let width = native_window.width();
        let height = native_window.height();

        log::info!(
            "AndroidWindow::new — {}×{} scale={:.1}",
            width,
            height,
            scale_factor
        );

        let renderer = Self::create_renderer(
            &native_window,
            Rc::clone(&gpu_context),
            width,
            height,
            transparent,
        )
        .context("failed to create gpui_wgpu renderer")?;

        let id = native_window.ptr().as_ptr() as u64;

        let state = Arc::new(Mutex::new(WindowState {
            native_window: Some(native_window),
            gpu_context,
            renderer: Some(renderer),
            width,
            height,
            scale_factor,
            safe_area_insets: SafeAreaInsets::default(),
            appearance: if super::jni::query_night_mode_via_jni() {
                WindowAppearance::Dark
            } else {
                WindowAppearance::Light
            },
            is_active: true,
            transparent,
            request_frame_callback: None,
            touch_callback: None,
            key_callback: None,
            resize_callback: None,
            close_callback: None,
            appearance_callback: None,
            active_status_callback: None,
            visibility_callback: None,
        }));

        Ok(Arc::new(Self {
            state,
            id,
            active: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }))
    }

    /// Create a headless `AndroidWindow` for testing.
    ///
    /// No wgpu surface is created.
    pub fn headless(width: i32, height: i32, scale_factor: f32) -> Arc<Self> {
        let state = Arc::new(Mutex::new(WindowState {
            native_window: None,
            gpu_context: Rc::new(RefCell::new(None)),
            renderer: None,
            width,
            height,
            scale_factor,
            safe_area_insets: SafeAreaInsets::default(),
            appearance: WindowAppearance::Light,
            is_active: false,
            transparent: false,
            request_frame_callback: None,
            touch_callback: None,
            key_callback: None,
            resize_callback: None,
            close_callback: None,
            appearance_callback: None,
            active_status_callback: None,
            visibility_callback: None,
        }));

        Arc::new(Self {
            state,
            id: ((width as u64) << 32) | (height as u64),
            active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    // ── window lifecycle ──────────────────────────────────────────────────────

    /// Called when `APP_CMD_INIT_WINDOW` fires and a new `NativeWindow` is
    /// available (e.g. after returning from the background).
    pub fn init_window(&self, native_window: NativeWindow, gpu_context: GpuContext) -> Result<()> {
        request_high_frame_rate(&native_window);

        let width = native_window.width();
        let height = native_window.height();

        let mut state = self.state.lock();
        let transparent = state.transparent;

        // If a renderer already exists (kept alive across term_window), just
        // replace its surface.  This preserves the atlas and all cached
        // AtlasTextureIds so GPUI's scene cache remains valid.
        if state.renderer.is_some() {
            let raw = Self::raw_window(&native_window);
            let config = WgpuSurfaceConfig {
                size: gpui::size(gpui::DevicePixels(width), gpui::DevicePixels(height)),
                transparent,
                preferred_present_mode: Some(wgpu::PresentMode::Fifo),
            };
            let instance = state
                .gpu_context
                .borrow()
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("gpu_context missing during surface replacement"))?
                .instance
                .clone();
            state
                .renderer
                .as_mut()
                .unwrap()
                .replace_surface(&raw, config, &instance)?;
            log::info!(
                "AndroidWindow::init_window — replaced surface {}×{}",
                width,
                height
            );
        } else {
            // First init or after a full destroy — create a fresh renderer.
            let ctx = if state.gpu_context.borrow().is_some() {
                Rc::clone(&state.gpu_context)
            } else {
                // Use the context from the platform; store it in our state too.
                state.gpu_context = Rc::clone(&gpu_context);
                gpu_context
            };
            let renderer = Self::create_renderer(&native_window, ctx, width, height, transparent)?;
            state.renderer = Some(renderer);
            log::info!(
                "AndroidWindow::init_window — created new renderer {}×{}",
                width,
                height
            );
        }

        // Store the new native window (drops previous one if any).
        state.native_window = Some(native_window);
        state.width = width;
        state.height = height;
        state.is_active = true;
        self.active
            .store(true, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    /// Called when `APP_CMD_TERM_WINDOW` fires and the surface is about to be
    /// destroyed.
    ///
    /// Drops the renderer (and therefore the wgpu surface) but keeps the window
    /// struct alive so callbacks are preserved.
    pub fn term_window(&self) {
        let mut state = self.state.lock();

        // Unconfigure the surface so the renderer stops trying to present,
        // but keep the renderer alive so the atlas (with all cached
        // texture IDs) survives across the background/foreground cycle.
        if let Some(ref mut renderer) = state.renderer {
            renderer.unconfigure_surface();
            log::info!("AndroidWindow::term_window — surface unconfigured (renderer kept alive)");
        }

        // Release our reference on the native window.
        state.native_window = None;

        state.is_active = false;
        self.active
            .store(false, std::sync::atomic::Ordering::Relaxed);

        // NOTE: We intentionally do NOT fire the close callback here.
        // On Android, term_window means the surface is being destroyed
        // (e.g. the app is going to the background), but the logical
        // window and all its GPUI callbacks should stay alive so that
        // when the surface is recreated on resume, rendering resumes
        // seamlessly via init_window().
    }

    /// Called when `APP_CMD_WINDOW_RESIZED` fires.
    pub fn handle_resize(&self) {
        let (new_w, new_h, scale) = {
            let mut state = self.state.lock();

            let nw = match state.native_window.as_ref() {
                Some(nw) => nw,
                None => return,
            };

            let new_w = nw.width();
            let new_h = nw.height();

            if new_w == state.width && new_h == state.height {
                log::debug!("handle_resize: no change ({}×{})", new_w, new_h);
                return;
            }

            log::info!(
                "AndroidWindow resize: {}×{} → {}×{}",
                state.width,
                state.height,
                new_w,
                new_h
            );

            state.width = new_w;
            state.height = new_h;
            let scale = state.scale_factor;

            // update_drawable_size calls device.poll(Wait) which can take
            // time — take the renderer out to avoid holding the state lock.
            if let Some(mut renderer) = state.renderer.take() {
                renderer.update_drawable_size(gpui::size(
                    gpui::DevicePixels(new_w),
                    gpui::DevicePixels(new_h),
                ));
                state.renderer = Some(renderer);
            }

            (new_w, new_h, scale)
        }; // state lock dropped

        // Fire the resize callback outside the lock — GPUI's callback may
        // call bounds() / scale_factor() which need the state lock.
        let cb = {
            let mut state = self.state.lock();
            state.resize_callback.take()
        };
        if let Some(mut cb) = cb {
            cb(
                Size {
                    width: DevicePixels(new_w),
                    height: DevicePixels(new_h),
                },
                scale,
            );
            // Put callback back.
            let mut state = self.state.lock();
            if state.resize_callback.is_none() {
                state.resize_callback = Some(cb);
            }
        }
    }

    // ── drawing ───────────────────────────────────────────────────────────────

    /// Returns `true` when the scene contains no renderable primitives.
    ///
    /// Checks every primitive bucket instead of just `quads`, because text
    /// and images are emitted as sprite primitives.
    fn scene_is_empty(scene: &gpui::Scene) -> bool {
        scene.shadows.is_empty()
            && scene.quads.is_empty()
            && scene.paths.is_empty()
            && scene.underlines.is_empty()
            && scene.monochrome_sprites.is_empty()
            && scene.subpixel_sprites.is_empty()
            && scene.polychrome_sprites.is_empty()
            && scene.surfaces.is_empty()
    }

    /// Draw `scene` into the window's next frame.
    ///
    /// Accepts a `gpui::Scene` directly — the `gpui_wgpu::WgpuRenderer`
    /// natively consumes it without any type bridging.
    ///
    /// No-ops if the renderer is not available (surface lost).
    ///
    /// **Guard against empty scenes**: If the scene contains zero
    /// primitives (no quads, shadows, paths, underlines, or sprites)
    /// we skip the draw entirely.  The upstream `gpui_wgpu` renderer
    /// clears the surface to transparent/black before drawing, so
    /// presenting an empty scene produces a visible flash where all
    /// content disappears for one frame.  This commonly happens:
    ///
    /// - During the first few event-loop iterations before GPUI has
    ///   finished building the view tree.
    /// - When the surface is reconfigured after a Lost/Outdated error
    ///   and the next frame callback produces an empty scene.
    /// - Intermittently during fast scrolling if the layout pass
    ///   hasn't produced new content yet.
    pub fn draw(&self, scene: &gpui::Scene) {
        // Skip only truly empty scenes. Text and images are emitted as sprite
        // primitives, not quads, so checking `scene.quads` alone can drop
        // valid text-only frames during scroll or cache refresh and produce
        // the flicker we were trying to avoid.
        if Self::scene_is_empty(scene) {
            log::trace!("AndroidWindow::draw — skipping empty scene");
            return;
        }

        // Take the renderer out of state so we can draw WITHOUT holding
        // the state lock.  renderer.draw() calls get_current_texture()
        // which can block on the GPU / Vulkan driver.  Holding the lock
        // during that time prevents all other state accessors (bounds,
        // scale_factor, etc.) from running, leading to deadlock when
        // GPUI's layout or the event loop needs state during a render.
        let mut renderer = {
            let mut state = self.state.lock();
            match state.renderer.take() {
                Some(r) => r,
                None => return,
            }
        };

        renderer.draw(scene);

        // Put the renderer back.
        let mut state = self.state.lock();
        state.renderer = Some(renderer);
    }

    /// Invoke the `request_frame_callback` if one is registered.
    ///
    /// Called by the event loop on every iteration (~60 fps).
    ///
    /// **Important**: The callback is taken out of the lock before being
    /// invoked and put back afterwards.  This avoids a deadlock: the GPUI
    /// callback runs layout → paint → `PlatformWindow::draw` →
    /// `AndroidWindow::draw`, which needs to acquire the same `state` lock
    /// to access the renderer.
    pub fn request_frame(&self) {
        // Take the callback out of the lock so it can be invoked without
        // holding it.  This lets `draw()` (called from inside the callback)
        // acquire the lock for the renderer.
        let cb = {
            let mut state = self.state.lock();
            state.request_frame_callback.take()
        };

        if let Some(mut cb) = cb {
            cb();

            // Put the callback back so it fires again next frame.
            let mut state = self.state.lock();
            // Only put it back if nothing else registered a new callback
            // while we were running (unlikely but defensive).
            if state.request_frame_callback.is_none() {
                state.request_frame_callback = Some(cb);
            }
        }
    }

    // ── input event delivery ──────────────────────────────────────────────────

    /// Deliver a touch point to the registered touch callback.
    ///
    /// The callback is taken out of the lock before invocation (same pattern
    /// as `request_frame`) to avoid potential deadlocks if the callback
    /// re-enters any window method that needs the lock.
    pub fn handle_touch(&self, point: TouchPoint) {
        let cb = {
            let mut state = self.state.lock();
            state.touch_callback.take()
        };
        if let Some(mut cb) = cb {
            let scale = self.scale_factor();
            log::debug!(
                "handle_touch: id={} action={} phys=({:.0},{:.0}) logical=({:.0},{:.0}) scale={:.1}",
                point.id, point.action, point.x, point.y,
                point.x / scale, point.y / scale, scale,
            );
            cb(point);
            let mut state = self.state.lock();
            if state.touch_callback.is_none() {
                state.touch_callback = Some(cb);
            }
        } else {
            log::warn!(
                "handle_touch: NO touch_callback registered — touch dropped (id={} action={})",
                point.id,
                point.action,
            );
        }
    }

    /// Deliver a key event to the registered key callback.
    pub fn handle_key_event(&self, event: AndroidKeyEvent) {
        let cb = {
            let mut state = self.state.lock();
            state.key_callback.take()
        };
        if let Some(mut cb) = cb {
            cb(event);
            let mut state = self.state.lock();
            if state.key_callback.is_none() {
                state.key_callback = Some(cb);
            }
        }
    }

    // ── appearance ────────────────────────────────────────────────────────────

    /// Update the window's appearance (e.g. after a dark-mode change).
    pub fn set_appearance(&self, appearance: WindowAppearance) {
        let changed = {
            let mut state = self.state.lock();
            if state.appearance == appearance {
                return;
            }
            state.appearance = appearance;
            true
        };
        if changed {
            // Fire callback outside the lock — it may call back into
            // GPUI which needs state access.
            let cb = {
                let mut state = self.state.lock();
                state.appearance_callback.take()
            };
            if let Some(mut cb) = cb {
                cb(appearance);
                let mut state = self.state.lock();
                if state.appearance_callback.is_none() {
                    state.appearance_callback = Some(cb);
                }
            }
        }
    }

    /// Returns the current appearance.
    pub fn appearance(&self) -> WindowAppearance {
        self.state.lock().appearance
    }

    /// Update the active (foreground/background) status of the window.
    ///
    /// Called by `on_window_focus_changed` in `jni.rs`.
    pub fn set_active(&self, active: bool) {
        use std::sync::atomic::Ordering;
        let prev = self.active.swap(active, Ordering::Relaxed);
        if prev != active {
            log::info!(
                "AndroidWindow::set_active({}) — changed from {}",
                active,
                prev
            );
            // Take the callback out of the state so we can invoke it WITHOUT
            // holding the window state lock.  The callback wraps a GPUI
            // closure that acquires its own Mutex (and may call back into
            // GPUI), so calling it under the state lock deadlocks.
            let mut taken_cb: Option<Box<dyn FnMut(bool) + Send>> = None;
            let mut visibility_cb: Option<Box<dyn FnMut(bool) + Send>> = None;
            if let Some(mut state) = self.state.try_lock() {
                state.is_active = active;
                taken_cb = state.active_status_callback.take();
                visibility_cb = state.visibility_callback.take();
            } else {
                log::info!(
                    "AndroidWindow::set_active({}) — lock busy, skipping",
                    active
                );
            }
            // Fire callback outside the lock.
            if let Some(mut cb) = taken_cb {
                cb(active);
                // Put it back so future calls still fire.
                if let Some(mut state) = self.state.try_lock() {
                    state.active_status_callback = Some(cb);
                }
            }
            if let Some(mut cb) = visibility_cb {
                cb(active);
                if let Some(mut state) = self.state.try_lock() {
                    state.visibility_callback = Some(cb);
                }
            }
            log::info!("AndroidWindow::set_active({}) — done", active);
        }
    }

    // ── transparency ──────────────────────────────────────────────────────────

    /// Enable or disable transparent compositing.
    pub fn set_transparent(&self, transparent: bool) {
        let mut state = self.state.lock();
        if state.transparent == transparent {
            return;
        }
        state.transparent = transparent;
        if let Some(renderer) = state.renderer.as_mut() {
            renderer.update_transparency(transparent);
        }
    }

    // ── geometry / scale ──────────────────────────────────────────────────────

    /// Physical size of the window in device pixels.
    pub fn physical_size(&self) -> Size<DevicePixels> {
        let state = self.state.lock();
        Size {
            width: DevicePixels(state.width),
            height: DevicePixels(state.height),
        }
    }

    /// Logical size of the window in density-independent pixels.
    pub fn logical_size(&self) -> Size<Pixels> {
        let state = self.state.lock();
        Size {
            width: Pixels(state.width as f32 / state.scale_factor),
            height: Pixels(state.height as f32 / state.scale_factor),
        }
    }

    /// Physical bounds with origin at `(0, 0)`.
    pub fn bounds(&self) -> Bounds<DevicePixels> {
        Bounds {
            origin: Point {
                x: DevicePixels(0),
                y: DevicePixels(0),
            },
            size: self.physical_size(),
        }
    }

    /// Display scale factor (device pixels per logical pixel).
    pub fn scale_factor(&self) -> f32 {
        self.state.lock().scale_factor
    }

    /// Returns the current safe area insets in physical (device) pixels.
    pub fn safe_area_insets(&self) -> SafeAreaInsets {
        self.state.lock().safe_area_insets
    }

    /// Returns the current safe area insets in logical pixels.
    pub fn safe_area_insets_logical(&self) -> SafeAreaInsets {
        let state = self.state.lock();
        state.safe_area_insets.to_logical(state.scale_factor)
    }

    /// Update the safe area insets from the content rect provided by the system.
    ///
    /// `content_rect` is `(left, top, right, bottom)` in physical pixels — the
    /// area of the window NOT covered by system bars.  We compute insets by
    /// subtracting from the full window dimensions.
    pub fn update_safe_area_from_content_rect(
        &self,
        content_left: i32,
        content_top: i32,
        content_right: i32,
        content_bottom: i32,
    ) {
        let mut state = self.state.lock();
        let insets = SafeAreaInsets {
            top: content_top as f32,
            bottom: (state.height - content_bottom).max(0) as f32,
            left: content_left as f32,
            right: (state.width - content_right).max(0) as f32,
        };
        log::info!(
            "safe_area_insets updated: top={:.0} bottom={:.0} left={:.0} right={:.0} (physical px)",
            insets.top,
            insets.bottom,
            insets.left,
            insets.right,
        );
        state.safe_area_insets = insets;
    }

    // ── state queries ─────────────────────────────────────────────────────────

    /// Whether the window currently has an active wgpu surface.
    pub fn has_surface(&self) -> bool {
        self.state.lock().renderer.is_some()
    }

    /// Returns the sprite atlas from the renderer, if available.
    pub fn sprite_atlas(&self) -> Option<Arc<dyn PlatformAtlas>> {
        let state = self.state.lock();
        state
            .renderer
            .as_ref()
            .map(|r| r.sprite_atlas().clone() as Arc<dyn PlatformAtlas>)
    }

    /// Returns GPU specs from the renderer, if available.
    pub fn gpu_specs(&self) -> Option<GpuSpecs> {
        let state = self.state.lock();
        state.renderer.as_ref().map(|r| r.gpu_specs())
    }

    /// Whether the window is currently active / visible.
    pub fn is_active(&self) -> bool {
        self.active.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A stable numeric identifier for this window.
    pub fn id(&self) -> u64 {
        self.id
    }

    // ── callback registration ─────────────────────────────────────────────────

    /// Register a callback invoked once per VSync tick.
    pub fn on_request_frame<F>(&self, cb: F)
    where
        F: FnMut() + Send + 'static,
    {
        self.state.lock().request_frame_callback = Some(Box::new(cb));
    }

    /// Register a callback invoked for each touch point.
    pub fn on_touch<F>(&self, cb: F)
    where
        F: FnMut(TouchPoint) + Send + 'static,
    {
        self.state.lock().touch_callback = Some(Box::new(cb));
    }

    /// Register a callback invoked for each key event.
    pub fn on_key_event<F>(&self, cb: F)
    where
        F: FnMut(AndroidKeyEvent) + Send + 'static,
    {
        self.state.lock().key_callback = Some(Box::new(cb));
    }

    /// Register a callback invoked when the window is resized.
    pub fn on_resize<F>(&self, cb: F)
    where
        F: FnMut(Size<DevicePixels>, f32) + Send + 'static,
    {
        self.state.lock().resize_callback = Some(Box::new(cb));
    }

    /// Register a callback invoked once when the window surface is lost.
    pub fn on_close<F>(&self, cb: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.state.lock().close_callback = Some(Box::new(cb));
    }

    /// Register a callback invoked when the appearance (light/dark) changes.
    pub fn on_appearance_changed<F>(&self, cb: F)
    where
        F: FnMut(WindowAppearance) + Send + 'static,
    {
        self.state.lock().appearance_callback = Some(Box::new(cb));
    }

    /// Register a callback invoked when the window's active status changes.
    pub fn on_active_status_change<F>(&self, cb: F)
    where
        F: FnMut(bool) + Send + 'static,
    {
        self.state.lock().active_status_callback = Some(Box::new(cb));
    }

    pub fn on_visibility_change<F>(&self, cb: F)
    where
        F: FnMut(bool) + Send + 'static,
    {
        self.state.lock().visibility_callback = Some(Box::new(cb));
    }

    // ── GPU introspection ─────────────────────────────────────────────────────

    /// Whether the GPU supports dual-source blending (subpixel text AA).
    pub fn supports_subpixel_aa(&self) -> bool {
        self.state
            .lock()
            .renderer
            .as_ref()
            .map(|r| r.supports_dual_source_blending())
            .unwrap_or(false)
    }

    // ── private helpers ───────────────────────────────────────────────────────

    /// Build a raw-window-handle wrapper for the given `NativeWindow`.
    ///
    /// `ndk::NativeWindow` implements `HasWindowHandle` but not `HasDisplayHandle`,
    /// so we wrap both into a single owned struct that satisfies the
    /// `Clone + Debug + Send + Sync + 'static` bounds required by `WgpuRenderer::new`.
    fn raw_window(native_window: &NativeWindow) -> RawAndroidWindow {
        let ptr = native_window
            .window_handle()
            .expect("NativeWindow handle unavailable")
            .as_raw();
        // Extract the raw pointer from the RawWindowHandle
        let nw_ptr = match ptr {
            RawWindowHandle::AndroidNdk(h) => h.a_native_window.as_ptr(),
            _ => panic!("Expected AndroidNdk window handle"),
        };
        RawAndroidWindow { nw_ptr }
    }

    /// Create a `WgpuRenderer` for the given `NativeWindow`.
    fn create_renderer(
        native_window: &NativeWindow,
        gpu_context: GpuContext,
        width: i32,
        height: i32,
        transparent: bool,
    ) -> Result<WgpuRenderer> {
        let raw = Self::raw_window(native_window);

        let config = WgpuSurfaceConfig {
            size: gpui::size(gpui::DevicePixels(width), gpui::DevicePixels(height)),
            transparent,
            preferred_present_mode: Some(wgpu::PresentMode::Fifo),
        };

        WgpuRenderer::new(gpu_context, &raw, config, None)
    }
}

impl Drop for AndroidWindow {
    fn drop(&mut self) {
        let mut state = self.state.lock();

        // Destroy renderer before releasing the native window.
        if let Some(mut renderer) = state.renderer.take() {
            renderer.destroy();
        }

        state.native_window = None;
    }
}

// ── AndroidPlatformWindow (PlatformWindow impl) ──────────────────────────────

/// A wrapper around `Arc<AndroidWindow>` that implements `gpui::PlatformWindow`.
///
/// GPUI expects `Box<dyn PlatformWindow>` from `Platform::open_window`.  This
/// struct provides the trait implementation by delegating to the underlying
/// `AndroidWindow` methods.
#[allow(clippy::type_complexity)]
pub struct AndroidPlatformWindow {
    window: Arc<AndroidWindow>,
    display: Option<Rc<dyn PlatformDisplay>>,
    input_handler: Rc<RefCell<Option<PlatformInputHandler>>>,
    ime_input_callback:
        Arc<Mutex<Box<dyn FnMut(gpui::PlatformInput) -> DispatchEventResult + Send>>>,
    title: String,
}

impl AndroidPlatformWindow {
    /// Create a new `AndroidPlatformWindow` wrapping an existing `AndroidWindow`.
    pub fn new(window: Arc<AndroidWindow>, display: Option<Rc<dyn PlatformDisplay>>) -> Self {
        Self {
            window,
            display,
            input_handler: Rc::new(RefCell::new(None)),
            ime_input_callback: Arc::new(Mutex::new(Box::new(|_| DispatchEventResult::default()))),
            title: String::new(),
        }
    }

    /// Access the underlying `AndroidWindow`.
    pub fn inner(&self) -> &Arc<AndroidWindow> {
        &self.window
    }
}

impl HasWindowHandle for AndroidPlatformWindow {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
    {
        let state = self.window.state.lock();
        let nw = state
            .native_window
            .as_ref()
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        // Build the handle from the raw pointer.  The pointer remains valid
        // because AndroidWindow holds a NativeWindow reference for as long
        // as this AndroidPlatformWindow (and thus `self`) is alive.
        let handle = AndroidNdkWindowHandle::new(NonNull::new(nw.ptr().as_ptr().cast()).unwrap());
        let raw = RawWindowHandle::AndroidNdk(handle);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for AndroidPlatformWindow {
    fn display_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError>
    {
        let raw = RawDisplayHandle::Android(AndroidDisplayHandle::new());
        // SAFETY: Android display handle has no lifetime requirements.
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(raw) })
    }
}

impl PlatformWindow for AndroidPlatformWindow {
    fn bounds(&self) -> gpui::Bounds<gpui::Pixels> {
        let state = self.window.state.lock();
        let w = state.width as f32 / state.scale_factor;
        let h = state.height as f32 / state.scale_factor;
        gpui::Bounds {
            origin: gpui::point(gpui::px(0.0), gpui::px(0.0)),
            size: gpui::size(gpui::px(w), gpui::px(h)),
        }
    }

    fn is_maximized(&self) -> bool {
        // Android windows are always effectively maximized (fullscreen).
        true
    }

    fn visibility(&self) -> WindowVisibility {
        if self.window.is_active() {
            WindowVisibility::Visible
        } else {
            WindowVisibility::Hidden
        }
    }

    fn window_bounds(&self) -> WindowBounds {
        // Android windows are always fullscreen.
        WindowBounds::Fullscreen(self.bounds())
    }

    fn content_size(&self) -> gpui::Size<gpui::Pixels> {
        self.bounds().size
    }

    fn resize(&mut self, _size: gpui::Size<gpui::Pixels>) {
        // Android windows are resized by the system, not the application.
        // No-op: the system controls window size.
    }

    fn scale_factor(&self) -> f32 {
        self.window.scale_factor()
    }

    fn appearance(&self) -> gpui::WindowAppearance {
        let local_appearance = self.window.appearance();
        match local_appearance {
            WindowAppearance::Dark => gpui::WindowAppearance::Dark,
            WindowAppearance::HighContrastDark => gpui::WindowAppearance::VibrantDark,
            WindowAppearance::Light | WindowAppearance::HighContrastLight => {
                gpui::WindowAppearance::Light
            }
        }
    }

    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.display.clone()
    }

    fn mouse_position(&self) -> gpui::Point<gpui::Pixels> {
        // Android is primarily touch-based; return a default position.
        // Touch events are delivered through the input callback.
        gpui::Point::default()
    }

    fn modifiers(&self) -> Modifiers {
        Modifiers::default()
    }

    fn capslock(&self) -> Capslock {
        Capslock::default()
    }

    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        *self.input_handler.borrow_mut() = Some(input_handler);
    }

    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.input_handler.borrow_mut().take()
    }

    fn text_input_state_changed(&self, change: gpui::TextInputStateChange) {
        match change {
            gpui::TextInputStateChange::FocusGained => {
                super::jni::show_keyboard_android(crate::KeyboardType::Default);
            }
            gpui::TextInputStateChange::FocusLost => super::jni::hide_keyboard_android(),
            _ => {}
        }
    }

    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        // Android prompts would require JNI calls to show an AlertDialog.
        // Return None to indicate the platform cannot show prompts natively.
        None
    }

    fn activate(&self) {
        // Android windows are always "active" when in the foreground.
        // The system manages window activation via the activity lifecycle.
    }

    fn is_active(&self) -> bool {
        self.window.is_active()
    }

    fn is_hovered(&self) -> bool {
        // Touch-based — always "hovered" when active.
        self.window.is_active()
    }

    fn background_appearance(&self) -> WindowBackgroundAppearance {
        WindowBackgroundAppearance::Opaque
    }

    fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
        // Android NativeActivity doesn't have a title bar to set.
        log::debug!("AndroidPlatformWindow::set_title({title})");
    }

    fn set_background_appearance(&self, _background: WindowBackgroundAppearance) {
        // Could toggle transparency on the renderer, but for now no-op.
    }

    fn minimize(&self) {
        // Android apps minimize via the system back/home button, not programmatically.
        log::debug!("AndroidPlatformWindow::minimize — no-op on Android");
    }

    fn zoom(&self) {
        // Android windows are always fullscreen.
    }

    fn toggle_fullscreen(&self) {
        // Android windows are always fullscreen.
    }

    fn is_fullscreen(&self) -> bool {
        true
    }

    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        let input_handler = Rc::clone(&self.input_handler);
        let input_callback = Arc::clone(&self.ime_input_callback);
        let mut callback = callback;
        let callback: Box<dyn FnMut(RequestFrameOptions)> = Box::new(move |options| {
            super::text_input::drain(&input_handler, &mut **input_callback.lock());
            callback(options);
        });
        // PlatformWindow gives us Box<dyn FnMut(...)> (not Send).
        // AndroidWindow::on_request_frame requires Send.  On Android the
        // request-frame callback is always invoked on the main thread, so
        // this transmute is safe in practice.
        let send_callback: Box<dyn FnMut(RequestFrameOptions) + Send> =
            unsafe { std::mem::transmute(callback) };
        let send_callback = Mutex::new(send_callback);

        self.window.on_request_frame(move || {
            // Check if text input arrived since last frame — if so, force a
            // render so drain_pending_text() runs and the UI updates.
            let text_dirty =
                crate::TEXT_INPUT_DIRTY.swap(false, std::sync::atomic::Ordering::AcqRel);

            let mut cb = send_callback.lock();
            cb(RequestFrameOptions {
                require_presentation: false,
                force_render: text_dirty,
            });
        });
    }

    fn on_input(&self, callback: Box<dyn FnMut(gpui::PlatformInput) -> DispatchEventResult>) {
        let input_handler = Rc::clone(&self.input_handler);
        let mut callback = callback;
        let callback: Box<dyn FnMut(gpui::PlatformInput) -> DispatchEventResult> = Box::new(
            move |event| {
                if matches!(&event, gpui::PlatformInput::Touch(touch) if touch.phase == gpui::TouchPhase::Started)
                {
                    // Apply any last IME update to the old input before a tap can
                    // change focus. A new native session rejects late IME callbacks.
                    super::text_input::drain(&input_handler, &mut callback);
                    super::text_input::finish_composition(&input_handler);
                }
                let typed = typed_text(&event);
                let result = callback(event);
                if let Some(text) = typed {
                    if result.propagate && !result.default_prevented {
                        insert_typed(&input_handler, &text);
                    }
                }
                result
            },
        );
        // Bridge AndroidWindow touch/key callbacks → gpui::PlatformInput.
        //
        // PlatformWindow gives us Box<dyn FnMut(...)> (not Send).
        // AndroidWindow callbacks require Send.  On Android the input
        // callbacks are always invoked on the main thread, so this
        // transmute is safe in practice.
        let send_callback: Box<dyn FnMut(gpui::PlatformInput) -> DispatchEventResult + Send> =
            unsafe { std::mem::transmute(callback) };
        *self.ime_input_callback.lock() = send_callback;
        let input_cb = Arc::clone(&self.ime_input_callback);

        // Preserve raw contacts so GPUI can arbitrate long presses, claimed
        // control drags and ordinary pans, including hover suppression and inertia.
        {
            let cb = Arc::clone(&input_cb);
            let window = Arc::downgrade(&self.window);
            let mut next_id = 0u64;
            let mut active_touches = HashMap::new();
            self.window.on_touch(move |touch| {
                let phase = match touch.action {
                    0 => gpui::TouchPhase::Started,
                    1 => gpui::TouchPhase::Ended,
                    2 => gpui::TouchPhase::Moved,
                    3 => gpui::TouchPhase::Cancelled,
                    _ => return,
                };
                let id = if phase == gpui::TouchPhase::Started {
                    // Android pointer IDs are reused after release.
                    let id = gpui::TouchId(next_id);
                    next_id = next_id.checked_add(1).expect("touch ID exhausted");
                    active_touches.insert(touch.id, id);
                    id
                } else {
                    let Some(id) = active_touches.get(&touch.id).copied() else {
                        return;
                    };
                    id
                };
                let Some(window) = window.upgrade() else {
                    return;
                };
                let scale = window.scale_factor();
                cb.lock()(gpui::PlatformInput::Touch(gpui::TouchEvent {
                    id,
                    phase,
                    position: gpui::point(gpui::px(touch.x / scale), gpui::px(touch.y / scale)),
                    predicted_position: None,
                    force: None,
                }));
                if matches!(phase, gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled) {
                    active_touches.remove(&touch.id);
                }
            });
        }

        // ── Key events → PlatformInput ───────────────────────────────────
        {
            let cb = Arc::clone(&input_cb);
            self.window.on_key_event(move |key_event| {
                use crate::android::keyboard::{
                    android_key_to_keystroke, AKEY_EVENT_ACTION_DOWN, AKEY_EVENT_ACTION_UP,
                };

                // On KeyDown, dispatch text through the global callback so
                // custom TextInput components (PENDING_TEXT) receive it.
                if key_event.action == AKEY_EVENT_ACTION_DOWN {
                    match key_event.key_code {
                        67 => crate::dispatch_text_input("\x08"), // KEYCODE_DEL (backspace)
                        21 => crate::dispatch_text_input("\x1b[D"), // DPAD_LEFT
                        22 => crate::dispatch_text_input("\x1b[C"), // DPAD_RIGHT
                        122 => crate::dispatch_text_input("\x1b[H"), // MOVE_HOME
                        123 => crate::dispatch_text_input("\x1b[F"), // MOVE_END
                        _ => {
                            if key_event.unicode_char != 0 {
                                if let Some(c) = char::from_u32(key_event.unicode_char) {
                                    let s = c.to_string();
                                    crate::dispatch_text_input(&s);
                                }
                            }
                            false
                        }
                    };
                }

                let keystroke = match android_key_to_keystroke(
                    key_event.key_code,
                    key_event.meta_state,
                    key_event.unicode_char,
                ) {
                    Some(ks) => ks,
                    None => return, // Modifier-only or unmapped key
                };

                let event = if key_event.action == AKEY_EVENT_ACTION_DOWN {
                    gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                        keystroke,
                        is_held: false,
                        prefer_character_input: key_event.unicode_char != 0,
                    })
                } else if key_event.action == AKEY_EVENT_ACTION_UP {
                    gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke })
                } else {
                    return; // ACTION_MULTIPLE or unknown
                };

                let mut guard = cb.lock();
                let _ = guard(event);
            });
        }
    }

    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        // Wire the callback to AndroidWindow's active status change.
        // PlatformWindow gives us Box<dyn FnMut(bool)> (not Send).
        // AndroidWindow::on_active_status_change requires Send.
        // On Android, this callback is always invoked on the main thread.
        let send_callback: Box<dyn FnMut(bool) + Send> = unsafe { std::mem::transmute(callback) };
        let send_callback = Mutex::new(send_callback);
        self.window.on_active_status_change(move |active| {
            let mut cb = send_callback.lock();
            cb(active);
        });
    }

    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>) {
        let send_callback: Box<dyn FnMut(WindowVisibility) + Send> =
            unsafe { std::mem::transmute(callback) };
        let send_callback = Mutex::new(send_callback);
        self.window.on_visibility_change(move |visible| {
            let mut cb = send_callback.lock();
            cb(if visible {
                WindowVisibility::Visible
            } else {
                WindowVisibility::Hidden
            });
        });
    }

    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        let _callback = Mutex::new(callback);
        // No hover concept on touch devices
    }

    fn on_resize(&self, callback: Box<dyn FnMut(gpui::Size<gpui::Pixels>, f32)>) {
        // PlatformWindow::on_resize gives us Box<dyn FnMut(...)> (not Send).
        // AndroidWindow::on_resize requires Send.  On Android the resize
        // callback is always invoked on the main thread, so this is safe.
        let send_callback: Box<dyn FnMut(gpui::Size<gpui::Pixels>, f32) + Send> =
            unsafe { std::mem::transmute(callback) };
        let send_callback = Arc::new(Mutex::new(send_callback));
        self.window.on_resize(move |device_size, scale| {
            let mut cb = send_callback.lock();
            cb(
                gpui::size(
                    gpui::px(device_size.width.0 as f32 / scale),
                    gpui::px(device_size.height.0 as f32 / scale),
                ),
                scale,
            );
        });
    }

    fn on_moved(&self, _callback: Box<dyn FnMut()>) {
        // Android windows don't move — they're always fullscreen.
    }

    fn on_should_close(&self, _callback: Box<dyn FnMut() -> bool>) {
        // Android app lifecycle is managed by the system.
    }

    fn on_hit_test_window_control(&self, _callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        // No window controls on Android.
    }

    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        // PlatformWindow gives us Box<dyn FnOnce()> (not Send).
        // AndroidWindow::on_close requires Send.  On Android, the close
        // callback is always invoked on the main thread, so this transmute
        // is safe in practice.
        let send_callback: Box<dyn FnOnce() + Send + 'static> =
            unsafe { std::mem::transmute::<Box<dyn FnOnce()>, Box<dyn FnOnce() + Send>>(callback) };
        self.window.on_close(send_callback);
    }

    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        // Wire to system dark mode changes via Configuration.
        // PlatformWindow gives us Box<dyn FnMut()> (not Send).
        // AndroidWindow::on_appearance_changed requires Send.
        // On Android, this callback is always invoked on the main thread.
        let send_callback: Box<dyn FnMut() + Send> = unsafe { std::mem::transmute(callback) };
        let send_callback = Mutex::new(send_callback);
        self.window.on_appearance_changed(move |_appearance| {
            let mut cb = send_callback.lock();
            cb();
        });
    }

    fn draw(&self, scene: &gpui::Scene) {
        // gpui_wgpu::WgpuRenderer natively consumes gpui::Scene — no bridging needed.
        log::trace!(
            "AndroidPlatformWindow::draw — {} quads, {} shadows",
            scene.quads.len(),
            scene.shadows.len(),
        );

        self.window.draw(scene);
    }

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.window
            .sprite_atlas()
            .unwrap_or_else(|| Arc::new(FallbackAtlas::new()))
    }

    fn is_subpixel_rendering_supported(&self) -> bool {
        self.window.supports_subpixel_aa()
    }

    fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.window.gpu_specs()
    }

    fn update_ime_position(&self, bounds: gpui::Bounds<gpui::Pixels>) {
        // Update the IME candidate window position via JNI.
        // Calls InputMethodManager.updateCursorAnchorInfo() with a
        // CursorAnchorInfo built from the given bounds.
        // Requires API level 21+ (Lollipop).

        use crate::android::jni as jni_helpers;
        use jni::objects::JValue;

        let x: f32 = bounds.origin.x.into();
        let y: f32 = bounds.origin.y.into();
        let h: f32 = bounds.size.height.into();

        let _ = jni_helpers::with_env(|env| {
            let activity = jni_helpers::activity(env)?;

            // 1. Get InputMethodManager
            let service_name = env.new_string("input_method").map_err(|e| e.to_string())?;
            let imm = env
                .call_method(
                    &activity,
                    jni::jni_str!("getSystemService"),
                    jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                    &[JValue::Object(&service_name)],
                )
                .and_then(|v| v.l())
                .map_err(|e| {
                    env.exception_clear();
                    e.to_string()
                })?;
            if imm.is_null() {
                return Err("getSystemService returned null".to_string());
            }

            // 2. Build CursorAnchorInfo
            let builder = env
                .new_object(
                    jni::jni_str!("android/view/inputmethod/CursorAnchorInfo$Builder"),
                    jni::jni_sig!("()V"),
                    &[],
                )
                .map_err(|e| {
                    env.exception_clear();
                    e.to_string()
                })?;

            let _ = env.call_method(
                &builder,
                jni::jni_str!("setInsertionMarkerLocation"),
                jni::jni_sig!("(FFFFI)Landroid/view/inputmethod/CursorAnchorInfo$Builder;"),
                &[
                    JValue::Float(x),
                    JValue::Float(y),
                    JValue::Float(y + h * 0.8),
                    JValue::Float(y + h),
                    JValue::Int(0),
                ],
            );
            env.exception_clear();

            let anchor_info = env
                .call_method(
                    &builder,
                    jni::jni_str!("build"),
                    jni::jni_sig!("()Landroid/view/inputmethod/CursorAnchorInfo;"),
                    &[],
                )
                .and_then(|v| v.l())
                .map_err(|e| {
                    env.exception_clear();
                    e.to_string()
                })?;
            if anchor_info.is_null() {
                return Err("CursorAnchorInfo.build() returned null".to_string());
            }

            // 3. Get decor view: activity.getWindow().getDecorView()
            let window = env
                .call_method(
                    &activity,
                    jni::jni_str!("getWindow"),
                    jni::jni_sig!("()Landroid/view/Window;"),
                    &[],
                )
                .and_then(|v| v.l())
                .map_err(|e| {
                    env.exception_clear();
                    e.to_string()
                })?;
            if window.is_null() {
                return Err("getWindow() returned null".to_string());
            }

            let decor_view = env
                .call_method(
                    &window,
                    jni::jni_str!("getDecorView"),
                    jni::jni_sig!("()Landroid/view/View;"),
                    &[],
                )
                .and_then(|v| v.l())
                .map_err(|e| {
                    env.exception_clear();
                    e.to_string()
                })?;
            if decor_view.is_null() {
                return Err("getDecorView() returned null".to_string());
            }

            // 4. imm.updateCursorAnchorInfo(view, info)
            let _ = env.call_method(
                &imm,
                jni::jni_str!("updateCursorAnchorInfo"),
                jni::jni_sig!("(Landroid/view/View;Landroid/view/inputmethod/CursorAnchorInfo;)V"),
                &[JValue::Object(&decor_view), JValue::Object(&anchor_info)],
            );
            env.exception_clear();

            log::trace!("update_ime_position: x={:.0} y={:.0} h={:.0}", x, y, h);

            Ok(())
        });
    }
}

// ── Fallback atlas ────────────────────────────────────────────────────────────

/// A minimal fallback `PlatformAtlas` used only when the renderer is not
/// available (e.g. before the wgpu surface is created).  Once the
/// `gpui_wgpu::WgpuRenderer` is initialised, `sprite_atlas()` returns the
/// real `WgpuAtlas` from the renderer instead.
struct FallbackAtlas {
    state: Mutex<FallbackAtlasState>,
}

struct FallbackAtlasState {
    next_id: u32,
    tiles: HashMap<AtlasKey, AtlasTile>,
}

impl FallbackAtlas {
    fn new() -> Self {
        Self {
            state: Mutex::new(FallbackAtlasState {
                next_id: 1,
                tiles: HashMap::new(),
            }),
        }
    }
}

impl PlatformAtlas for FallbackAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> anyhow::Result<
            Option<(gpui::Size<gpui::DevicePixels>, std::borrow::Cow<'a, [u8]>)>,
        >,
    ) -> anyhow::Result<Option<AtlasTile>> {
        let mut state = self.state.lock();

        if let Some(tile) = state.tiles.get(key) {
            return Ok(Some(tile.clone()));
        }

        let data = build()?;
        if let Some((size, _pixels)) = data {
            let id = state.next_id;
            state.next_id += 1;

            let tile = AtlasTile {
                texture_id: gpui::AtlasTextureId {
                    index: 0,
                    kind: gpui::AtlasTextureKind::Monochrome,
                },
                tile_id: gpui::TileId(id),
                padding: 0,
                bounds: gpui::Bounds {
                    origin: gpui::point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
                    size,
                },
            };

            state.tiles.insert(key.clone(), tile.clone());
            Ok(Some(tile))
        } else {
            Ok(None)
        }
    }

    fn remove(&self, key: &AtlasKey) {
        self.state.lock().tiles.remove(key);
    }
}

// ── WindowList helper ─────────────────────────────────────────────────────────

/// Tracks all live `AndroidWindow` instances in the process.
///
/// On a typical Android device there is at most one window, but the list
/// supports more for forward compatibility.
#[derive(Default)]
pub struct WindowList {
    windows: Vec<Arc<AndroidWindow>>,
}

impl WindowList {
    /// Add a window to the list.
    pub fn push(&mut self, window: Arc<AndroidWindow>) {
        self.windows.push(window);
    }

    /// Remove and return the window with the given `id`, if present.
    pub fn remove(&mut self, id: u64) -> Option<Arc<AndroidWindow>> {
        if let Some(pos) = self.windows.iter().position(|w| w.id() == id) {
            Some(self.windows.remove(pos))
        } else {
            None
        }
    }

    /// Find a window by id (shared reference).
    pub fn get(&self, id: u64) -> Option<&Arc<AndroidWindow>> {
        self.windows.iter().find(|w| w.id() == id)
    }

    /// Returns the first (primary) window, if any.
    pub fn primary(&self) -> Option<&Arc<AndroidWindow>> {
        self.windows.first()
    }

    /// Iterate over all windows.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<AndroidWindow>> {
        self.windows.iter()
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_window_geometry() {
        let w = AndroidWindow::headless(1080, 1920, 3.0);
        assert_eq!(w.physical_size().width, DevicePixels(1080));
        assert_eq!(w.physical_size().height, DevicePixels(1920));
        assert!((w.scale_factor() - 3.0).abs() < f32::EPSILON);
        assert!(!w.has_surface());
        assert!(!w.is_active());
    }

    #[test]
    fn headless_window_logical_size() {
        let w = AndroidWindow::headless(1080, 1920, 3.0);
        let ls = w.logical_size();
        assert!((ls.width.0 - 360.0).abs() < f32::EPSILON);
        assert!((ls.height.0 - 640.0).abs() < f32::EPSILON);
    }

    #[test]
    fn headless_window_bounds_origin_is_zero() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        let b = w.bounds();
        assert_eq!(b.origin.x, DevicePixels(0));
        assert_eq!(b.origin.y, DevicePixels(0));
    }

    #[test]
    fn window_id_is_stable() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        assert_eq!(w.id(), w.id());
    }

    #[test]
    fn window_appearance_defaults_to_light() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        assert_eq!(w.appearance(), WindowAppearance::Light);
    }

    #[test]
    fn window_set_appearance_dark() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        w.set_appearance(WindowAppearance::Dark);
        assert_eq!(w.appearance(), WindowAppearance::Dark);
    }

    #[test]
    fn window_appearance_callback_fires() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);

        let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fired_clone = fired.clone();

        w.on_appearance_changed(move |_| {
            fired_clone.store(true, std::sync::atomic::Ordering::Relaxed);
        });

        w.set_appearance(WindowAppearance::Dark);
        assert!(fired.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn window_appearance_callback_not_fired_if_unchanged() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        // Default is Light; setting Light again should not fire.
        let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fired_clone = fired.clone();

        w.on_appearance_changed(move |_| {
            fired_clone.store(true, std::sync::atomic::Ordering::Relaxed);
        });

        w.set_appearance(WindowAppearance::Light);
        assert!(!fired.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn touch_callback_fires() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        let received = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<TouchPoint>::new()));
        let r2 = received.clone();

        w.on_touch(move |pt| {
            r2.lock().push(pt);
        });

        w.handle_touch(TouchPoint {
            id: 1,
            x: 100.0,
            y: 200.0,
            action: 0,
        });

        let pts = received.lock();
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].id, 1);
    }

    #[test]
    fn key_callback_fires() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        let received = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<AndroidKeyEvent>::new()));
        let r2 = received.clone();

        w.on_key_event(move |e| {
            r2.lock().push(e);
        });

        w.handle_key_event(AndroidKeyEvent {
            key_code: 29, // KEYCODE_A
            action: 0,
            meta_state: 0,
            unicode_char: b'a' as u32,
        });

        let events = received.lock();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].key_code, 29);
    }

    #[test]
    fn request_frame_callback_fires() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c2 = count.clone();

        w.on_request_frame(move || {
            c2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });

        w.request_frame();
        w.request_frame();

        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn window_list_push_get_remove() {
        let mut list = WindowList::default();
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        let id = w.id();

        list.push(w);
        assert_eq!(list.len(), 1);
        assert!(list.get(id).is_some());

        let removed = list.remove(id);
        assert!(removed.is_some());
        assert!(list.is_empty());
        assert!(list.get(id).is_none());
    }

    #[test]
    fn window_list_primary() {
        let mut list = WindowList::default();
        list.push(AndroidWindow::headless(1080, 1920, 2.0));
        assert!(list.primary().is_some());
    }

    #[test]
    fn window_list_remove_missing_returns_none() {
        let mut list = WindowList::default();
        assert!(list.remove(0xDEADBEEF).is_none());
    }

    #[test]
    fn subpixel_aa_false_for_headless() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        assert!(!w.supports_subpixel_aa());
    }

    #[test]
    fn gpu_specs_none_for_headless() {
        let w = AndroidWindow::headless(1080, 1920, 2.0);
        assert!(w.gpu_specs().is_none());
    }

    fn touch_input_harness() -> (
        Arc<AndroidWindow>,
        AndroidPlatformWindow,
        Arc<Mutex<Vec<gpui::PlatformInput>>>,
    ) {
        let window = AndroidWindow::headless(400, 800, 2.0);
        let platform = AndroidPlatformWindow::new(window.clone(), None);
        let events = Arc::new(Mutex::new(Vec::new()));
        platform.on_input(Box::new({
            let events = events.clone();
            move |event| {
                events.lock().push(event);
                DispatchEventResult::default()
            }
        }));
        platform.on_request_frame(Box::new(|_| {}));
        (window, platform, events)
    }

    fn send_touch(window: &AndroidWindow, action: u32, y: f32) {
        window.handle_touch(TouchPoint {
            id: 1,
            x: 100.,
            y,
            action,
        });
    }

    #[test]
    fn touch_contacts_preserve_phases_and_logical_coordinates() {
        let (window, _platform, events) = touch_input_harness();
        send_touch(&window, 0, 200.);
        send_touch(&window, 2, 240.);
        send_touch(&window, 1, 242.);
        let events = events.lock();
        assert_eq!(events.len(), 3);
        let contacts = events
            .iter()
            .map(|event| {
                let gpui::PlatformInput::Touch(touch) = event else {
                    panic!("touch must not be downgraded to mouse or scroll");
                };
                touch
            })
            .collect::<Vec<_>>();
        assert_eq!(contacts[0].phase, gpui::TouchPhase::Started);
        assert_eq!(contacts[1].phase, gpui::TouchPhase::Moved);
        assert_eq!(contacts[2].phase, gpui::TouchPhase::Ended);
        assert_eq!(
            contacts[0].position,
            gpui::point(gpui::px(50.), gpui::px(100.))
        );
        assert_eq!(contacts[1].position.y, gpui::px(120.));
        assert_eq!(contacts[2].position.y, gpui::px(121.));
        assert!(contacts.iter().all(|touch| touch.id == contacts[0].id));
    }

    #[test]
    fn touch_cancel_is_forwarded_and_discards_stray_moves() {
        let (window, _platform, events) = touch_input_harness();
        send_touch(&window, 0, 200.);
        send_touch(&window, 2, 250.);
        send_touch(&window, 3, 250.);
        send_touch(&window, 2, 280.);
        send_touch(&window, 1, 280.);
        window.request_frame();
        let events = events.lock();
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[2], gpui::PlatformInput::Touch(event)
            if event.phase == gpui::TouchPhase::Cancelled));
    }

    #[test]
    fn touch_pointer_ids_are_not_reused_across_contacts() {
        let (window, _platform, events) = touch_input_harness();
        send_touch(&window, 0, 200.);
        window.handle_touch(TouchPoint {
            id: 2,
            x: 120.,
            y: 220.,
            action: 0,
        });
        send_touch(&window, 1, 200.);
        send_touch(&window, 0, 200.);
        let events = events.lock();
        let ids = events
            .iter()
            .map(|event| {
                let gpui::PlatformInput::Touch(event) = event else {
                    panic!("expected touch")
                };
                event.id
            })
            .collect::<Vec<_>>();
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids[0], ids[2]);
        assert_ne!(ids[0], ids[3]);
        assert_ne!(ids[1], ids[3]);
    }
}
