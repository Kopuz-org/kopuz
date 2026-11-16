//! Haptic feedback and vibration.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// The type of haptic feedback to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HapticFeedback {
    /// A light impact (subtle tap).
    Light,
    /// A medium impact.
    Medium,
    /// A heavy impact (strong tap).
    Heavy,
    /// Selection feedback (light tick for picker scrolling).
    Selection,
    /// Success notification feedback.
    Success,
    /// Warning notification feedback.
    Warning,
    /// Error notification feedback.
    Error,
}

/// Vibrate the device for the given duration in milliseconds.
pub fn vibrate(duration_ms: u32) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::vibrate(duration_ms)
    }
    #[cfg(target_os = "android")]
    {
        android::vibrate(duration_ms)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = duration_ms;
        Err("vibration is only available on iOS and Android".into())
    }
}

/// Trigger a specific haptic feedback pattern.
pub fn haptic_feedback(feedback: HapticFeedback) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::haptic_feedback(feedback)
    }
    #[cfg(target_os = "android")]
    {
        android::haptic_feedback(feedback)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = feedback;
        Err("vibration is only available on iOS and Android".into())
    }
}

/// Check whether the device supports vibration.
pub fn can_vibrate() -> bool {
    #[cfg(target_os = "ios")]
    {
        ios::can_vibrate()
    }
    #[cfg(target_os = "android")]
    {
        android::can_vibrate()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        false
    }
}
