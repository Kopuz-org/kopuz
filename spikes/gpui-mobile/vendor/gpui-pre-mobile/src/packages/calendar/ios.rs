use super::{Calendar, CalendarEvent};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

/// EKEntityType.event = 0
const EK_ENTITY_TYPE_EVENT: u64 = 0;

/// EKSpan.thisEvent = 0
const EK_SPAN_THIS_EVENT: u64 = 0;

/// Create an EKEventStore instance.
unsafe fn new_event_store() -> *mut AnyObject {
    let store: *mut AnyObject = msg_send![class!(EKEventStore), alloc];
    let store: *mut AnyObject = msg_send![store, init];
    store
}

/// Convert a Unix timestamp in milliseconds to an NSDate.
unsafe fn nsdate_from_ms(ms: i64) -> *mut AnyObject {
    let seconds = (ms as f64) / 1000.0;
    let date: *mut AnyObject = msg_send![class!(NSDate), dateWithTimeIntervalSince1970: seconds];
    date
}

/// Convert an NSDate to a Unix timestamp in milliseconds.
unsafe fn ms_from_nsdate(date: *mut AnyObject) -> i64 {
    if date.is_null() {
        return 0;
    }
    let seconds: f64 = msg_send![date, timeIntervalSince1970];
    (seconds * 1000.0) as i64
}

/// Read a UTF-8 string from an NSString pointer. Returns empty string if null.
unsafe fn nsstring_to_string(ns: *mut AnyObject) -> String {
    if ns.is_null() {
        return String::new();
    }
    let utf8: *const std::ffi::c_char = msg_send![ns, UTF8String];
    if utf8.is_null() {
        return String::new();
    }
    let c_str = std::ffi::CStr::from_ptr(utf8);
    c_str.to_string_lossy().into_owned()
}

use crate::ios::util::nsstring;

unsafe fn nsstring_from_str(s: &str) -> *mut AnyObject {
    nsstring(s)
}

pub fn get_calendars() -> Result<Vec<Calendar>, String> {
    unsafe {
        let store = new_event_store();
        if store.is_null() {
            return Err("Failed to create EKEventStore".into());
        }

        let calendars: *mut AnyObject =
            msg_send![store, calendarsForEntityType: EK_ENTITY_TYPE_EVENT];
        if calendars.is_null() {
            let _: () = msg_send![store, release];
            return Ok(vec![]);
        }

        let count: u64 = msg_send![calendars, count];
        let mut result = Vec::with_capacity(count as usize);

        for i in 0..count {
            let cal: *mut AnyObject = msg_send![calendars, objectAtIndex: i];
            if cal.is_null() {
                continue;
            }

            let identifier: *mut AnyObject = msg_send![cal, calendarIdentifier];
            let title: *mut AnyObject = msg_send![cal, title];
            let allows_modifications: bool = msg_send![cal, allowsContentModifications];

            // Get calendar color from CGColor
            let cg_color: *mut AnyObject = msg_send![cal, CGColor];
            let color = cgcolor_to_argb(cg_color);

            result.push(Calendar {
                id: nsstring_to_string(identifier),
                name: nsstring_to_string(title),
                is_read_only: !allows_modifications,
                color,
            });
        }

        let _: () = msg_send![store, release];
        Ok(result)
    }
}

