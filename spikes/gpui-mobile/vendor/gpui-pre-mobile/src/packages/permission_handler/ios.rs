use super::*;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send};
use std::sync::mpsc;

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

#[link(name = "CoreLocation", kind = "framework")]
extern "C" {}

#[link(name = "Photos", kind = "framework")]
extern "C" {}

#[link(name = "Contacts", kind = "framework")]
extern "C" {}

#[link(name = "EventKit", kind = "framework")]
extern "C" {}

#[link(name = "UserNotifications", kind = "framework")]
extern "C" {}

#[link(name = "CoreBluetooth", kind = "framework")]
extern "C" {}

#[link(name = "CoreMotion", kind = "framework")]
extern "C" {}

#[link(name = "Speech", kind = "framework")]
extern "C" {}

#[link(name = "AppTrackingTransparency", kind = "framework")]
extern "C" {}

// ── Helpers ─────────────────────────────────────────────────────────────────

use crate::ios::util::nsstring;

// ── Check permission ────────────────────────────────────────────────────────

pub fn check_permission(permission: Permission) -> Result<PermissionStatus, String> {
    unsafe {
        match permission {
            Permission::Camera => check_av_authorization("vide"),
            Permission::Microphone => check_av_authorization("soun"),
            Permission::Photos => check_photos_authorization(),
            Permission::LocationWhenInUse | Permission::LocationAlways => {
                check_location_authorization()
            }
            Permission::Contacts => check_contacts_authorization(),
            Permission::Calendar => check_event_authorization(0), // EKEntityTypeEvent
            Permission::Reminders => check_event_authorization(1), // EKEntityTypeReminder
            Permission::Notification => check_notification_authorization(),
            Permission::Bluetooth => check_bluetooth_authorization(),
            Permission::Speech => check_speech_authorization(),
            Permission::AppTrackingTransparency => check_tracking_authorization(),
            Permission::Sensors => Ok(PermissionStatus::Granted), // CoreMotion doesn't require explicit permission
            Permission::MediaLibrary => Ok(PermissionStatus::Granted), // Simplified
            // Android-only permissions
            Permission::Storage
            | Permission::SystemAlertWindow
            | Permission::InstallPackages
            | Permission::AccessNotificationPolicy
            | Permission::Phone
            | Permission::Sms
            | Permission::Audio => Ok(PermissionStatus::Granted),
            Permission::Videos => check_photos_authorization(), // iOS uses Photos framework for videos too
        }
    }
}

unsafe fn check_av_authorization(media_type: &str) -> Result<PermissionStatus, String> {
    let ns_media = nsstring(media_type);
    let status: i64 = msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: ns_media];
    Ok(av_status_to_permission(status))
}

fn av_status_to_permission(status: i64) -> PermissionStatus {
    match status {
        0 => PermissionStatus::Denied, // AVAuthorizationStatusNotDetermined
        1 => PermissionStatus::Restricted, // AVAuthorizationStatusRestricted
        2 => PermissionStatus::Denied, // AVAuthorizationStatusDenied
        3 => PermissionStatus::Granted, // AVAuthorizationStatusAuthorized
        _ => PermissionStatus::Denied,
    }
}

unsafe fn check_photos_authorization() -> Result<PermissionStatus, String> {
    // PHAuthorizationStatus
    let _status: i64 = msg_send![class!(PHPhotoLibrary), authorizationStatusForAccessLevel: 0i64]; // PHAccessLevelReadWrite=1, but 0 for addOnly
                                                                                                   // Fallback to the older API
    let status: i64 = msg_send![class!(PHPhotoLibrary), authorizationStatus];
    Ok(match status {
        0 => PermissionStatus::Denied, // PHAuthorizationStatusNotDetermined
        1 => PermissionStatus::Restricted, // PHAuthorizationStatusRestricted
        2 => PermissionStatus::Denied, // PHAuthorizationStatusDenied
        3 => PermissionStatus::Granted, // PHAuthorizationStatusAuthorized
        4 => PermissionStatus::Limited, // PHAuthorizationStatusLimited
        _ => PermissionStatus::Denied,
    })
}

unsafe fn check_location_authorization() -> Result<PermissionStatus, String> {
    let mgr: *mut AnyObject = msg_send![class!(CLLocationManager), alloc];
    let mgr: *mut AnyObject = msg_send![mgr, init];
    let status: i32 = msg_send![mgr, authorizationStatus];
    Ok(match status {
        0 => PermissionStatus::Denied, // kCLAuthorizationStatusNotDetermined
        1 => PermissionStatus::Restricted, // kCLAuthorizationStatusRestricted
        2 => PermissionStatus::PermanentlyDenied, // kCLAuthorizationStatusDenied
        3 => PermissionStatus::Granted, // kCLAuthorizationStatusAuthorizedAlways
        4 => PermissionStatus::Granted, // kCLAuthorizationStatusAuthorizedWhenInUse
        _ => PermissionStatus::Denied,
    })
}

