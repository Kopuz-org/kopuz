use super::{CameraDevice, ImagePickerOptions, ImageSource, PickedFile};
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::{JObject, JValue};

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiImagePicker";

pub fn pick_image(options: &ImagePickerOptions) -> Result<Option<PickedFile>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let source = match options.source {
            ImageSource::Gallery => 0i32,
            ImageSource::Camera => 1i32,
        };
        let camera_facing = match options.preferred_camera {
            CameraDevice::Rear => 0i32,
            CameraDevice::Front => 1i32,
        };

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("pickImage"),
                jni::jni_sig!("(Landroid/app/Activity;II)Ljava/lang/String;"),
                &[
                    JValue::Object(&activity),
                    JValue::Int(source),
                    JValue::Int(camera_facing),
                ],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Ok(None);
        }

        let path = jni_helpers::get_string(env, &result);
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        Ok(Some(PickedFile { path, name }))
    })
}

pub fn pick_multi_image(
    _max_width: Option<f64>,
    _max_height: Option<f64>,
    _image_quality: Option<u8>,
) -> Result<Vec<PickedFile>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("pickMultiImage"),
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
        let mut files = Vec::with_capacity(len);
        for i in 0..len {
            let obj: JObject = arr.get_element(env, i).e()?;
            let path = jni_helpers::get_string(env, &obj);
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            files.push(PickedFile { path, name });
        }
        Ok(files)
    })
}

pub fn pick_video(
    source: ImageSource,
    preferred_camera: CameraDevice,
) -> Result<Option<PickedFile>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let source_int = match source {
            ImageSource::Gallery => 0i32,
            ImageSource::Camera => 1i32,
        };
        let camera_facing = match preferred_camera {
            CameraDevice::Rear => 0i32,
            CameraDevice::Front => 1i32,
        };

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("pickVideo"),
                jni::jni_sig!("(Landroid/app/Activity;II)Ljava/lang/String;"),
                &[
                    JValue::Object(&activity),
                    JValue::Int(source_int),
                    JValue::Int(camera_facing),
                ],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Ok(None);
        }

        let path = jni_helpers::get_string(env, &result);
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        Ok(Some(PickedFile { path, name }))
    })
}
