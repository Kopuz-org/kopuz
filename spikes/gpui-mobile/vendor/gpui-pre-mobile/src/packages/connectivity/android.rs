use super::ConnectivityStatus;
use crate::android::jni as jni_helpers;
use jni::objects::JValue;

pub fn check_connectivity() -> ConnectivityStatus {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        // context.getSystemService("connectivity") → ConnectivityManager
        let service_name = env.new_string("connectivity").map_err(|e| e.to_string())?;
        let cm = match env
            .call_method(
                &activity,
                jni::jni_str!("getSystemService"),
                jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                &[JValue::Object(&service_name)],
            )
            .and_then(|v| v.l())
        {
            Ok(o) if !o.is_null() => o,
            _ => {
                env.exception_clear();
                return Ok(ConnectivityStatus::None);
            }
        };

        // cm.getActiveNetworkInfo() → NetworkInfo
        let net_info = match env
            .call_method(
                &cm,
                jni::jni_str!("getActiveNetworkInfo"),
                jni::jni_sig!("()Landroid/net/NetworkInfo;"),
                &[],
            )
            .and_then(|v| v.l())
        {
            Ok(o) if !o.is_null() => o,
            _ => {
                env.exception_clear();
                return Ok(ConnectivityStatus::None);
            }
        };

        // networkInfo.isConnected()
        let connected = match env
            .call_method(
                &net_info,
                jni::jni_str!("isConnected"),
                jni::jni_sig!("()Z"),
                &[],
            )
            .and_then(|v| v.z())
        {
            Ok(c) => c,
            Err(_) => {
                env.exception_clear();
                return Ok(ConnectivityStatus::None);
            }
        };
        if !connected {
            return Ok(ConnectivityStatus::None);
        }

        // networkInfo.getType()
        match env
            .call_method(
                &net_info,
                jni::jni_str!("getType"),
                jni::jni_sig!("()I"),
                &[],
            )
            .and_then(|v| v.i())
        {
            Ok(1) => Ok(ConnectivityStatus::Wifi),     // TYPE_WIFI
            Ok(0) => Ok(ConnectivityStatus::Cellular), // TYPE_MOBILE
            Ok(_) => Ok(ConnectivityStatus::Wifi),     // Ethernet etc. treated as Wifi
            Err(_) => {
                env.exception_clear();
                Ok(ConnectivityStatus::None)
            }
        }
    })
    .unwrap_or(ConnectivityStatus::None)
}
