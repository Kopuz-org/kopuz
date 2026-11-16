use super::WebViewHandle;
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiHelper";

pub fn evaluate_javascript(handle: &WebViewHandle, script: &str) -> Result<(), String> {
    if handle.platform_handle.is_none() {
        return Err("No active WebView".into());
    }
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let jscript = env.new_string(script).e()?;
        env.call_static_method(
            &cls,
            jni::jni_str!("evaluateJavascript"),
            jni::jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::Object(&jscript)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

pub fn go_back(handle: &WebViewHandle) -> Result<(), String> {
    if handle.platform_handle.is_none() {
        return Err("No active WebView".into());
    }
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        env.call_static_method(&cls, jni::jni_str!("goBack"), jni::jni_sig!("()V"), &[])
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;
        Ok(())
    })
}

pub fn reload(handle: &WebViewHandle) -> Result<(), String> {
    if handle.platform_handle.is_none() {
        return Err("No active WebView".into());
    }
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        env.call_static_method(&cls, jni::jni_str!("reload"), jni::jni_sig!("()V"), &[])
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;
        Ok(())
    })
}

pub fn stop_loading(handle: &WebViewHandle) -> Result<(), String> {
    if handle.platform_handle.is_none() {
        return Err("No active WebView".into());
    }
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        env.call_static_method(
            &cls,
            jni::jni_str!("stopLoading"),
            jni::jni_sig!("()V"),
            &[],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}
