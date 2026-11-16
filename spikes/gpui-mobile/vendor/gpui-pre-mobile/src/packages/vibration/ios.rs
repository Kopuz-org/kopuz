use super::HapticFeedback;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {}

pub fn vibrate(_duration_ms: u32) -> Result<(), String> {
    // On iOS, custom duration vibration isn't supported via public API.
    // Use AudioServicesPlaySystemSound(kSystemSoundID_Vibrate) for a standard vibration.
    unsafe {
        extern "C" {
            fn AudioServicesPlaySystemSound(sound_id: u32);
        }
        AudioServicesPlaySystemSound(4095); // kSystemSoundID_Vibrate
    }
    Ok(())
}

pub fn haptic_feedback(feedback: HapticFeedback) -> Result<(), String> {
    unsafe {
        match feedback {
            HapticFeedback::Light => impact_feedback(0), // UIImpactFeedbackStyleLight
            HapticFeedback::Medium => impact_feedback(1), // UIImpactFeedbackStyleMedium
            HapticFeedback::Heavy => impact_feedback(2), // UIImpactFeedbackStyleHeavy
            HapticFeedback::Selection => selection_feedback(),
            HapticFeedback::Success => notification_feedback(0), // UINotificationFeedbackTypeSuccess
            HapticFeedback::Warning => notification_feedback(1), // UINotificationFeedbackTypeWarning
            HapticFeedback::Error => notification_feedback(2),   // UINotificationFeedbackTypeError
        }
    }
}

pub fn can_vibrate() -> bool {
    // All iPhones with Taptic Engine (iPhone 7+) support haptics.
    // We assume true since we target modern iOS.
    true
}

unsafe fn impact_feedback(style: i64) -> Result<(), String> {
    let generator: *mut AnyObject = msg_send![class!(UIImpactFeedbackGenerator), alloc];
    let generator: *mut AnyObject = msg_send![generator, initWithStyle: style];
    if generator.is_null() {
        return Err("Failed to create UIImpactFeedbackGenerator".into());
    }
    let _: () = msg_send![generator, prepare];
    let _: () = msg_send![generator, impactOccurred];
    Ok(())
}

unsafe fn selection_feedback() -> Result<(), String> {
    let generator: *mut AnyObject = msg_send![class!(UISelectionFeedbackGenerator), alloc];
    let generator: *mut AnyObject = msg_send![generator, init];
    if generator.is_null() {
        return Err("Failed to create UISelectionFeedbackGenerator".into());
    }
    let _: () = msg_send![generator, prepare];
    let _: () = msg_send![generator, selectionChanged];
    Ok(())
}

unsafe fn notification_feedback(type_: i64) -> Result<(), String> {
    let generator: *mut AnyObject = msg_send![class!(UINotificationFeedbackGenerator), alloc];
    let generator: *mut AnyObject = msg_send![generator, init];
    if generator.is_null() {
        return Err("Failed to create UINotificationFeedbackGenerator".into());
    }
    let _: () = msg_send![generator, prepare];
    let _: () = msg_send![generator, notificationOccurred: type_];
    Ok(())
}
