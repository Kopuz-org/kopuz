//! Embedded maps for Android and iOS.
//!
//! Provides a cross-platform embedded map view backed by:
//! - Android: Android MapView (via platform view system)
//! - iOS: MKMapView via Objective-C (via platform view system)
//!
//! Unlike `maps_launcher` which opens an external app, this package
//! embeds an interactive map directly in the GPUI render tree.
//!
//! Feature-gated behind `maps`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

use crate::platform_view::{
    PlatformViewBounds, PlatformViewHandle, PlatformViewParams, PlatformViewRegistry,
};
use std::sync::Arc;

/// Geographic coordinate.
#[derive(Debug, Clone, Copy)]
pub struct LatLng {
    pub latitude: f64,
    pub longitude: f64,
}

/// Map type / style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapType {
    /// Standard road map.
    Normal,
    /// Satellite imagery.
    Satellite,
    /// Satellite with road overlays.
    Hybrid,
    /// Terrain/topographic.
    Terrain,
}

/// Configuration for creating a map view.
#[derive(Debug, Clone)]
pub struct MapSettings {
    /// Initial center coordinate.
    pub center: LatLng,
    /// Initial zoom level (0-20, higher = closer).
    pub zoom: f64,
    /// Map type/style.
    pub map_type: MapType,
    /// Enable zoom gestures.
    pub zoom_gestures_enabled: bool,
    /// Enable scroll/pan gestures.
    pub scroll_gestures_enabled: bool,
    /// Enable rotate gestures.
    pub rotate_gestures_enabled: bool,
    /// Show the user's location (requires permission).
    pub my_location_enabled: bool,
}

impl Default for MapSettings {
    fn default() -> Self {
        Self {
            center: LatLng {
                latitude: 37.7749,
                longitude: -122.4194,
            },
            zoom: 12.0,
            map_type: MapType::Normal,
            zoom_gestures_enabled: true,
            scroll_gestures_enabled: true,
            rotate_gestures_enabled: true,
            my_location_enabled: false,
        }
    }
}

/// A map marker/pin.
#[derive(Debug, Clone)]
pub struct MapMarker {
    /// Unique identifier for this marker.
    pub id: String,
    /// Position of the marker.
    pub position: LatLng,
    /// Title shown in the info window.
    pub title: Option<String>,
    /// Snippet/subtitle shown in the info window.
    pub snippet: Option<String>,
}

/// Handle to an embedded map view.
pub struct MapView {
    /// Platform view handle for the map.
    platform_handle: Option<Arc<PlatformViewHandle>>,
    /// Current settings.
    settings: MapSettings,
}

impl std::fmt::Debug for MapView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapView")
            .field("has_handle", &self.platform_handle.is_some())
            .field("center", &self.settings.center)
            .field("zoom", &self.settings.zoom)
            .finish()
    }
}

/// Register the "map" platform view factory.
fn ensure_factory_registered() {
    let registry = PlatformViewRegistry::global();
    if !registry.has_factory("map") {
        #[cfg(target_os = "android")]
        {
            use crate::android::platform_view::AndroidPlatformViewFactory;
            registry.register("map", Box::new(AndroidPlatformViewFactory::new("map")));
        }
        #[cfg(target_os = "ios")]
        {
            use crate::ios::platform_view::IosPlatformViewFactory;
            registry.register("map", Box::new(IosPlatformViewFactory::new("map")));
        }
    }
}

impl MapView {
    /// Create and display an embedded map view.
    pub fn new(settings: MapSettings) -> Result<Self, String> {
        ensure_factory_registered();

        let mut creation_params = std::collections::HashMap::new();
        creation_params.insert("latitude".to_string(), settings.center.latitude.to_string());
        creation_params.insert(
            "longitude".to_string(),
            settings.center.longitude.to_string(),
        );
        creation_params.insert("zoom".to_string(), settings.zoom.to_string());
        creation_params.insert(
            "map_type".to_string(),
            (settings.map_type as i32).to_string(),
        );
        creation_params.insert(
            "zoom_gestures".to_string(),
            settings.zoom_gestures_enabled.to_string(),
        );
        creation_params.insert(
            "scroll_gestures".to_string(),
            settings.scroll_gestures_enabled.to_string(),
        );
        creation_params.insert(
            "rotate_gestures".to_string(),
            settings.rotate_gestures_enabled.to_string(),
        );
        creation_params.insert(
            "my_location".to_string(),
            settings.my_location_enabled.to_string(),
        );

        let params = PlatformViewParams {
            bounds: PlatformViewBounds::default(),
            creation_params,
        };

        let handle = PlatformViewRegistry::global().create_view("map", params)?;
        Ok(Self {
            platform_handle: Some(Arc::new(handle)),
            settings,
        })
    }

    /// Get the platform view handle for embedding in a GPUI element.
    pub fn platform_view_handle(&self) -> Option<Arc<PlatformViewHandle>> {
        self.platform_handle.clone()
    }

    /// Set the map center coordinate.
    pub fn set_center(&mut self, center: LatLng) -> Result<(), String> {
        self.settings.center = center;
        #[cfg(target_os = "ios")]
        {
            ios::set_center(center)
        }
        #[cfg(target_os = "android")]
        {
            android::set_center(center)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Ok(())
        }
    }

    /// Set the zoom level.
    pub fn set_zoom(&mut self, zoom: f64) -> Result<(), String> {
        self.settings.zoom = zoom;
        #[cfg(target_os = "ios")]
        {
            ios::set_zoom(zoom)
        }
        #[cfg(target_os = "android")]
        {
            android::set_zoom(zoom)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Ok(())
        }
    }

    /// Set the map type/style.
    pub fn set_map_type(&mut self, map_type: MapType) -> Result<(), String> {
        self.settings.map_type = map_type;
        #[cfg(target_os = "ios")]
        {
            ios::set_map_type(map_type)
        }
        #[cfg(target_os = "android")]
        {
            android::set_map_type(map_type)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Ok(())
        }
    }

    /// Add a marker to the map.
    pub fn add_marker(&self, marker: &MapMarker) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::add_marker(marker)
        }
        #[cfg(target_os = "android")]
        {
            android::add_marker(marker)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = marker;
            Ok(())
        }
    }

    /// Remove a marker by ID.
    pub fn remove_marker(&self, marker_id: &str) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::remove_marker(marker_id)
        }
        #[cfg(target_os = "android")]
        {
            android::remove_marker(marker_id)
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            let _ = marker_id;
            Ok(())
        }
    }

    /// Remove all markers from the map.
    pub fn clear_markers(&self) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            ios::clear_markers()
        }
        #[cfg(target_os = "android")]
        {
            android::clear_markers()
        }
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            Ok(())
        }
    }

    /// Dispose of the map view and release resources.
    pub fn dispose(&mut self) {
        if let Some(h) = self.platform_handle.take() {
            h.dispose();
        }
    }
}

impl Drop for MapView {
    fn drop(&mut self) {
        self.dispose();
    }
}
