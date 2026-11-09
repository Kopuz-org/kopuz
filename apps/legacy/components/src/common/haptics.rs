//! Haptic feedback for the touch layouts, through the `KopuzHaptics` bridge
//! `MainActivity` adds to the WebView. A no-op anywhere the bridge is missing.

#[derive(Clone, Copy, Debug)]
pub enum Haptic {
    /// A press held long enough to open a menu.
    LongPress,
    /// A gesture that crossed its commit point: a swipe that skips, a pull that
    /// will dismiss or refresh.
    Tick,
    /// A toggle such as favouriting.
    Confirm,
}

impl Haptic {
    fn kind(self) -> &'static str {
        match self {
            Haptic::LongPress => "long-press",
            Haptic::Tick => "tick",
            Haptic::Confirm => "confirm",
        }
    }
}

pub fn perform(haptic: Haptic) {
    if !cfg!(target_os = "android") {
        return;
    }
    let _ = dioxus::document::eval(&format!(
        "window.KopuzHaptics && window.KopuzHaptics.perform('{}');",
        haptic.kind()
    ));
}
