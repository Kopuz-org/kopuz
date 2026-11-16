use crate::android::jni::{self as jni_helpers, get_string, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiClipboard";

pub fn set_text(text: &str) -> Result<(), String> {
    let text = text.to_owned();
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let j_text = env.new_string(&text).e()?;

        env.call_static_method(
            &cls,
            jni::jni_str!("setText"),
            jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)V"),
            &[JValue::Object(&activity), JValue::Object(&j_text)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn get_text() -> Result<Option<String>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getText"),
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

        let text = get_string(env, &result);
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    })
}

pub fn has_text() -> Result<bool, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("hasText"),
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
