use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

pub fn launch_url(url: &str) -> Result<bool, String> {
    let url = url.to_owned();
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        let intent = create_view_intent(env, &url)?;

        // activity.startActivity(intent)
        let result = env.call_method(
            &activity,
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&intent)],
        );
        match result {
            Ok(_) => Ok(true),
            Err(_) => {
                env.exception_clear();
                Ok(false)
            }
        }
    })
}

pub fn can_launch_url(url: &str) -> Result<bool, String> {
    let url = url.to_owned();
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        let intent = create_view_intent(env, &url)?;

        // activity.getPackageManager()
        let pm = env
            .call_method(
                &activity,
                jni::jni_str!("getPackageManager"),
                jni::jni_sig!("()Landroid/content/pm/PackageManager;"),
                &[],
            )
            .and_then(|v| v.l())
            .e()?;
        if pm.is_null() {
            return Err("getPackageManager returned null".into());
        }

        // pm.resolveActivity(intent, 0)
        let resolved = env
            .call_method(
                &pm,
                jni::jni_str!("resolveActivity"),
                jni::jni_sig!("(Landroid/content/Intent;I)Landroid/content/pm/ResolveInfo;"),
                &[JValue::Object(&intent), JValue::Int(0)],
            )
            .and_then(|v| v.l());

        match resolved {
            Ok(r) => Ok(!r.is_null()),
            Err(_) => {
                env.exception_clear();
                Ok(false)
            }
        }
    })
}

/// Create an Intent(ACTION_VIEW, Uri.parse(url)).
fn create_view_intent<'local>(
    env: &mut jni::Env<'local>,
    url: &str,
) -> Result<jni::objects::JObject<'local>, String> {
    // Uri.parse(url)
    let jurl = env.new_string(url).e()?;
    let uri = env
        .call_static_method(
            jni::jni_str!("android/net/Uri"),
            jni::jni_str!("parse"),
            jni::jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
            &[JValue::Object(&jurl)],
        )
        .and_then(|v| v.l())
        .e()?;
    if uri.is_null() {
        return Err(format!("Uri.parse returned null for: {url}"));
    }

    // new Intent(ACTION_VIEW, uri)
    let action_view = env.new_string("android.intent.action.VIEW").e()?;
    let intent = env
        .new_object(
            jni::jni_str!("android/content/Intent"),
            jni::jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::Object(&action_view), JValue::Object(&uri)],
        )
        .e()?;

    Ok(intent)
}
