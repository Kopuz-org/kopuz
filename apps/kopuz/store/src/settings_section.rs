//! A settings category asked for from inside the settings page itself. Going to
//! its route does nothing when the route already names it, as it does once the
//! person has picked another category from the rail since, so the page also
//! watches this.

use gpui_kit::{App, Global, SharedString};

/// The category last asked for, by its slug, and how many times anything was.
#[derive(Default)]
pub struct SettingsSection {
    pub slug: SharedString,
    pub asked: u64,
}

impl Global for SettingsSection {}

/// Show the settings category `slug`, whatever the route already says.
pub fn ask_settings_section(slug: impl Into<SharedString>, cx: &mut App) {
    let asked = cx.default_global::<SettingsSection>().asked + 1;
    cx.set_global(SettingsSection {
        slug: slug.into(),
        asked,
    });
}