pub fn get_events(
    calendar_id: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<CalendarEvent>, String> {
    unsafe {
        let store = new_event_store();
        if store.is_null() {
            return Err("Failed to create EKEventStore".into());
        }

        let start_date = nsdate_from_ms(start_ms);
        let end_date = nsdate_from_ms(end_ms);

        // Find the calendar with matching identifier
        let all_calendars: *mut AnyObject =
            msg_send![store, calendarsForEntityType: EK_ENTITY_TYPE_EVENT];
        if all_calendars.is_null() {
            let _: () = msg_send![store, release];
            return Ok(vec![]);
        }

        let cal_count: u64 = msg_send![all_calendars, count];
        let mut target_calendar: *mut AnyObject = std::ptr::null_mut();

        for i in 0..cal_count {
            let cal: *mut AnyObject = msg_send![all_calendars, objectAtIndex: i];
            if cal.is_null() {
                continue;
            }
            let identifier: *mut AnyObject = msg_send![cal, calendarIdentifier];
            let id_str = nsstring_to_string(identifier);
            if id_str == calendar_id {
                target_calendar = cal;
                break;
            }
        }

        if target_calendar.is_null() {
            let _: () = msg_send![store, release];
            return Err(format!("Calendar not found: {}", calendar_id));
        }

        // Create an NSArray with just this calendar
        let calendar_array: *mut AnyObject =
            msg_send![class!(NSArray), arrayWithObject: target_calendar];

        let predicate: *mut AnyObject = msg_send![store,
            predicateForEventsWithStartDate: start_date,
            endDate: end_date,
            calendars: calendar_array
        ];
        if predicate.is_null() {
            let _: () = msg_send![store, release];
            return Ok(vec![]);
        }

        let events: *mut AnyObject = msg_send![store, eventsMatchingPredicate: predicate];
        if events.is_null() {
            let _: () = msg_send![store, release];
            return Ok(vec![]);
        }

        let count: u64 = msg_send![events, count];
        let mut result = Vec::with_capacity(count as usize);

        for i in 0..count {
            let event: *mut AnyObject = msg_send![events, objectAtIndex: i];
            if event.is_null() {
                continue;
            }

            let event_identifier: *mut AnyObject = msg_send![event, eventIdentifier];
            let title: *mut AnyObject = msg_send![event, title];
            let notes: *mut AnyObject = msg_send![event, notes];
            let location: *mut AnyObject = msg_send![event, location];
            let start: *mut AnyObject = msg_send![event, startDate];
            let end: *mut AnyObject = msg_send![event, endDate];
            let is_all_day: bool = msg_send![event, isAllDay];
            let cal: *mut AnyObject = msg_send![event, calendar];
            let cal_id: *mut AnyObject = msg_send![cal, calendarIdentifier];

            result.push(CalendarEvent {
                id: nsstring_to_string(event_identifier),
                title: nsstring_to_string(title),
                description: nsstring_to_string(notes),
                location: nsstring_to_string(location),
                start_ms: ms_from_nsdate(start),
                end_ms: ms_from_nsdate(end),
                all_day: is_all_day,
                calendar_id: nsstring_to_string(cal_id),
            });
        }

        let _: () = msg_send![store, release];
        Ok(result)
    }
}

pub fn create_event(event: &CalendarEvent) -> Result<String, String> {
    unsafe {
        let store = new_event_store();
        if store.is_null() {
            return Err("Failed to create EKEventStore".into());
        }

        let ek_event: *mut AnyObject = msg_send![class!(EKEvent), eventWithEventStore: store];
        if ek_event.is_null() {
            let _: () = msg_send![store, release];
            return Err("Failed to create EKEvent".into());
        }

        let ns_title = nsstring_from_str(&event.title);
        let _: () = msg_send![ek_event, setTitle: ns_title];

        if !event.description.is_empty() {
            let ns_notes = nsstring_from_str(&event.description);
            let _: () = msg_send![ek_event, setNotes: ns_notes];
        }

        if !event.location.is_empty() {
            let ns_location = nsstring_from_str(&event.location);
            let _: () = msg_send![ek_event, setLocation: ns_location];
        }

        let start_date = nsdate_from_ms(event.start_ms);
        let _: () = msg_send![ek_event, setStartDate: start_date];

        let end_date = nsdate_from_ms(event.end_ms);
        let _: () = msg_send![ek_event, setEndDate: end_date];

        let _: () = msg_send![ek_event, setAllDay: event.all_day];

        // Find and set the calendar
        let all_calendars: *mut AnyObject =
            msg_send![store, calendarsForEntityType: EK_ENTITY_TYPE_EVENT];
        if !all_calendars.is_null() {
            let cal_count: u64 = msg_send![all_calendars, count];
            for i in 0..cal_count {
                let cal: *mut AnyObject = msg_send![all_calendars, objectAtIndex: i];
                if cal.is_null() {
                    continue;
                }
                let identifier: *mut AnyObject = msg_send![cal, calendarIdentifier];
                let id_str = nsstring_to_string(identifier);
                if id_str == event.calendar_id {
                    let _: () = msg_send![ek_event, setCalendar: cal];
                    break;
                }
            }
        }

        let mut error: *mut AnyObject = std::ptr::null_mut();
        let success: bool = msg_send![store,
            saveEvent: ek_event,
            span: EK_SPAN_THIS_EVENT,
            error: &mut error
        ];

        if !success {
            if !error.is_null() {
                let desc: *mut AnyObject = msg_send![error, localizedDescription];
                let err_str = nsstring_to_string(desc);
                let _: () = msg_send![store, release];
                return Err(format!("Failed to save event: {}", err_str));
            }
            let _: () = msg_send![store, release];
            return Err("Failed to save event".into());
        }

        let event_identifier: *mut AnyObject = msg_send![ek_event, eventIdentifier];
        let id_str = nsstring_to_string(event_identifier);
        let _: () = msg_send![store, release];
        if id_str.is_empty() {
            Err("Event saved but no identifier returned".into())
        } else {
            Ok(id_str)
        }
    }
}

pub fn delete_event(event_id: &str) -> Result<bool, String> {
    unsafe {
        let store = new_event_store();
        if store.is_null() {
            return Err("Failed to create EKEventStore".into());
        }

        let ns_event_id = nsstring_from_str(event_id);
        let event: *mut AnyObject = msg_send![store, eventWithIdentifier: ns_event_id];
        if event.is_null() {
            let _: () = msg_send![store, release];
            return Ok(false);
        }

        let mut error: *mut AnyObject = std::ptr::null_mut();
        let success: bool = msg_send![store,
            removeEvent: event,
            span: EK_SPAN_THIS_EVENT,
            error: &mut error
        ];

        if !success {
            if !error.is_null() {
                let desc: *mut AnyObject = msg_send![error, localizedDescription];
                let err_str = nsstring_to_string(desc);
                let _: () = msg_send![store, release];
                return Err(format!("Failed to delete event: {}", err_str));
            }
            let _: () = msg_send![store, release];
            return Err("Failed to delete event".into());
        }

        let _: () = msg_send![store, release];
        Ok(true)
    }
}

/// Convert a CGColor to an ARGB u32 value.
unsafe fn cgcolor_to_argb(cg_color: *mut AnyObject) -> u32 {
    if cg_color.is_null() {
        return 0xFF000000; // Default to opaque black
    }

    // Get the number of components
    let num_components: usize = {
        extern "C" {
            fn CGColorGetNumberOfComponents(color: *const AnyObject) -> usize;
        }
        CGColorGetNumberOfComponents(cg_color)
    };

    // Get the component array
    let components: *const f64 = {
        extern "C" {
            fn CGColorGetComponents(color: *const AnyObject) -> *const f64;
        }
        CGColorGetComponents(cg_color)
    };

    if components.is_null() {
        return 0xFF000000;
    }

    let (r, g, b, a) = if num_components >= 4 {
        // RGBA color space
        (
            (*components.add(0) * 255.0) as u32,
            (*components.add(1) * 255.0) as u32,
            (*components.add(2) * 255.0) as u32,
            (*components.add(3) * 255.0) as u32,
        )
    } else if num_components >= 2 {
        // Grayscale + alpha
        let gray = (*components.add(0) * 255.0) as u32;
        let alpha = (*components.add(1) * 255.0) as u32;
        (gray, gray, gray, alpha)
    } else {
        return 0xFF000000;
    };

    (a << 24) | (r << 16) | (g << 8) | b
}
