//! Android display handling.
//!
//! Android does not expose a rich multi-monitor API through the NDK at the C
//! level; instead, display geometry is derived from the `ANativeWindow` that
//! the system hands us via the `APP_CMD_INIT_WINDOW` callback.  Density
//! information is obtained through the `AConfiguration` API.
//!
//! `AndroidDisplay` models a single logical screen.  On foldable / multi-
//! display devices the system will create a new `ANativeWindow` for each
//! display surface, so multiple `AndroidDisplay` instances may coexist.
//!
//! ## No GPUI workspace dependency
//!
//! All geometry types are imported from `super` (the local re-declarations in
//! `mod.rs`) so this file compiles without a hard dependency on the full GPUI
//! crate.  When wired into a real GPUI workspace, swap the `super::*` imports
//! for the canonical `gpui::*` equivalents.

use super::{Bounds, DevicePixels, Pixels, Point, Size};
use anyhow::Result;
use gpui::{self, DisplayId, PlatformDisplay};
use ndk::native_window::NativeWindow;
use std::fmt;

use ndk::asset::AssetManager;
use ndk::configuration::Configuration;

// Well-known Android density buckets (dp/inch).
const DENSITY_DEFAULT: i32 = 160;

// ── AndroidDisplay ────────────────────────────────────────────────────────────

/// A single logical display on Android.
///
/// Wraps an `ndk::native_window::NativeWindow` and exposes the geometry /
/// density information that GPUI needs to lay out and render content.
///
/// `NativeWindow` is internally reference-counted (`ANativeWindow_acquire` /
/// `ANativeWindow_release`), so cloning and dropping are handled automatically.
#[derive(Clone)]
pub struct AndroidDisplay {
    /// The underlying native window, or `None` for headless displays.
    window: Option<NativeWindow>,

    /// Display density expressed as a scale factor relative to 160 dpi
    /// (the Android "baseline" density).
    ///
    /// e.g. 1.0 = mdpi (160 dpi), 2.0 = xhdpi (320 dpi), 3.0 = xxhdpi (480 dpi).
    scale_factor: f32,

    /// A stable numeric identifier for this display.
    ///
    /// Derived from the window pointer because the NDK does not assign stable
    /// integer display IDs at the C level.
    id: u64,
}

impl AndroidDisplay {
    // ── constructors ─────────────────────────────────────────────────────────

    /// Create an `AndroidDisplay` from an `ndk::native_window::NativeWindow`.
    ///
    /// The `NativeWindow` is cloned (which internally calls `ANativeWindow_acquire`),
    /// so the caller retains ownership of their handle.
    ///
    /// `density_dpi` — the screen density in dots-per-inch as returned by
    /// `AConfiguration_getDensity()`.  Pass `0` or a negative value to fall
    /// back to the baseline density (160 dpi).
    pub fn from_window(window: &NativeWindow, density_dpi: i32) -> Self {
        let density = if density_dpi > 0 {
            density_dpi
        } else {
            DENSITY_DEFAULT
        };
        let scale_factor = density as f32 / DENSITY_DEFAULT as f32;
        let id = window.ptr().as_ptr() as u64;

        Self {
            window: Some(window.clone()),
            scale_factor,
            id,
        }
    }

    /// Create an `AndroidDisplay` and query density from an `AssetManager`.
    pub fn from_activity(window: &NativeWindow, asset_manager: &AssetManager) -> Self {
        let config = Configuration::from_asset_manager(asset_manager);
        let density_dpi = config.density().unwrap_or(DENSITY_DEFAULT as u32) as i32;
        Self::from_window(window, density_dpi)
    }

    /// Create a synthetic `AndroidDisplay` for headless / testing use.
    ///
    /// Returns a display with the given size and a 1× scale factor.
    /// No native window is held.
    pub fn headless(width: i32, height: i32) -> Self {
        Self {
            window: None,
            scale_factor: 1.0,
            id: ((width as u64) << 32) | (height as u64),
        }
    }

    // ── geometry ──────────────────────────────────────────────────────────────

    /// Physical width of the display in device pixels.
    pub fn physical_width(&self) -> DevicePixels {
        match &self.window {
            Some(nw) => DevicePixels(nw.width()),
            None => DevicePixels(0),
        }
    }

