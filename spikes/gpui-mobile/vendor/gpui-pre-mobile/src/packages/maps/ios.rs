use super::{LatLng, MapMarker, MapType};

// Note: MKMapView integration requires MapKit framework.
// These are placeholder implementations that log the operations.

pub fn set_center(center: LatLng) -> Result<(), String> {
    log::info!(
        "maps::ios::set_center({}, {})",
        center.latitude,
        center.longitude
    );
    // TODO: Use MKMapView.setCenterCoordinate:animated:
    Ok(())
}

pub fn set_zoom(zoom: f64) -> Result<(), String> {
    log::info!("maps::ios::set_zoom({})", zoom);
    // TODO: Use MKMapView camera altitude or region span
    Ok(())
}

pub fn set_map_type(map_type: MapType) -> Result<(), String> {
    log::info!("maps::ios::set_map_type({:?})", map_type);
    // TODO: Set MKMapView.mapType
    Ok(())
}

pub fn add_marker(marker: &MapMarker) -> Result<(), String> {
    log::info!(
        "maps::ios::add_marker(id={}, lat={}, lng={})",
        marker.id,
        marker.position.latitude,
        marker.position.longitude
    );
    // TODO: Add MKPointAnnotation to MKMapView
    Ok(())
}

pub fn remove_marker(marker_id: &str) -> Result<(), String> {
    log::info!("maps::ios::remove_marker({})", marker_id);
    Ok(())
}

pub fn clear_markers() -> Result<(), String> {
    log::info!("maps::ios::clear_markers()");
    // TODO: Remove all annotations from MKMapView
    Ok(())
}
