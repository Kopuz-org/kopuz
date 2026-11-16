use super::Notification;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

/// Get the shared UNUserNotificationCenter instance.
unsafe fn notification_center() -> *mut AnyObject {
    msg_send![class!(UNUserNotificationCenter), currentNotificationCenter]
}

pub fn initialize() -> Result<(), String> {
    unsafe {
        let center = notification_center();
        if center.is_null() {
            return Err("UNUserNotificationCenter not available".into());
        }

        // Request authorization: alert | sound | badge = 0x04 | 0x01 | 0x02 = 7
        let options: u64 = 7;

        // requestAuthorizationWithOptions:completionHandler:
        // We pass a nil completion handler for simplicity — authorization is
        // granted asynchronously and the first show() call will work once the
        // user has responded to the system prompt.
        let null_block: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![
            center,
            requestAuthorizationWithOptions: options,
            completionHandler: null_block
        ];

        Ok(())
    }
}

pub fn show(notification: &Notification) -> Result<(), String> {
    unsafe {
        let center = notification_center();
        if center.is_null() {
            return Err("UNUserNotificationCenter not available".into());
        }

        // Create UNMutableNotificationContent
        let content: *mut AnyObject = msg_send![class!(UNMutableNotificationContent), alloc];
        let content: *mut AnyObject = msg_send![content, init];
        if content.is_null() {
            return Err("Failed to create UNMutableNotificationContent".into());
        }

        // Set title
        let title = nsstring(&notification.title);
        let _: () = msg_send![content, setTitle: title];

        // Set body
        let body = nsstring(&notification.body);
        let _: () = msg_send![content, setBody: body];

        // Set sound to default
        let default_sound: *mut AnyObject = msg_send![class!(UNNotificationSound), defaultSound];
        let _: () = msg_send![content, setSound: default_sound];

        // Set userInfo with payload if present
        if let Some(ref payload) = notification.payload {
            let payload_str = nsstring(payload);
            let key = nsstring("payload");
            let user_info: *mut AnyObject = msg_send![
                class!(NSDictionary),
                dictionaryWithObject: payload_str,
                forKey: key
            ];
            let _: () = msg_send![content, setUserInfo: user_info];
        }

        // Create a UNNotificationRequest with the notification ID as identifier
        let identifier = nsstring(&notification.id.to_string());
        let null_trigger: *mut AnyObject = std::ptr::null_mut();
        let request: *mut AnyObject = msg_send![
            class!(UNNotificationRequest),
            requestWithIdentifier: identifier,
            content: content,
            trigger: null_trigger
        ];

        if request.is_null() {
            return Err("Failed to create UNNotificationRequest".into());
        }

        // Add the request to the notification center (nil completion handler)
        let null_block: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![
            center,
            addNotificationRequest: request,
            withCompletionHandler: null_block
        ];

        Ok(())
    }
}

pub fn cancel(id: i32) -> Result<(), String> {
    unsafe {
        let center = notification_center();
        if center.is_null() {
            return Err("UNUserNotificationCenter not available".into());
        }

        let identifier = nsstring(&id.to_string());
        let array: *mut AnyObject = msg_send![
            class!(NSArray),
            arrayWithObject: identifier
        ];

        // Remove both pending and delivered notifications
        let _: () = msg_send![
            center,
            removePendingNotificationRequestsWithIdentifiers: array
        ];
        let _: () = msg_send![
            center,
            removeDeliveredNotificationsWithIdentifiers: array
        ];

        Ok(())
    }
}

pub fn cancel_all() -> Result<(), String> {
    unsafe {
        let center = notification_center();
        if center.is_null() {
            return Err("UNUserNotificationCenter not available".into());
        }

        let _: () = msg_send![center, removeAllPendingNotificationRequests];
        let _: () = msg_send![center, removeAllDeliveredNotifications];

        Ok(())
    }
}

use crate::ios::util::nsstring;
