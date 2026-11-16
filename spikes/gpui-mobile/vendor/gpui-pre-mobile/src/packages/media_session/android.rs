use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiMediaSession";

pub fn init() -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("init"),
            jni::jni_sig!("(Landroid/app/Activity;)V"),
            &[JValue::Object(&activity)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn set_metadata(title: &str, artist: &str, duration_ms: u64) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_title = env.new_string(title).e()?;
        let j_artist = env.new_string(artist).e()?;

        env.call_static_method(
            &cls,
            jni::jni_str!("setMetadata"),
            jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;J)V"),
            &[
                JValue::Object(&j_title),
                JValue::Object(&j_artist),
                JValue::Long(duration_ms as i64),
            ],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn set_playback_state(is_playing: bool, position_ms: u64, speed: f32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("setPlaybackState"),
            jni::jni_sig!("(ZJF)V"),
            &[
                JValue::Bool(is_playing),
                JValue::Long(position_ms as i64),
                JValue::Float(speed),
            ],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn release() -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(&cls, jni::jni_str!("release"), jni::jni_sig!("()V"), &[])
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(())
    })
}
