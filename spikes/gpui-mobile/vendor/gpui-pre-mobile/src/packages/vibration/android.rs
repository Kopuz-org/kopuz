use super::HapticFeedback;
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::{JObject, JValue};

pub fn vibrate(duration_ms: u32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        let vibrator = get_vibrator_service(env, &activity)?;

        // Try VibrationEffect.createOneShot (API 26+)
        if let Ok(ve_cls) = env.find_class(jni::jni_str!("android/os/VibrationEffect")) {
            if let Ok(effect) = env
                .call_static_method(
                    &ve_cls,
                    jni::jni_str!("createOneShot"),
                    jni::jni_sig!("(JI)Landroid/os/VibrationEffect;"),
                    &[JValue::Long(duration_ms as i64), JValue::Int(-1)], // DEFAULT_AMPLITUDE = -1
                )
                .and_then(|v| v.l())
            {
                if !effect.is_null() {
                    let _ = env.call_method(
                        &vibrator,
                        jni::jni_str!("vibrate"),
                        jni::jni_sig!("(Landroid/os/VibrationEffect;)V"),
                        &[JValue::Object(&effect)],
                    );
                    env.exception_clear();
                    return Ok(());
                }
            }
            env.exception_clear();
        }

        // Fallback: vibrator.vibrate(long) for older APIs
        let _ = env.call_method(
            &vibrator,
            jni::jni_str!("vibrate"),
            jni::jni_sig!("(J)V"),
            &[JValue::Long(duration_ms as i64)],
        );
        env.exception_clear();
        Ok(())
    })
}

pub fn haptic_feedback(feedback: HapticFeedback) -> Result<(), String> {
    // Map to Android HapticFeedbackConstants
    let constant: i32 = match feedback {
        HapticFeedback::Light => 1,     // VIRTUAL_KEY
        HapticFeedback::Medium => 1,    // VIRTUAL_KEY
        HapticFeedback::Heavy => 0,     // LONG_PRESS
        HapticFeedback::Selection => 3, // KEYBOARD_TAP
        HapticFeedback::Success => 1,   // VIRTUAL_KEY
        HapticFeedback::Warning => 0,   // LONG_PRESS
        HapticFeedback::Error => 0,     // LONG_PRESS
    };

    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        // activity.getWindow().getDecorView().performHapticFeedback(constant)
        let window = env
            .call_method(
                &activity,
                jni::jni_str!("getWindow"),
                jni::jni_sig!("()Landroid/view/Window;"),
                &[],
            )
            .and_then(|v| v.l())
            .e()?;
        if window.is_null() {
            return Err("getWindow returned null".into());
        }

        let decor = env
            .call_method(
                &window,
                jni::jni_str!("getDecorView"),
                jni::jni_sig!("()Landroid/view/View;"),
                &[],
            )
            .and_then(|v| v.l())
            .e()?;
        if decor.is_null() {
            return Err("getDecorView returned null".into());
        }

        let _ = env.call_method(
            &decor,
            jni::jni_str!("performHapticFeedback"),
            jni::jni_sig!("(I)Z"),
            &[JValue::Int(constant)],
        );
        env.exception_clear();
        Ok(())
    })
}

pub fn can_vibrate() -> bool {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        let vibrator = get_vibrator_service(env, &activity)?;

        let result = env
            .call_method(
                &vibrator,
                jni::jni_str!("hasVibrator"),
                jni::jni_sig!("()Z"),
                &[],
            )
            .and_then(|v| v.z())
            .unwrap_or(false);
        Ok(result)
    })
    .unwrap_or(false)
}

fn get_vibrator_service<'local>(
    env: &mut jni::Env<'local>,
    activity: &JObject<'_>,
) -> Result<JObject<'local>, String> {
    let service_name = env.new_string("vibrator").e()?;
    let vibrator = env
        .call_method(
            activity,
            jni::jni_str!("getSystemService"),
            jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[JValue::Object(&service_name)],
        )
        .and_then(|v| v.l())
        .e()?;
    if vibrator.is_null() {
        return Err("Vibrator service not available".into());
    }
    Ok(vibrator)
}
