use super::Notification;
use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiNotifications";

pub fn initialize() -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("initialize"),
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

pub fn show(notification: &Notification) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let j_title = env.new_string(&notification.title).e()?;
        let j_body = env.new_string(&notification.body).e()?;
        let j_channel_id = env.new_string(&notification.channel.id).e()?;
        let j_channel_name = env.new_string(&notification.channel.name).e()?;
        let j_channel_desc = env.new_string(&notification.channel.description).e()?;
        let j_payload = env
            .new_string(notification.payload.as_deref().unwrap_or(""))
            .e()?;

        let importance = notification.channel.importance.as_i32();

        env.call_static_method(
            &cls,
            jni::jni_str!("show"),
            jni::jni_sig!("(Landroid/app/Activity;ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;)V"),
            &[
                JValue::Object(&activity),
                JValue::Int(notification.id),
                JValue::Object(&j_title),
                JValue::Object(&j_body),
                JValue::Object(&j_channel_id),
                JValue::Object(&j_channel_name),
                JValue::Object(&j_channel_desc),
                JValue::Int(importance),
                JValue::Object(&j_payload),
            ],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn cancel(id: i32) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("cancel"),
            jni::jni_sig!("(Landroid/app/Activity;I)V"),
            &[JValue::Object(&activity), JValue::Int(id)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;

        Ok(())
    })
}

pub fn cancel_all() -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        env.call_static_method(
            &cls,
            jni::jni_str!("cancelAll"),
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