unsafe fn check_contacts_authorization() -> Result<PermissionStatus, String> {
    let status: i64 = msg_send![class!(CNContactStore), authorizationStatusForEntityType: 0i64]; // CNEntityTypeContacts
    Ok(match status {
        0 => PermissionStatus::Denied, // CNAuthorizationStatusNotDetermined
        1 => PermissionStatus::Restricted, // CNAuthorizationStatusRestricted
        2 => PermissionStatus::Denied, // CNAuthorizationStatusDenied
        3 => PermissionStatus::Granted, // CNAuthorizationStatusAuthorized
        4 => PermissionStatus::Limited, // CNAuthorizationStatusLimited (iOS 18+)
        _ => PermissionStatus::Denied,
    })
}

unsafe fn check_event_authorization(entity_type: i64) -> Result<PermissionStatus, String> {
    let status: i64 =
        msg_send![class!(EKEventStore), authorizationStatusForEntityType: entity_type];
    Ok(match status {
        0 => PermissionStatus::Denied, // EKAuthorizationStatusNotDetermined
        1 => PermissionStatus::Restricted, // EKAuthorizationStatusRestricted
        2 => PermissionStatus::Denied, // EKAuthorizationStatusDenied
        3 => PermissionStatus::Granted, // EKAuthorizationStatusAuthorized
        _ => PermissionStatus::Denied,
    })
}

unsafe fn check_notification_authorization() -> Result<PermissionStatus, String> {
    // UNUserNotificationCenter is async, so we use a sync channel
    let (tx, rx) = mpsc::channel();

    let center: *mut AnyObject =
        msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];
    let block = block2::RcBlock::new(move |settings: *mut AnyObject| {
        let auth_status: i64 = unsafe { msg_send![settings, authorizationStatus] };
        let status = match auth_status {
            0 => PermissionStatus::Denied,  // UNAuthorizationStatusNotDetermined
            1 => PermissionStatus::Denied,  // UNAuthorizationStatusDenied
            2 => PermissionStatus::Granted, // UNAuthorizationStatusAuthorized
            3 => PermissionStatus::Provisional, // UNAuthorizationStatusProvisional
            _ => PermissionStatus::Denied,
        };
        let _ = tx.send(status);
    });
    let _: () = msg_send![center, getNotificationSettingsWithCompletionHandler: &*block];

    rx.recv()
        .map_err(|_| "Failed to get notification settings".to_string())
}

unsafe fn check_bluetooth_authorization() -> Result<PermissionStatus, String> {
    let status: i64 = msg_send![class!(CBManager), authorization];
    Ok(match status {
        0 => PermissionStatus::Denied, // CBManagerAuthorizationNotDetermined
        1 => PermissionStatus::Restricted, // CBManagerAuthorizationRestricted
        2 => PermissionStatus::Denied, // CBManagerAuthorizationDenied
        3 => PermissionStatus::Granted, // CBManagerAuthorizationAllowedAlways
        _ => PermissionStatus::Denied,
    })
}

unsafe fn check_speech_authorization() -> Result<PermissionStatus, String> {
    let status: i64 = msg_send![class!(SFSpeechRecognizer), authorizationStatus];
    Ok(match status {
        0 => PermissionStatus::Denied, // SFSpeechRecognizerAuthorizationStatusNotDetermined
        1 => PermissionStatus::Denied, // SFSpeechRecognizerAuthorizationStatusDenied
        2 => PermissionStatus::Restricted, // SFSpeechRecognizerAuthorizationStatusRestricted
        3 => PermissionStatus::Granted, // SFSpeechRecognizerAuthorizationStatusAuthorized
        _ => PermissionStatus::Denied,
    })
}

unsafe fn check_tracking_authorization() -> Result<PermissionStatus, String> {
    let status: u32 = msg_send![class!(ATTrackingManager), trackingAuthorizationStatus];
    Ok(match status {
        0 => PermissionStatus::Denied, // ATTrackingManagerAuthorizationStatusNotDetermined
        1 => PermissionStatus::Restricted, // ATTrackingManagerAuthorizationStatusRestricted
        2 => PermissionStatus::Denied, // ATTrackingManagerAuthorizationStatusDenied
        3 => PermissionStatus::Granted, // ATTrackingManagerAuthorizationStatusAuthorized
        _ => PermissionStatus::Denied,
    })
}

