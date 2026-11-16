use super::{
    CameraDescription, CameraHandle, CameraLensDirection, CapturedImage, ExposureMode, FlashMode,
    FocusMode, RecordedVideo, ResolutionPreset,
};
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::{JObject, JValue};

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiCamera";

pub fn available_cameras() -> Result<Vec<CameraDescription>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        // Returns a String[] of "id|facing|orientation" entries
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("availableCameras"),
                jni::jni_sig!("(Landroid/app/Activity;)[Ljava/lang/String;"),
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

        let arr = unsafe { jni::objects::JObjectArray::<JObject>::from_raw(env, result.as_raw()) };
        let len = arr.len(env).e()?;
        let mut cameras = Vec::with_capacity(len);

        for i in 0..len {
            let obj: JObject = arr.get_element(env, i).e()?;
            let entry = jni_helpers::get_string(env, &obj);
            // Parse "id|facing|orientation"
            let parts: Vec<&str> = entry.split('|').collect();
            if parts.len() >= 3 {
                let name = parts[0].to_string();
                let facing: i32 = parts[1].parse().unwrap_or(0);
                let orientation: i32 = parts[2].parse().unwrap_or(0);
                let lens_direction = match facing {
                    0 => CameraLensDirection::Front,
                    1 => CameraLensDirection::Back,
                    _ => CameraLensDirection::External,
                };
                cameras.push(CameraDescription {
                    name,
                    lens_direction,
                    sensor_orientation: orientation,
                });
            }
        }

        Ok(cameras)
    })
}

pub fn create_camera(
    camera: &CameraDescription,
    resolution: ResolutionPreset,
    enable_audio: bool,
) -> Result<usize, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_camera_id = env.new_string(&camera.name).e()?;
        let j_resolution = resolution as i32;

        let handle_id = env
            .call_static_method(
                &cls,
                jni::jni_str!("createCamera"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;IZ)I"),
                &[
                    JValue::Object(&activity),
                    JValue::Object(&j_camera_id),
                    JValue::Int(j_resolution),
                    JValue::Bool(enable_audio),
                ],
            )
            .and_then(|v| v.i())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if handle_id < 0 {
            return Err("Failed to create camera session".into());
        }

        Ok(handle_id as usize)
    })
}

pub fn stop_preview_session(handle: &CameraHandle) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        env.call_static_method(
            &cls,
            jni::jni_str!("stopPreview"),
            jni::jni_sig!("(I)V"),
            &[JValue::Int(handle.id as i32)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

pub fn take_picture(handle: &CameraHandle) -> Result<CapturedImage, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        // Returns "path|width|height"
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("takePicture"),
                jni::jni_sig!("(I)Ljava/lang/String;"),
                &[JValue::Int(handle.id as i32)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Err("Failed to capture photo".into());
        }

        let entry = jni_helpers::get_string(env, &result);
        let parts: Vec<&str> = entry.split('|').collect();
        if parts.len() >= 3 {
            Ok(CapturedImage {
                path: parts[0].to_string(),
                width: parts[1].parse().unwrap_or(0),
                height: parts[2].parse().unwrap_or(0),
            })
        } else {
            Ok(CapturedImage {
                path: entry,
                width: 0,
                height: 0,
            })
        }
    })
}

pub fn start_video_recording(handle: &CameraHandle) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("startVideoRecording"),
            jni::jni_sig!("(I)V"),
            &[JValue::Int(handle.id as i32)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn stop_video_recording(handle: &CameraHandle) -> Result<RecordedVideo, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("stopVideoRecording"),
                jni::jni_sig!("(I)Ljava/lang/String;"),
                &[JValue::Int(handle.id as i32)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Err("Failed to stop video recording".into());
        }

        let path = jni_helpers::get_string(env, &result);
        Ok(RecordedVideo { path })
    })
}

pub fn set_flash_mode(handle: &CameraHandle, mode: FlashMode) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let mode_int = match mode {
            FlashMode::Off => 0i32,
            FlashMode::Auto => 1i32,
            FlashMode::Always => 2i32,
            FlashMode::Torch => 3i32,
        };

        env.call_static_method(
            &cls,
            jni::jni_str!("setFlashMode"),
            jni::jni_sig!("(II)V"),
            &[JValue::Int(handle.id as i32), JValue::Int(mode_int)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn set_focus_mode(handle: &CameraHandle, mode: FocusMode) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let mode_int = match mode {
            FocusMode::Auto => 0i32,
            FocusMode::Locked => 1i32,
        };

        env.call_static_method(
            &cls,
            jni::jni_str!("setFocusMode"),
            jni::jni_sig!("(II)V"),
            &[JValue::Int(handle.id as i32), JValue::Int(mode_int)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn set_exposure_mode(handle: &CameraHandle, mode: ExposureMode) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let mode_int = match mode {
            ExposureMode::Auto => 0i32,
            ExposureMode::Locked => 1i32,
        };

        env.call_static_method(
            &cls,
            jni::jni_str!("setExposureMode"),
            jni::jni_sig!("(II)V"),
            &[JValue::Int(handle.id as i32), JValue::Int(mode_int)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn get_min_zoom(handle: &CameraHandle) -> Result<f64, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getMinZoom"),
                jni::jni_sig!("(I)F"),
                &[JValue::Int(handle.id as i32)],
            )
            .and_then(|v| v.f())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(result as f64)
    })
}

pub fn get_max_zoom(handle: &CameraHandle) -> Result<f64, String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getMaxZoom"),
                jni::jni_sig!("(I)F"),
                &[JValue::Int(handle.id as i32)],
            )
            .and_then(|v| v.f())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(result as f64)
    })
}

pub fn set_zoom(handle: &CameraHandle, zoom: f64) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("setZoom"),
            jni::jni_sig!("(IF)V"),
            &[JValue::Int(handle.id as i32), JValue::Float(zoom as f32)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn set_camera(handle: &CameraHandle, camera: &CameraDescription) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let j_camera_id = env.new_string(&camera.name).e()?;

        env.call_static_method(
            &cls,
            jni::jni_str!("setCamera"),
            jni::jni_sig!("(ILjava/lang/String;)V"),
            &[JValue::Int(handle.id as i32), JValue::Object(&j_camera_id)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn dispose(handle: CameraHandle) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("dispose"),
            jni::jni_sig!("(I)V"),
            &[JValue::Int(handle.id as i32)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}
