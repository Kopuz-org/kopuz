use super::{Calendar, CalendarEvent};
use crate::android::jni::{self as jni_helpers, get_string, JniExt};
use jni::objects::JValue;

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiCalendar";

pub fn get_calendars() -> Result<Vec<Calendar>, String> {
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getCalendars"),
                jni::jni_sig!("(Landroid/app/Activity;)Ljava/lang/String;"),
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

        let result_str = get_string(env, &result);
        if result_str.is_empty() {
            return Ok(vec![]);
        }

        let calendars = result_str
            .lines()
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(4, '|').collect();
                if parts.len() < 4 {
                    return None;
                }
                Some(Calendar {
                    id: parts[0].to_string(),
                    name: parts[1].to_string(),
                    is_read_only: parts[2] == "1",
                    color: parts[3].parse::<u64>().unwrap_or(0) as u32,
                })
            })
            .collect();

        Ok(calendars)
    })
}

pub fn get_events(
    calendar_id: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<CalendarEvent>, String> {
    let calendar_id = calendar_id.to_owned();
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let j_calendar_id = env.new_string(&calendar_id).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("getEvents"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;JJ)Ljava/lang/String;"),
                &[
                    JValue::Object(&activity),
                    JValue::Object(&j_calendar_id),
                    JValue::Long(start_ms),
                    JValue::Long(end_ms),
                ],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Ok(vec![]);
        }

        let result_str = get_string(env, &result);
        if result_str.is_empty() {
            return Ok(vec![]);
        }

        let events = result_str
            .lines()
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(8, '|').collect();
                if parts.len() < 8 {
                    return None;
                }
                Some(CalendarEvent {
                    id: parts[0].to_string(),
                    title: parts[1].to_string(),
                    description: parts[2].to_string(),
                    location: parts[3].to_string(),
                    start_ms: parts[4].parse().unwrap_or(0),
                    end_ms: parts[5].parse().unwrap_or(0),
                    all_day: parts[6] == "1",
                    calendar_id: parts[7].to_string(),
                })
            })
            .collect();

        Ok(events)
    })
}

pub fn create_event(event: &CalendarEvent) -> Result<String, String> {
    let calendar_id = event.calendar_id.clone();
    let title = event.title.clone();
    let description = event.description.clone();
    let location = event.location.clone();
    let start_ms = event.start_ms;
    let end_ms = event.end_ms;
    let all_day = event.all_day;

    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let j_calendar_id = env.new_string(&calendar_id).e()?;
        let j_title = env.new_string(&title).e()?;
        let j_description = env.new_string(&description).e()?;
        let j_location = env.new_string(&location).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("createEvent"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;JJZ)Ljava/lang/String;"),
                &[
                    JValue::Object(&activity),
                    JValue::Object(&j_calendar_id),
                    JValue::Object(&j_title),
                    JValue::Object(&j_description),
                    JValue::Object(&j_location),
                    JValue::Long(start_ms),
                    JValue::Long(end_ms),
                    JValue::Bool(all_day),
                ],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        if result.is_null() {
            return Err("Failed to create event".into());
        }

        let event_id = get_string(env, &result);
        if event_id.is_empty() {
            Err("Failed to create event: empty ID returned".into())
        } else {
            Ok(event_id)
        }
    })
}

pub fn delete_event(event_id: &str) -> Result<bool, String> {
    let event_id = event_id.to_owned();
    jni_helpers::with_env(|env| {
        let activity = jni_helpers::activity(env)?;
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;

        let j_event_id = env.new_string(&event_id).e()?;

        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("deleteEvent"),
                jni::jni_sig!("(Landroid/app/Activity;Ljava/lang/String;)Z"),
                &[JValue::Object(&activity), JValue::Object(&j_event_id)],
            )
            .and_then(|v| v.z())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;

        Ok(result)
    })
}