// ── Request permission ──────────────────────────────────────────────────────

pub fn request_permission(permission: Permission) -> Result<PermissionStatus, String> {
    // First check current status
    let current = check_permission(permission)?;
    if current.is_granted()
        || current == PermissionStatus::PermanentlyDenied
        || current == PermissionStatus::Restricted
    {
        return Ok(current);
    }

    unsafe {
        match permission {
            Permission::Camera => request_av_authorization("vide"),
            Permission::Microphone => request_av_authorization("soun"),
            Permission::Photos => request_photos_authorization(),
            Permission::LocationWhenInUse => request_location_when_in_use(),
            Permission::LocationAlways => request_location_always(),
            Permission::Contacts => request_contacts_authorization(),
            Permission::Calendar => request_event_authorization(0),
            Permission::Reminders => request_event_authorization(1),
            Permission::Notification => request_notification_authorization(),
            Permission::Speech => request_speech_authorization(),
            Permission::AppTrackingTransparency => request_tracking_authorization(),
            Permission::Bluetooth => {
                // Bluetooth is implicitly requested when CBCentralManager is created
                Ok(check_bluetooth_authorization()?)
            }
            Permission::Sensors => Ok(PermissionStatus::Granted),
            Permission::MediaLibrary => Ok(PermissionStatus::Granted),
            // Android-only
            Permission::Storage
            | Permission::SystemAlertWindow
            | Permission::InstallPackages
            | Permission::AccessNotificationPolicy
            | Permission::Phone
            | Permission::Sms
            | Permission::Audio => Ok(PermissionStatus::Granted),
            Permission::Videos => request_photos_authorization(),
        }
    }
}

unsafe fn request_av_authorization(media_type: &str) -> Result<PermissionStatus, String> {
    let (tx, rx) = mpsc::channel();
    let ns_media = nsstring(media_type);

    let block = block2::RcBlock::new(move |granted: Bool| {
        let status = if granted.as_bool() {
            PermissionStatus::Granted
        } else {
            PermissionStatus::PermanentlyDenied
        };
        let _ = tx.send(status);
    });

    let _: () = msg_send![class!(AVCaptureDevice),
        requestAccessForMediaType: ns_media,
        completionHandler: &*block
    ];

    rx.recv()
        .map_err(|_| "AV authorization request failed".to_string())
}

unsafe fn request_photos_authorization() -> Result<PermissionStatus, String> {
    let (tx, rx) = mpsc::channel();

    let block = block2::RcBlock::new(move |status: i64| {
        let perm = match status {
            0 => PermissionStatus::Denied,
            1 => PermissionStatus::Restricted,
            2 => PermissionStatus::Denied,
            3 => PermissionStatus::Granted,
            4 => PermissionStatus::Limited,
            _ => PermissionStatus::Denied,
        };
        let _ = tx.send(perm);
    });

    let _: () = msg_send![class!(PHPhotoLibrary),
        requestAuthorizationForAccessLevel: 1i64, // PHAccessLevelReadWrite
        handler: &*block
    ];

    rx.recv()
        .map_err(|_| "Photos authorization request failed".to_string())
}

unsafe fn request_location_when_in_use() -> Result<PermissionStatus, String> {
    let mgr: *mut AnyObject = msg_send![class!(CLLocationManager), alloc];
    let mgr: *mut AnyObject = msg_send![mgr, init];
    let _: () = msg_send![mgr, requestWhenInUseAuthorization];
    // Location authorization is async via delegate; return current status after a brief wait
    std::thread::sleep(std::time::Duration::from_millis(500));
    check_location_authorization()
}

unsafe fn request_location_always() -> Result<PermissionStatus, String> {
    let mgr: *mut AnyObject = msg_send![class!(CLLocationManager), alloc];
    let mgr: *mut AnyObject = msg_send![mgr, init];
    let _: () = msg_send![mgr, requestAlwaysAuthorization];
    std::thread::sleep(std::time::Duration::from_millis(500));
    check_location_authorization()
}

unsafe fn request_contacts_authorization() -> Result<PermissionStatus, String> {
    let (tx, rx) = mpsc::channel();
    let store: *mut AnyObject = msg_send![class!(CNContactStore), alloc];
    let store: *mut AnyObject = msg_send![store, init];

    let block = block2::RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
        let status = if granted.as_bool() {
            PermissionStatus::Granted
        } else {
            PermissionStatus::PermanentlyDenied
        };
        let _ = tx.send(status);
    });

    let _: () = msg_send![store,
        requestAccessForEntityType: 0i64, // CNEntityTypeContacts
        completionHandler: &*block
    ];

    rx.recv()
        .map_err(|_| "Contacts authorization request failed".to_string())
}

