use super::{LoopMode, PlayerState};
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiAudio";

pub fn create() -> Result<u32, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let id = env
            .call_static_method(
                &cls,
                jni::jni_str!("create"),
                jni::jni_sig!("(Landroid/app/Activity;)I"),
                &[JValue::Object(&activity)],
            )
            .and_then(|v| v.i())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if id < 0 {
            return Err("Failed to create audio player".into());
        }

        Ok(id as u32)
    })
}

pub fn set_url(id: u32, url: &str) -> Result<Option<u64>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_url = env.new_string(url).e()?;

        let duration = env
            .call_static_method(
                &cls,
                jni::jni_str!("setUrl"),
                jni::jni_sig!("(Landroid/app/Activity;ILjava/lang/String;)J"),
                &[
                    JValue::Object(&activity),
                    JValue::Int(id as i32),
                    JValue::Object(&j_url),
                ],
            )
            .and_then(|v| v.j())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if duration < 0 {
            Ok(None)
        } else {
            Ok(Some(duration as u64))
        }
    })
}

pub fn play(id: u32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("play"),
            jni::jni_sig!("(I)V"),
            &[JValue::Int(id as i32)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn pause(id: u32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("pause"),
            jni::jni_sig!("(I)V"),
            &[JValue::Int(id as i32)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn stop(id: u32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("stop"),
            jni::jni_sig!("(I)V"),
            &[JValue::Int(id as i32)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn seek(id: u32, position_ms: u64) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("seek"),
            jni::jni_sig!("(IJ)V"),
            &[JValue::Int(id as i32), JValue::Long(position_ms as i64)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn set_volume(id: u32, volume: f32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("setVolume"),
            jni::jni_sig!("(IF)V"),
            &[JValue::Int(id as i32), JValue::Float(volume)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn set_speed(id: u32, speed: f32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("setSpeed"),
            jni::jni_sig!("(IF)V"),
            &[JValue::Int(id as i32), JValue::Float(speed)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn set_loop_mode(id: u32, mode: LoopMode) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let looping = match mode {
            LoopMode::Off => false,
            LoopMode::One | LoopMode::All => true,
        };

        env.call_static_method(
            &cls,
            jni::jni_str!("setLooping"),
            jni::jni_sig!("(IZ)V"),
            &[JValue::Int(id as i32), JValue::Bool(looping)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn get_position(id: u32) -> Result<u64, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let pos = env
            .call_static_method(
                &cls,
                jni::jni_str!("getPosition"),
                jni::jni_sig!("(I)J"),
                &[JValue::Int(id as i32)],
            )
            .and_then(|v| v.j())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(if pos < 0 { 0 } else { pos as u64 })
    })
}

pub fn get_duration(id: u32) -> Result<u64, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let dur = env
            .call_static_method(
                &cls,
                jni::jni_str!("getDuration"),
                jni::jni_sig!("(I)J"),
                &[JValue::Int(id as i32)],
            )
            .and_then(|v| v.j())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(if dur < 0 { 0 } else { dur as u64 })
    })
}

pub fn is_playing(id: u32) -> Result<bool, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let playing = env
            .call_static_method(
                &cls,
                jni::jni_str!("isPlaying"),
                jni::jni_sig!("(I)Z"),
                &[JValue::Int(id as i32)],
            )
            .and_then(|v| v.z())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(playing)
    })
}

pub fn get_state(id: u32) -> Result<PlayerState, String> {
    let playing = is_playing(id)?;
    let pos = get_position(id)?;
    let dur = get_duration(id)?;

    if playing {
        Ok(PlayerState::Playing)
    } else if dur > 0 && pos >= dur {
        Ok(PlayerState::Completed)
    } else if pos > 0 {
        Ok(PlayerState::Paused)
    } else {
        Ok(PlayerState::Ready)
    }
}

pub fn dispose(id: u32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("dispose"),
            jni::jni_sig!("(I)V"),
            &[JValue::Int(id as i32)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}
