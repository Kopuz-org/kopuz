use super::{LatLng, MapMarker, MapType};

// Note: Full Google Maps integration requires the Maps SDK dependency.
// These are placeholder implementations that log the operations.
// When Google Maps SDK is available, these should use the MapView API.

pub fn set_center(center: LatLng) -> Result<(), String> {
    log::info!(
        "maps::android::set_center({}, {})",
        center.latitude,
        center.longitude
    );
    // TODO: Call GpuiMaps.setCenter() when Maps SDK is integrated
    Ok(())
}

pub fn set_zoom(zoom: f64) -> Result<(), String> {
    log::info!("maps::android::set_zoom({})", zoom);
    Ok(())
}

pub fn set_map_type(map_type: MapType) -> Result<(), String> {
    log::info!("maps::android::set_map_type({:?})", map_type);
    Ok(())
}

pub fn add_marker(marker: &MapMarker) -> Result<(), String> {
    log::info!(
        "maps::android::add_marker(id={}, lat={}, lng={})",
        marker.id,
        marker.position.latitude,
        marker.position.longitude
    );
    Ok(())
}

pub fn remove_marker(marker_id: &str) -> Result<(), String> {
    log::info!("maps::android::remove_marker({})", marker_id);
    Ok(())
}

pub fn clear_markers() -> Result<(), String> {
    log::info!("maps::android::clear_markers()");
    Ok(())
}