unsafe fn request_event_authorization(entity_type: i64) -> Result<PermissionStatus, String> {
    let (tx, rx) = mpsc::channel();
    let store: *mut AnyObject = msg_send![class!(EKEventStore), alloc];
    let store: *mut AnyObject = msg_send![store, init];

    let block = block2::RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
        let status = if granted.as_bool() {
            PermissionStatus::Granted
        } else {
            PermissionStatus::PermanentlyDenied
        };
        let _ = tx.send(status);
    });

    let _: () = msg_send![store,
        requestAccessToEntityType: entity_type,
        completion: &*block
    ];

    rx.recv()
        .map_err(|_| "EventKit authorization request failed".to_string())
}

unsafe fn request_notification_authorization() -> Result<PermissionStatus, String> {
    let (tx, rx) = mpsc::channel();
    let center: *mut AnyObject =
        msg_send![class!(UNUserNotificationCenter), currentNotificationCenter];

    // UNAuthorizationOptionAlert | UNAuthorizationOptionBadge | UNAuthorizationOptionSound
    let options: u64 = (1 << 0) | (1 << 1) | (1 << 2);

    let block = block2::RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
        let status = if granted.as_bool() {
            PermissionStatus::Granted
        } else {
            PermissionStatus::PermanentlyDenied
        };
        let _ = tx.send(status);
    });

    let _: () = msg_send![center,
        requestAuthorizationWithOptions: options,
        completionHandler: &*block
    ];

    rx.recv()
        .map_err(|_| "Notification authorization request failed".to_string())
}

unsafe fn request_speech_authorization() -> Result<PermissionStatus, String> {
    let (tx, rx) = mpsc::channel();

    let block = block2::RcBlock::new(move |status: i64| {
        let perm = match status {
            0 => PermissionStatus::Denied,
            1 => PermissionStatus::Denied,
            2 => PermissionStatus::Restricted,
            3 => PermissionStatus::Granted,
            _ => PermissionStatus::Denied,
        };
        let _ = tx.send(perm);
    });

    let _: () = msg_send![class!(SFSpeechRecognizer),
        requestAuthorization: &*block
    ];

    rx.recv()
        .map_err(|_| "Speech authorization request failed".to_string())
}

unsafe fn request_tracking_authorization() -> Result<PermissionStatus, String> {
    let (tx, rx) = mpsc::channel();

    let block = block2::RcBlock::new(move |status: u32| {
        let perm = match status {
            0 => PermissionStatus::Denied,
            1 => PermissionStatus::Restricted,
            2 => PermissionStatus::PermanentlyDenied,
            3 => PermissionStatus::Granted,
            _ => PermissionStatus::Denied,
        };
        let _ = tx.send(perm);
    });

    let _: () = msg_send![class!(ATTrackingManager),
        requestTrackingAuthorizationWithCompletionHandler: &*block
    ];

    rx.recv()
        .map_err(|_| "Tracking authorization request failed".to_string())
}

// ── Batch request ───────────────────────────────────────────────────────────

pub fn request_permissions(
    permissions: &[Permission],
) -> Result<Vec<(Permission, PermissionStatus)>, String> {
    let mut results = Vec::with_capacity(permissions.len());
    for &perm in permissions {
        let status = request_permission(perm)?;
        results.push((perm, status));
    }
    Ok(results)
}

// ── Service status ──────────────────────────────────────────────────────────

pub fn service_status(permission: Permission) -> Result<ServiceStatus, String> {
    unsafe {
        match permission {
            Permission::LocationWhenInUse | Permission::LocationAlways => {
                let enabled: Bool = msg_send![class!(CLLocationManager), locationServicesEnabled];
                Ok(if enabled.as_bool() {
                    ServiceStatus::Enabled
                } else {
                    ServiceStatus::Disabled
                })
            }
            Permission::Bluetooth => {
                // Would need CBCentralManager instance; simplified
                Ok(ServiceStatus::Enabled)
            }
            _ => Ok(ServiceStatus::NotApplicable),
        }
    }
}

// ── Open app settings ───────────────────────────────────────────────────────

pub fn open_app_settings() -> Result<bool, String> {
    unsafe {
        let url_string = nsstring("app-settings:");
        let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: url_string];
        if url.is_null() {
            return Err("Failed to create settings URL".into());
        }

        let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
        let can_open: Bool = msg_send![app, canOpenURL: url];
        if !can_open.as_bool() {
            return Ok(false);
        }

        let empty_dict: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![app,
            openURL: url,
            options: empty_dict,
            completionHandler: nil
        ];

        Ok(true)
    }
}
