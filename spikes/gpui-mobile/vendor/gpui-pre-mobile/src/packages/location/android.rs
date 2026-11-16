use super::{LocationAccuracy, LocationSettings, Position};
use crate::android::jni::{self as jni_helpers};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiLocation";

pub fn is_location_service_enabled() -> Result<bool, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("isLocationEnabled"),
                jni::jni_sig!("(Landroid/app/Activity;)Z"),
                &[JValue::Object(&activity)],
            )
            .and_then(|v| v.z())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(result)
    })
}

pub fn get_current_position(settings: &LocationSettings) -> Result<Position, String> {
    let accuracy_int = accuracy_to_int(settings.accuracy);

    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getCurrentPosition"),
                jni::jni_sig!("(Landroid/app/Activity;I)Ljava/lang/String;"),
                &[JValue::Object(&activity), JValue::Int(accuracy_int)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Err("Failed to get current position".into());
        }

        let data = jni_helpers::get_string(env, &result);
        parse_position_string(&data)
    })
}

pub fn get_last_known_position() -> Result<Option<Position>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getLastKnownPosition"),
                jni::jni_sig!("(Landroid/app/Activity;)Ljava/lang/String;"),
                &[JValue::Object(&activity)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Ok(None);
        }

        let data = jni_helpers::get_string(env, &result);
        if data.is_empty() {
            return Ok(None);
        }
        parse_position_string(&data).map(Some)
    })
}

/// Parse a pipe-delimited position string:
/// "lat|lon|alt|accuracy|speed|speedAccuracy|heading|headingAccuracy|timestamp"
fn parse_position_string(data: &str) -> Result<Position, String> {
    let parts: Vec<&str> = data.split('|').collect();
    if parts.len() < 9 {
        return Err(format!(
            "Expected 9 pipe-delimited values in position string, got {}",
            parts.len()
        ));
    }

    let parse = |i: usize, name: &str| -> Result<f64, String> {
        parts[i]
            .parse::<f64>()
            .map_err(|e| format!("Failed to parse {}: {}", name, e))
    };

    Ok(Position {
        latitude: parse(0, "latitude")?,
        longitude: parse(1, "longitude")?,
        altitude: parse(2, "altitude")?,
        accuracy: parse(3, "accuracy")?,
        speed: parse(4, "speed")?,
        speed_accuracy: parse(5, "speed_accuracy")?,
        heading: parse(6, "heading")?,
        heading_accuracy: parse(7, "heading_accuracy")?,
        timestamp: parse(8, "timestamp")? as u64,
    })
}

fn accuracy_to_int(accuracy: LocationAccuracy) -> i32 {
    match accuracy {
        LocationAccuracy::Lowest => 0,
        LocationAccuracy::Low => 1,
        LocationAccuracy::Medium => 2,
        LocationAccuracy::High => 3,
        LocationAccuracy::Best => 4,
        LocationAccuracy::BestForNavigation => 5,
    }
}