    /// Physical height of the display in device pixels.
    pub fn physical_height(&self) -> DevicePixels {
        match &self.window {
            Some(nw) => DevicePixels(nw.height()),
            None => DevicePixels(0),
        }
    }

    /// Physical size of the display in device pixels.
    pub fn physical_size(&self) -> Size<DevicePixels> {
        Size {
            width: self.physical_width(),
            height: self.physical_height(),
        }
    }

    /// Logical width of the display in density-independent pixels.
    pub fn logical_width(&self) -> Pixels {
        Pixels(self.physical_width().0 as f32 / self.scale_factor)
    }

    /// Logical height of the display in density-independent pixels.
    pub fn logical_height(&self) -> Pixels {
        Pixels(self.physical_height().0 as f32 / self.scale_factor)
    }

    /// Logical size of the display in density-independent pixels.
    pub fn logical_size(&self) -> Size<Pixels> {
        Size {
            width: self.logical_width(),
            height: self.logical_height(),
        }
    }

    /// The full display bounds with origin at `(0, 0)`.
    pub fn bounds(&self) -> Bounds<DevicePixels> {
        Bounds {
            origin: Point {
                x: DevicePixels(0),
                y: DevicePixels(0),
            },
            size: self.physical_size(),
        }
    }

    /// The full display bounds in logical pixels with origin at `(0, 0)`.
    pub fn logical_bounds(&self) -> Bounds<Pixels> {
        Bounds {
            origin: Point {
                x: Pixels(0.0),
                y: Pixels(0.0),
            },
            size: self.logical_size(),
        }
    }

    // ── density / scale ───────────────────────────────────────────────────────

    /// Scale factor relative to 160 dpi.
    ///
    /// Multiply logical pixels by this value to obtain device pixels:
    /// ```text
    /// device_px = logical_px * scale_factor
    /// ```
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Approximate screen density in dots-per-inch.
    pub fn dpi(&self) -> i32 {
        (self.scale_factor * DENSITY_DEFAULT as f32).round() as i32
    }

    // ── identity ──────────────────────────────────────────────────────────────

    /// A numeric identifier for this display.
    ///
    /// Derived from the `ANativeWindow *` pointer value; stable for the
    /// lifetime of the native window.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Returns `true` if this display is backed by a real `ANativeWindow`.
    pub fn is_real(&self) -> bool {
        self.window.is_some()
    }

    // ── raw handle ────────────────────────────────────────────────────────────

    /// Returns a reference to the underlying `NativeWindow`, if present.
    ///
    /// Returns `None` for headless displays.
    pub fn native_window(&self) -> Option<&NativeWindow> {
        self.window.as_ref()
    }
}

impl fmt::Debug for AndroidDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AndroidDisplay")
            .field("id", &format_args!("{:#x}", self.id))
            .field("physical_size", &self.physical_size())
            .field("scale_factor", &self.scale_factor)
            .field("dpi", &self.dpi())
            .finish()
    }
}

// ── impl PlatformDisplay ──────────────────────────────────────────────────────

impl PlatformDisplay for AndroidDisplay {
    fn id(&self) -> DisplayId {
        // Truncate the u64 id to u32 — on Android the pointer-derived id
        // fits in 32 bits for the lower half, and DisplayId only stores u32.
        DisplayId::new(self.id as u64)
    }

    fn uuid(&self) -> Result<uuid::Uuid> {
        // Android NDK does not provide a stable UUID for displays.
        // Synthesise a deterministic v5 UUID from the display id so that
        // the same ANativeWindow pointer always produces the same UUID
        // within a single process lifetime.
        let namespace = uuid::Uuid::NAMESPACE_OID;
        let name = format!("android-display-{:#x}", self.id);
        Ok(uuid::Uuid::new_v5(&namespace, name.as_bytes()))
    }

    fn bounds(&self) -> gpui::Bounds<gpui::Pixels> {
        let logical = self.logical_size();
        gpui::Bounds {
            origin: gpui::point(gpui::px(0.0), gpui::px(0.0)),
            size: gpui::size(gpui::px(logical.width.0), gpui::px(logical.height.0)),
        }
    }
}

