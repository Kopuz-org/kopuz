use super::{AuthResult, BiometricType};
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiLocalAuth";

pub fn is_device_supported() -> Result<bool, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("isDeviceSupported"),
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

pub fn can_authenticate() -> Result<bool, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("canAuthenticate"),
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

pub fn get_available_biometrics() -> Result<Vec<BiometricType>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getAvailableBiometrics"),
                jni::jni_sig!("(Landroid/app/Activity;)Ljava/lang/String;"),
                &[JValue::Object(&activity)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let result_str = jni_helpers::get_string(env, &result);
        let types = result_str
            .split('|')
            .filter(|s| !s.is_empty())
            .filter_map(|s| match s {
                "fingerprint" => Some(BiometricType::Fingerprint),
                "face" => Some(BiometricType::Face),
                "iris" => Some(BiometricType::Iris),
                _ => None,
            })
            .collect();

        Ok(types)
    })
}

pub fn authenticate(reason: &str) -> Result<AuthResult, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_reason = env.new_string(reason).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("authenticate"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)I"),
                &[JValue::Object(&activity), JValue::Object(&j_reason)],
            )
            .and_then(|v| v.i())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(int_to_auth_result(result))
    })
}

fn int_to_auth_result(code: i32) -> AuthResult {
    match code {
        0 => AuthResult::Success,
        1 => AuthResult::Failed,
        2 => AuthResult::ErrorNotAvailable,
        3 => AuthResult::ErrorNotEnrolled,
        4 => AuthResult::ErrorUserCancelled,
        5 => AuthResult::ErrorPasscodeNotSet,
        6 => AuthResult::ErrorLockout,
        _ => AuthResult::ErrorOther,
    }
}
