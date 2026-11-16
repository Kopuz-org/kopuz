use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

pub fn open_coordinates(
    latitude: f64,
    longitude: f64,
    label: Option<&str>,
) -> Result<bool, String> {
    let label = label.map(|s| s.to_owned());
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        let uri_str = match &label {
            Some(l) => format!(
                "geo:{},{}?q={},{}({})",
                latitude,
                longitude,
                latitude,
                longitude,
                percent_encode(l)
            ),
            None => format!(
                "geo:{},{}?q={},{}",
                latitude, longitude, latitude, longitude
            ),
        };

        let intent = create_geo_intent(env, &uri_str)?;

        // Add FLAG_ACTIVITY_NEW_TASK
        add_new_task_flag(env, &intent)?;

        match env.call_method(
            &activity,
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        ) {
            Ok(_) => Ok(true),
            Err(_) => {
                env.exception_clear();
                Ok(false)
            }
        }
    })
}

pub fn open_query(query: &str) -> Result<bool, String> {
    let query = query.to_owned();
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        let uri_str = format!("geo:0,0?q={}", percent_encode(&query));

        let intent = create_geo_intent(env, &uri_str)?;
        add_new_task_flag(env, &intent)?;

        match env.call_method(
            &activity,
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        ) {
            Ok(_) => Ok(true),
            Err(_) => {
                env.exception_clear();
                Ok(false)
            }
        }
    })
}

pub fn open_directions(
    dest_latitude: f64,
    dest_longitude: f64,
    dest_label: Option<&str>,
) -> Result<bool, String> {
    let dest_label = dest_label.map(|s| s.to_owned());
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        // Try Google Maps navigation first
        let dest = match &dest_label {
            Some(l) => format!("{}@{},{}", percent_encode(l), dest_latitude, dest_longitude),
            None => format!("{},{}", dest_latitude, dest_longitude),
        };
        let nav_uri = format!("google.navigation:q={}", dest);

        let intent = create_geo_intent(env, &nav_uri)?;
        add_new_task_flag(env, &intent)?;

        // Set package to Google Maps
        let pkg = env.new_string("com.google.android.apps.maps").e()?;
        let _ = env
            .call_method(
                &intent,
                jni::jni_str!("setPackage"),
                jni::jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
                &[JValue::Object(&pkg)],
            )
            .e()?;

        match env.call_method(
            &activity,
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        ) {
            Ok(_) => Ok(true),
            Err(_) => {
                env.exception_clear();
                // Fallback to generic geo intent
                let fallback_uri = format!(
                    "geo:{},{}?q={},{}",
                    dest_latitude, dest_longitude, dest_latitude, dest_longitude
                );
                let fallback_intent = create_geo_intent(env, &fallback_uri)?;
                add_new_task_flag(env, &fallback_intent)?;

                match env.call_method(
                    &activity,
                    jni::jni_str!("startActivity"),
                    jni::jni_sig!("(Landroid/content/Intent;)V"),
                    &[JValue::Object(&fallback_intent)],
                ) {
                    Ok(_) => Ok(true),
                    Err(_) => {
                        env.exception_clear();
                        Ok(false)
                    }
                }
            }
        }
    })
}

pub fn is_available() -> Result<bool, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        let intent = create_geo_intent(env, "geo:0,0")?;

        // activity.getPackageManager()
        let pm = env
            .call_method(
                &activity,
                jni::jni_str!("getPackageManager"),
                jni::jni_sig!("()Landroid/content/pm/PackageManager;"),
                &[],
            )
            .and_then(|v| v.l())
            .e()?;
        if pm.is_null() {
            return Err("getPackageManager returned null".into());
        }

        // pm.resolveActivity(intent, 0)
        let resolved = env
            .call_method(
                &pm,
                jni::jni_str!("resolveActivity"),
                jni::jni_sig!("(Landroid/content/Intent;I)Landroid/content/pm/ResolveInfo;"),
                &[JValue::Object(&intent), JValue::Int(0)],
            )
            .and_then(|v| v.l());

        match resolved {
            Ok(r) => Ok(!r.is_null()),
            Err(_) => {
                env.exception_clear();
                Ok(false)
            }
        }
    })
}

/// Create an Intent(ACTION_VIEW, Uri.parse(uri_str)).
fn create_geo_intent<'local>(
    env: &mut jni::Env<'local>,
    uri_str: &str,
) -> Result<jni::objects::JObject<'local>, String> {
    let jurl = env.new_string(uri_str).e()?;
    let uri = env
        .call_static_method(
            jni::jni_str!("android/net/Uri"),
            jni::jni_str!("parse"),
            jni::jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
            &[JValue::Object(&jurl)],
        )
        .and_then(|v| v.l())
        .e()?;
    if uri.is_null() {
        return Err(format!("Uri.parse returned null for: {uri_str}"));
    }

    let action_view = env.new_string("android.intent.action.VIEW").e()?;
    let intent = env
        .new_object(
            jni::jni_str!("android/content/Intent"),
            jni::jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::Object(&action_view), JValue::Object(&uri)],
        )
        .e()?;

    Ok(intent)
}

/// Add FLAG_ACTIVITY_NEW_TASK to an intent.
fn add_new_task_flag(
    env: &mut jni::Env<'_>,
    intent: &jni::objects::JObject<'_>,
) -> Result<(), String> {
    // FLAG_ACTIVITY_NEW_TASK = 0x10000000
    let _ = env
        .call_method(
            intent,
            jni::jni_str!("addFlags"),
            jni::jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::Int(0x10000000)],
        )
        .e()?;
    Ok(())
}

/// Simple percent-encoding for URI components.
fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => {
                result.push_str("%20");
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}