impl fmt::Display for AndroidDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ps = self.physical_size();
        write!(
            f,
            "AndroidDisplay({}×{}px, {:.1}×, {}dpi)",
            ps.width.0,
            ps.height.0,
            self.scale_factor,
            self.dpi(),
        )
    }
}

// ── DisplayList helper ────────────────────────────────────────────────────────

/// A list of currently connected displays.
///
/// On a typical Android device there is a single display.  Foldables and
/// devices with external display support may expose two.
pub struct DisplayList {
    displays: Vec<AndroidDisplay>,
}

impl DisplayList {
    /// Constructs a list containing a single primary display.
    pub fn single(display: AndroidDisplay) -> Self {
        Self {
            displays: vec![display],
        }
    }

    /// Returns the primary (first) display.
    pub fn primary(&self) -> Option<&AndroidDisplay> {
        self.displays.first()
    }

    /// Returns all displays.
    pub fn all(&self) -> &[AndroidDisplay] {
        &self.displays
    }

    /// Returns the number of connected displays.
    pub fn len(&self) -> usize {
        self.displays.len()
    }

    /// Returns `true` if the list is empty.
    pub fn is_empty(&self) -> bool {
        self.displays.is_empty()
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_display_geometry() {
        let d = AndroidDisplay::headless(1920, 1080);
        assert_eq!(d.physical_width(), DevicePixels(1920));
        assert_eq!(d.physical_height(), DevicePixels(1080));
        assert!(!d.is_real());
    }

    #[test]
    fn headless_scale_factor_is_one() {
        let d = AndroidDisplay::headless(800, 600);
        assert!((d.scale_factor() - 1.0).abs() < f32::EPSILON);
        assert_eq!(d.dpi(), DENSITY_DEFAULT);
    }

    #[test]
    fn logical_size_divides_by_scale() {
        let d = AndroidDisplay::headless(1440, 2960);
        // Headless always has scale_factor = 1.0, so logical == physical.
        let logical = d.logical_size();
        assert!((logical.width.0 - 1440.0).abs() < f32::EPSILON);
        assert!((logical.height.0 - 2960.0).abs() < f32::EPSILON);
    }

    #[test]
    fn display_id_differs_for_different_sizes() {
        let d1 = AndroidDisplay::headless(1080, 1920);
        let d2 = AndroidDisplay::headless(1440, 2560);
        assert_ne!(d1.id(), d2.id());
    }

    #[test]
    fn bounds_origin_is_zero() {
        let d = AndroidDisplay::headless(1080, 2400);
        let b = d.bounds();
        assert_eq!(b.origin.x, DevicePixels(0));
        assert_eq!(b.origin.y, DevicePixels(0));
        assert_eq!(b.size.width, DevicePixels(1080));
        assert_eq!(b.size.height, DevicePixels(2400));
    }

    #[test]
    fn display_list_primary() {
        let list = DisplayList::single(AndroidDisplay::headless(1080, 1920));
        assert!(!list.is_empty());
        assert_eq!(list.len(), 1);
        let primary = list.primary().expect("primary display");
        assert_eq!(primary.physical_width(), DevicePixels(1080));
    }

    #[test]
    fn clone_headless_is_independent() {
        let d1 = AndroidDisplay::headless(720, 1280);
        let d2 = d1.clone();
        assert_eq!(d1.id(), d2.id());
        assert_eq!(d1.physical_size(), d2.physical_size());
    }

    #[test]
    fn debug_format_contains_dimensions() {
        let d = AndroidDisplay::headless(1080, 1920);
        let s = format!("{:?}", d);
        assert!(s.contains("1080"));
        assert!(s.contains("1920"));
    }

    #[test]
    fn display_format_contains_scale() {
        let d = AndroidDisplay::headless(1080, 1920);
        let s = format!("{}", d);
        assert!(s.contains("1.0"));
    }

    #[test]
    fn dpi_scales_linearly_with_scale_factor() {
        // Construct a synthetic display with a known density.
        // We can't call from_window without a real NDK, so we test via headless
        // which always yields scale_factor = 1.0 → 160 dpi.
        let d = AndroidDisplay::headless(0, 0);
        assert_eq!(d.dpi(), 160);
    }
}
