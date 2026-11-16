use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiInAppReview";

pub fn is_available() -> Result<bool, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("isAvailable"),
                jni::jni_sig!("(Landroid/app/Activity;)Z"),
                &[JValue::Object(&activity)],
            )
            .and_then(|v| v.z())
            .e()?;

        Ok(result)
    })
}

pub fn request_review() -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let success = env
            .call_static_method(
                &cls,
                jni::jni_str!("requestReview"),
                jni::jni_sig!("(Landroid/app/Activity;)Z"),
                &[JValue::Object(&activity)],
            )
            .and_then(|v| v.z())
            .e()?;

        if success {
            Ok(())
        } else {
            Err("Failed to launch review flow".into())
        }
    })
}

pub fn open_store_listing(app_id: &str) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_app_id = env.new_string(app_id).e()?;
        let success = env
            .call_static_method(
                &cls,
                jni::jni_str!("openStoreListing"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)Z"),
                &[JValue::Object(&activity), JValue::Object(&j_app_id)],
            )
            .and_then(|v| v.z())
            .e()?;

        if success {
            Ok(())
        } else {
            Err("Failed to open store listing".into())
        }
    })
}
