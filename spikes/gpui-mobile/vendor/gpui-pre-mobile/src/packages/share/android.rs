use crate::android::jni::{self as jni_helpers, JniExt};
use jni::objects::JValue;

pub fn share_text(text: &str, subject: Option<&str>) -> Result<(), String> {
    let text = text.to_owned();
    let subject = subject.map(|s| s.to_owned());
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;

        // Intent intent = new Intent(Intent.ACTION_SEND);
        let action_send = env.new_string("android.intent.action.SEND").e()?;
        let intent = env
            .new_object(
                jni::jni_str!("android/content/Intent"),
                jni::jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&action_send)],
            )
            .e()?;

        // intent.setType("text/plain")
        let mime = env.new_string("text/plain").e()?;
        let _ = env
            .call_method(
                &intent,
                jni::jni_str!("setType"),
                jni::jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
                &[JValue::Object(&mime)],
            )
            .e()?;

        // intent.putExtra(Intent.EXTRA_TEXT, text)
        let extra_text_key = env.new_string("android.intent.extra.TEXT").e()?;
        let extra_text_val = env.new_string(&text).e()?;
        let _ = env
            .call_method(
                &intent,
                jni::jni_str!("putExtra"),
                jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;"),
                &[
                    JValue::Object(&extra_text_key),
                    JValue::Object(&extra_text_val),
                ],
            )
            .e()?;

        // intent.putExtra(Intent.EXTRA_SUBJECT, subject) if provided
        if let Some(ref subj) = subject {
            let extra_subj_key = env.new_string("android.intent.extra.SUBJECT").e()?;
            let extra_subj_val = env.new_string(subj).e()?;
            let _ = env
                .call_method(
                    &intent,
                    jni::jni_str!("putExtra"),
                    jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;"),
                    &[
                        JValue::Object(&extra_subj_key),
                        JValue::Object(&extra_subj_val),
                    ],
                )
                .e()?;
        }

        // Intent chooser = Intent.createChooser(intent, "Share")
        let chooser_title = env.new_string("Share").e()?;
        let chooser_cls = env
            .find_class(jni::jni_str!("android/content/Intent"))
            .e()?;
        let chooser = env
            .call_static_method(
                &chooser_cls,
                jni::jni_str!("createChooser"),
                jni::jni_sig!(
                    "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;"
                ),
                &[JValue::Object(&intent), JValue::Object(&chooser_title)],
            )
            .and_then(|v| v.l())
            .e()?;

        // activity.startActivity(chooser)
        let result = env.call_method(
            &activity,
            jni::jni_str!("startActivity"),
            jni::jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::Object(&chooser)],
        );
        match result {
            Ok(_) => Ok(()),
            Err(_) => {
                env.exception_clear();
                Err("Failed to start share activity".into())
            }
        }
    })
}
