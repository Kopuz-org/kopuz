use super::{AudioFormat, Recording, RecordingConfig};
use crate::android::jni::{self as jni_helpers, get_string, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiMicrophone";

pub fn is_available() -> bool {
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
    .unwrap_or(false)
}

pub fn start_recording(config: &RecordingConfig) -> Result<String, String> {
    let format = match config.format {
        AudioFormat::Aac => 0,
        AudioFormat::Wav => 1,
        AudioFormat::Amr => 2,
    };
    let sample_rate = config.sample_rate as i32;
    let channels = config.channels as i32;
    let bit_rate = config.bit_rate as i32;

    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("startRecording"),
                jni::jni_sig!("(Landroid/app/Activity;IIII)Ljava/lang/String;"),
                &[
                    JValue::Object(&activity),
                    JValue::Int(format),
                    JValue::Int(sample_rate),
                    JValue::Int(channels),
                    JValue::Int(bit_rate),
                ],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Err("Failed to start recording".into());
        }

        let path = get_string(env, &result);
        if path.is_empty() {
            Err("Failed to start recording: empty path".into())
        } else {
            Ok(path)
        }
    })
}

pub fn stop_recording() -> Result<Recording, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("stopRecording"),
                jni::jni_sig!("()Ljava/lang/String;"),
                &[],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Err("Not recording or failed to stop".into());
        }

        let result_str = get_string(env, &result);
        // Format: "path|duration_ms"
        let parts: Vec<&str> = result_str.splitn(2, '|').collect();
        if parts.len() < 2 {
            return Err("Invalid recording result".into());
        }

        Ok(Recording {
            path: parts[0].to_string(),
            duration_ms: parts[1].parse().unwrap_or(0),
        })
    })
}

pub fn is_recording() -> bool {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("isRecording"),
                jni::jni_sig!("()Z"),
                &[],
            )
            .and_then(|v| v.z())
            .e()?;
        Ok(result)
    })
    .unwrap_or(false)
}

pub fn pause_recording() -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let success = env
            .call_static_method(
                &cls,
                jni::jni_str!("pauseRecording"),
                jni::jni_sig!("()Z"),
                &[],
            )
            .and_then(|v| v.z())
            .e()?;
        if success {
            Ok(())
        } else {
            Err("Failed to pause recording".into())
        }
    })
}

pub fn resume_recording() -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let success = env
            .call_static_method(
                &cls,
                jni::jni_str!("resumeRecording"),
                jni::jni_sig!("()Z"),
                &[],
            )
            .and_then(|v| v.z())
            .e()?;
        if success {
            Ok(())
        } else {
            Err("Failed to resume recording".into())
        }
    })
}

pub fn get_amplitude() -> Result<f64, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getAmplitude"),
                jni::jni_sig!("()D"),
                &[],
            )
            .and_then(|v| v.d())
            .e()?;
        Ok(result)
    })
}
