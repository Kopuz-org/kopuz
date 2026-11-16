use crate::android::jni::{self as jni_helpers, get_string, JniExt};
use jni::objects::{JObject, JValue};

pub struct AndroidSharedPreferences;

impl AndroidSharedPreferences {
    pub fn new() -> Self {
        Self
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        let key = key.to_owned();
        jni_helpers::with_env(|env| {
            let prefs = get_default_prefs(env).ok_or_else(|| "Failed to get prefs".to_string())?;

            let jkey = env.new_string(&key).map_err(|e| e.to_string())?;
            let result = env
                .call_method(
                    &prefs,
                    jni::jni_str!("getString"),
                    jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"),
                    &[JValue::Object(&jkey), JValue::Object(&JObject::null())],
                )
                .and_then(|v| v.l())
                .map_err(|e| e.to_string())?;

            if result.is_null() {
                Ok(None)
            } else {
                Ok(Some(get_string(env, &result)))
            }
        })
        .ok()
        .flatten()
    }

    pub fn set_string(&self, key: &str, value: &str) -> Result<(), String> {
        with_editor(|env, editor| {
            let jkey = env.new_string(key).e()?;
            let jval = env.new_string(value).e()?;
            let _ = env.call_method(
                editor,
                jni::jni_str!("putString"),
                jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;"),
                &[JValue::Object(&jkey), JValue::Object(&jval)],
            );
            Ok(())
        })
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        let key = key.to_owned();
        jni_helpers::with_env(|env| {
            let prefs = get_default_prefs(env).ok_or_else(|| "Failed to get prefs".to_string())?;

            if !self.contains_key_jni(env, &prefs, &key) {
                return Ok(None);
            }
            let jkey = env.new_string(&key).map_err(|e| e.to_string())?;
            let val = env
                .call_method(
                    &prefs,
                    jni::jni_str!("getLong"),
                    jni::jni_sig!("(Ljava/lang/String;J)J"),
                    &[JValue::Object(&jkey), JValue::Long(0)],
                )
                .and_then(|v| v.j())
                .map_err(|e| e.to_string())?;
            Ok(Some(val))
        })
        .ok()
        .flatten()
    }

    pub fn set_int(&self, key: &str, value: i64) -> Result<(), String> {
        with_editor(|env, editor| {
            let jkey = env.new_string(key).e()?;
            let _ = env.call_method(
                editor,
                jni::jni_str!("putLong"),
                jni::jni_sig!("(Ljava/lang/String;J)Landroid/content/SharedPreferences$Editor;"),
                &[JValue::Object(&jkey), JValue::Long(value)],
            );
            Ok(())
        })
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        let key = key.to_owned();
        jni_helpers::with_env(|env| {
            let prefs = get_default_prefs(env).ok_or_else(|| "Failed to get prefs".to_string())?;

            if !self.contains_key_jni(env, &prefs, &key) {
                return Ok(None);
            }
            let jkey = env.new_string(&key).map_err(|e| e.to_string())?;
            let val = env
                .call_method(
                    &prefs,
                    jni::jni_str!("getBoolean"),
                    jni::jni_sig!("(Ljava/lang/String;Z)Z"),
                    &[JValue::Object(&jkey), JValue::Bool(false)],
                )
                .and_then(|v| v.z())
                .map_err(|e| e.to_string())?;
            Ok(Some(val))
        })
        .ok()
        .flatten()
    }

    pub fn set_bool(&self, key: &str, value: bool) -> Result<(), String> {
        with_editor(|env, editor| {
            let jkey = env.new_string(key).e()?;
            let _ = env.call_method(
                editor,
                jni::jni_str!("putBoolean"),
                jni::jni_sig!("(Ljava/lang/String;Z)Landroid/content/SharedPreferences$Editor;"),
                &[JValue::Object(&jkey), JValue::Bool(value)],
            );
            Ok(())
        })
    }

    pub fn remove(&self, key: &str) -> Result<(), String> {
        with_editor(|env, editor| {
            let jkey = env.new_string(key).e()?;
            let _ = env.call_method(
                editor,
                jni::jni_str!("remove"),
                jni::jni_sig!("(Ljava/lang/String;)Landroid/content/SharedPreferences$Editor;"),
                &[JValue::Object(&jkey)],
            );
            Ok(())
        })
    }

    pub fn clear(&self) -> Result<(), String> {
        with_editor(|env, editor| {
            let _ = env.call_method(
                editor,
                jni::jni_str!("clear"),
                jni::jni_sig!("()Landroid/content/SharedPreferences$Editor;"),
                &[],
            );
            Ok(())
        })
    }

    pub fn contains_key(&self, key: &str) -> bool {
        let key = key.to_owned();
        jni_helpers::with_env(|env| {
            let prefs = get_default_prefs(env).ok_or_else(|| "Failed to get prefs".to_string())?;
            Ok(self.contains_key_jni(env, &prefs, &key))
        })
        .unwrap_or(false)
    }

    fn contains_key_jni(&self, env: &mut jni::Env<'_>, prefs: &JObject<'_>, key: &str) -> bool {
        let jkey = match env.new_string(key) {
            Ok(k) => k,
            Err(_) => return false,
        };
        env.call_method(
            prefs,
            jni::jni_str!("contains"),
            jni::jni_sig!("(Ljava/lang/String;)Z"),
            &[JValue::Object(&jkey)],
        )
        .and_then(|v| v.z())
        .unwrap_or(false)
    }
}

/// Get default SharedPreferences via PreferenceManager.
fn get_default_prefs<'local>(env: &mut jni::Env<'local>) -> Option<JObject<'local>> {
    let activity = jni_helpers::activity(env).ok()?;
    let prefs = env
        .call_static_method(
            jni::jni_str!("android/preference/PreferenceManager"),
            jni::jni_str!("getDefaultSharedPreferences"),
            jni::jni_sig!("(Landroid/content/Context;)Landroid/content/SharedPreferences;"),
            &[JValue::Object(&activity)],
        )
        .and_then(|v| v.l())
        .ok()?;
    if prefs.is_null() {
        None
    } else {
        Some(prefs)
    }
}

/// Get an editor, run the callback, then commit.
fn with_editor(
    f: impl FnOnce(&mut jni::Env<'_>, &JObject<'_>) -> Result<(), String>,
) -> Result<(), String> {
    jni_helpers::with_env(|env| {
        let prefs =
            get_default_prefs(env).ok_or_else(|| "Failed to get SharedPreferences".to_string())?;

        let editor = env
            .call_method(
                &prefs,
                jni::jni_str!("edit"),
                jni::jni_sig!("()Landroid/content/SharedPreferences$Editor;"),
                &[],
            )
            .and_then(|v| v.l())
            .e()?;
        if editor.is_null() {
            return Err("edit() returned null".into());
        }

        f(env, &editor)?;

        // Commit
        let _ = env.call_method(&editor, jni::jni_str!("commit"), jni::jni_sig!("()Z"), &[]);
        Ok(())
    })
}
