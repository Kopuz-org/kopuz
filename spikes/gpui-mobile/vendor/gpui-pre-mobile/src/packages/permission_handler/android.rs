use super::*;
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiPermissions";

/// Android manifest permission strings for each Permission variant.
fn permission_to_android_string(permission: Permission) -> &'static str {
    match permission {
        Permission::Camera => "android.permission.CAMERA",
        Permission::Microphone => "android.permission.RECORD_AUDIO",
        Permission::LocationWhenInUse => "android.permission.ACCESS_FINE_LOCATION",
        Permission::LocationAlways => "android.permission.ACCESS_BACKGROUND_LOCATION",
        Permission::Contacts => "android.permission.READ_CONTACTS",
        Permission::Calendar => "android.permission.READ_CALENDAR",
        Permission::Reminders => "android.permission.READ_CALENDAR", // No separate Android permission
        Permission::Photos => "android.permission.READ_MEDIA_IMAGES",
        Permission::MediaLibrary => "android.permission.READ_MEDIA_AUDIO",
        Permission::Sensors => "android.permission.BODY_SENSORS",
        Permission::Bluetooth => "android.permission.BLUETOOTH_CONNECT",
        Permission::Notification => "android.permission.POST_NOTIFICATIONS",
        Permission::Storage => "android.permission.READ_EXTERNAL_STORAGE",
        Permission::Speech => "android.permission.RECORD_AUDIO",
        Permission::AppTrackingTransparency => "", // iOS-only
        Permission::SystemAlertWindow => "android.permission.SYSTEM_ALERT_WINDOW",
        Permission::InstallPackages => "android.permission.REQUEST_INSTALL_PACKAGES",
        Permission::AccessNotificationPolicy => "android.permission.ACCESS_NOTIFICATION_POLICY",
        Permission::Phone => "android.permission.READ_PHONE_STATE",
        Permission::Sms => "android.permission.READ_SMS",
        Permission::Videos => "android.permission.READ_MEDIA_VIDEO",
        Permission::Audio => "android.permission.READ_MEDIA_AUDIO",
    }
}

pub fn check_permission(permission: Permission) -> Result<PermissionStatus, String> {
    let perm_string = permission_to_android_string(permission);
    if perm_string.is_empty() {
        return Ok(PermissionStatus::Granted); // Not applicable on Android
    }

    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_perm = env.new_string(perm_string).e()?;

        let status = env
            .call_static_method(
                &cls,
                jni::jni_str!("checkPermission"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)I"),
                &[JValue::Object(&activity), JValue::Object(&j_perm)],
            )
            .and_then(|v| v.i())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(int_to_status(status))
    })
}

pub fn request_permission(permission: Permission) -> Result<PermissionStatus, String> {
    let perm_string = permission_to_android_string(permission);
    if perm_string.is_empty() {
        return Ok(PermissionStatus::Granted);
    }

    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_perm = env.new_string(perm_string).e()?;

        let status = env
            .call_static_method(
                &cls,
                jni::jni_str!("requestPermission"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)I"),
                &[JValue::Object(&activity), JValue::Object(&j_perm)],
            )
            .and_then(|v| v.i())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(int_to_status(status))
    })
}

pub fn request_permissions(
    permissions: &[Permission],
) -> Result<Vec<(Permission, PermissionStatus)>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        // Build pipe-separated permission string
        let perm_strings: Vec<&str> = permissions
            .iter()
            .map(|p| permission_to_android_string(*p))
            .collect();
        let joined = perm_strings.join("|");
        let j_perms = env.new_string(&joined).e()?;

        // Returns pipe-separated status ints
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("requestPermissions"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(&activity), JValue::Object(&j_perms)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            // Fallback: request one by one
            return Ok(permissions
                .iter()
                .map(|&p| (p, PermissionStatus::Denied))
                .collect());
        }

        let result_str = jni_helpers::get_string(env, &result);
        let statuses: Vec<i32> = result_str
            .split('|')
            .filter_map(|s| s.parse().ok())
            .collect();

        Ok(permissions
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let status = statuses.get(i).copied().unwrap_or(-1);
                (p, int_to_status(status))
            })
            .collect())
    })
}

pub fn service_status(permission: Permission) -> Result<ServiceStatus, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let service_type = match permission {
            Permission::LocationWhenInUse | Permission::LocationAlways => 0i32,
            Permission::Bluetooth => 1i32,
            _ => {
                return Ok(ServiceStatus::NotApplicable);
            }
        };

        let enabled = env
            .call_static_method(
                &cls,
                jni::jni_str!("isServiceEnabled"),
                jni::jni_sig!("(Landroid/app/Activity;I)Z"),
                &[JValue::Object(&activity), JValue::Int(service_type)],
            )
            .and_then(|v| v.z())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(if enabled {
            ServiceStatus::Enabled
        } else {
            ServiceStatus::Disabled
        })
    })
}

pub fn open_app_settings() -> Result<bool, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("openAppSettings"),
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

pub fn should_show_request_rationale(permission: Permission) -> Result<bool, String> {
    let perm_string = permission_to_android_string(permission);
    if perm_string.is_empty() {
        return Ok(false);
    }

    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_perm = env.new_string(perm_string).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("shouldShowRationale"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)Z"),
                &[JValue::Object(&activity), JValue::Object(&j_perm)],
            )
            .and_then(|v| v.z())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(result)
    })
}

fn int_to_status(status: i32) -> PermissionStatus {
    match status {
        0 => PermissionStatus::Granted,
        1 => PermissionStatus::Denied,
        2 => PermissionStatus::PermanentlyDenied,
        3 => PermissionStatus::Restricted,
        _ => PermissionStatus::Denied,
    }
}
