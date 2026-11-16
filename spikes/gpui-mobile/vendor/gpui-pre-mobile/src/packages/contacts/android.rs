use super::{Contact, EmailAddress, PhoneNumber};
use crate::android::jni::{self as jni_helpers, get_string, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiContacts";

pub fn get_contacts() -> Result<Vec<Contact>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getContacts"),
                jni::jni_sig!("(Landroid/app/Activity;)Ljava/lang/String;"),
                &[JValue::Object(&activity)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Ok(Vec::new());
        }

        let raw = get_string(env, &result);
        Ok(parse_contacts(&raw))
    })
}

pub fn search_contacts(query: &str) -> Result<Vec<Contact>, String> {
    let query = query.to_owned();
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let j_query = env.new_string(&query).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("searchContacts"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(&activity), JValue::Object(&j_query)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Ok(Vec::new());
        }

        let raw = get_string(env, &result);
        Ok(parse_contacts(&raw))
    })
}

pub fn get_contact(id: &str) -> Result<Option<Contact>, String> {
    let id = id.to_owned();
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let j_id = env.new_string(&id).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getContact"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(&activity), JValue::Object(&j_id)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Ok(None);
        }

        let raw = get_string(env, &result);
        if raw.is_empty() {
            return Ok(None);
        }

        let contacts = parse_contacts(&raw);
        Ok(contacts.into_iter().next())
    })
}

/// Parse the pipe/newline-delimited contact format from Java.
///
/// Each line: `id|displayName|givenName|familyName|phones|emails`
/// Where phones = `number:label,number:label` and emails = `address:label,address:label`.
fn parse_contacts(raw: &str) -> Vec<Contact> {
    if raw.is_empty() {
        return Vec::new();
    }

    let mut contacts = Vec::new();

    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(6, '|').collect();
        if parts.len() < 6 {
            continue;
        }

        let id = parts[0].to_owned();
        let display_name = parts[1].to_owned();
        let given_name = parts[2].to_owned();
        let family_name = parts[3].to_owned();

        let phones = if parts[4].is_empty() {
            Vec::new()
        } else {
            parts[4]
                .split(',')
                .filter_map(|entry| {
                    let mut split = entry.splitn(2, ':');
                    let number = split.next()?.to_owned();
                    let label = split.next().unwrap_or("other").to_owned();
                    Some(PhoneNumber { number, label })
                })
                .collect()
        };

        let emails = if parts[5].is_empty() {
            Vec::new()
        } else {
            parts[5]
                .split(',')
                .filter_map(|entry| {
                    let mut split = entry.splitn(2, ':');
                    let address = split.next()?.to_owned();
                    let label = split.next().unwrap_or("other").to_owned();
                    Some(EmailAddress { address, label })
                })
                .collect()
        };

        contacts.push(Contact {
            id,
            display_name,
            given_name,
            family_name,
            phones,
            emails,
        });
    }

    contacts
}
