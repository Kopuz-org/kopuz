//! Phone or tablet, read from the viewport at runtime.
//!
//! One Android build runs on both, so `cfg!(target_os = "android")` only says
//! "touch device". A tablet is a screen whose shortest side is at least 600dp
//! (Android's own `sw600dp` line) in a window that still has that width, so a
//! tablet in split screen falls back to the phone layout.

use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub struct FormFactor {
    tablet: Signal<bool>,
    landscape: Signal<bool>,
}

impl FormFactor {
    /// A tablet-sized window: a permanent sidebar instead of the drawer and tab bar.
    pub fn is_tablet(&self) -> bool {
        *self.tablet.read()
    }

    /// A tablet held sideways, wide enough for two panes side by side.
    pub fn is_wide(&self) -> bool {
        self.is_tablet() && *self.landscape.read()
    }
}

const WATCH_VIEWPORT: &str = r#"(function(){
    if (window.__kopuzFormFactor) {
        window.removeEventListener('resize', window.__kopuzFormFactor);
    }
    const send = () => {
        const shortest = Math.min(screen.width, screen.height);
        dioxus.send([
            shortest >= 600 && window.innerWidth >= 600,
            window.innerWidth > window.innerHeight,
        ]);
    };
    window.__kopuzFormFactor = send;
    window.addEventListener('resize', send);
    send();
})()"#;

/// Provides the [`FormFactor`] for the app; call once, above anything that reads it.
pub fn use_form_factor_provider() -> FormFactor {
    let mut tablet = use_signal(|| false);
    let mut landscape = use_signal(|| false);
    let form_factor = use_context_provider(|| FormFactor { tablet, landscape });
    use_future(move || async move {
        if !cfg!(target_os = "android") {
            return;
        }
        let mut viewport = dioxus::document::eval(WATCH_VIEWPORT);
        while let Ok([is_tablet, is_landscape]) = viewport.recv::<[bool; 2]>().await {
            if *tablet.peek() != is_tablet {
                tablet.set(is_tablet);
            }
            if *landscape.peek() != is_landscape {
                landscape.set(is_landscape);
            }
        }
    });
    form_factor
}

/// The app's [`FormFactor`], or a phone-sized one where nothing provides it.
pub fn use_form_factor() -> FormFactor {
    let tablet = use_signal(|| false);
    let landscape = use_signal(|| false);
    try_consume_context::<FormFactor>().unwrap_or(FormFactor { tablet, landscape })
}
