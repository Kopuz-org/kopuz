use super::VideoInfo;
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::{JObject, JValue};

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiVideoPlayer";

/// Create a new MediaPlayer on the Java side and return its ID.
pub fn create_player() -> Result<u32, String> {
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

        if id <= 0 {
            return Err("GpuiVideoPlayer.create failed".into());
        }
        Ok(id as u32)
    })
}

pub fn set_url(id: u32, url: &str) -> Result<VideoInfo, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let jurl = env.new_string(url).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("setUrl"),
                jni::jni_sig!("(Landroid/app/Activity;ILjava/lang/String;)Ljava/lang/String;"),
                &[
                    JValue::Object(&activity),
                    JValue::Int(id as i32),
                    JValue::Object(&jurl),
                ],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        parse_video_info(env, &result)
    })
}

pub fn set_file_path(id: u32, path: &str) -> Result<VideoInfo, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let jpath = env.new_string(path).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("setFilePath"),
                jni::jni_sig!("(Landroid/app/Activity;ILjava/lang/String;)Ljava/lang/String;"),
                &[
                    JValue::Object(&activity),
                    JValue::Int(id as i32),
                    JValue::Object(&jpath),
                ],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        parse_video_info(env, &result)
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

pub fn set_looping(id: u32, looping: bool) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
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

pub fn position(id: u32) -> Result<u64, String> {
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
        Ok(pos.max(0) as u64)
    })
}

pub fn duration(id: u32) -> Result<u64, String> {
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
        Ok(dur.max(0) as u64)
    })
}

pub fn video_size(id: u32) -> Result<(u32, u32), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let w = env
            .call_static_method(
                &cls,
                jni::jni_str!("getWidth"),
                jni::jni_sig!("(I)I"),
                &[JValue::Int(id as i32)],
            )
            .and_then(|v| v.i())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        let h = env
            .call_static_method(
                &cls,
                jni::jni_str!("getHeight"),
                jni::jni_sig!("(I)I"),
                &[JValue::Int(id as i32)],
            )
            .and_then(|v| v.i())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok((w.max(0) as u32, h.max(0) as u32))
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

/// Parse the "duration|width|height" response string from the Java helper.
fn parse_video_info(env: &mut jni::Env<'_>, obj: &JObject<'_>) -> Result<VideoInfo, String> {
    if obj.is_null() {
        return Err("Failed to set video source".into());
    }
    let s = jni_helpers::get_string(env, obj);
    if s.is_empty() {
        return Err("Empty video info response".into());
    }
    let parts: Vec<&str> = s.split('|').collect();
    if parts.len() != 3 {
        return Err(format!("Invalid video info format: {s}"));
    }
    let duration_ms = parts[0].parse::<u64>().map_err(|e| e.to_string())?;
    let width = parts[1].parse::<u32>().map_err(|e| e.to_string())?;
    let height = parts[2].parse::<u32>().map_err(|e| e.to_string())?;
    Ok(VideoInfo {
        duration_ms,
        width,
        height,
    })
}
