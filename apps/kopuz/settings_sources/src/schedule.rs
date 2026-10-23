//! How often the daemon syncs the library and rescans its folders on its own.

use components::format::{t, t_with};
use components::settings::{SettingRow, choice, section};
use config::{LibraryEdit, SettingsEdit};
use gpui_kit::{App, IntoElement, ParentElement as _, SharedString};
use store::ConfigModel;

const PRESETS: [u32; 6] = [0, 60, 6 * 60, 12 * 60, 24 * 60, 7 * 24 * 60];

fn label(minutes: u32) -> SharedString {
    let (key, count) = match minutes {
        0 => return t("sync_interval_never"),
        minutes if minutes % (24 * 60) == 0 => ("sync_interval_days", minutes / (24 * 60)),
        minutes if minutes % 60 == 0 => ("sync_interval_hours", minutes / 60),
        minutes => ("sync_interval_minutes", minutes),
    };
    t_with(key, &[("count", count.into())])
}

/// The presets, plus the current value when a settings file set one that is not among them.
fn options(current: u32) -> Vec<u32> {
    let mut minutes = PRESETS.to_vec();
    if !minutes.contains(&current) {
        minutes.push(current);
        minutes.sort_unstable();
    }
    minutes
}

/// The library's own schedule, under the sources it applies to.
pub fn settings(cx: &App) -> impl IntoElement {
    let current = ConfigModel::global(cx)
        .read(cx)
        .config()
        .library
        .sync_interval_minutes;
    let options = options(current)
        .into_iter()
        .map(|minutes| (minutes, label(minutes)))
        .collect();
    section(t("settings_library")).children(
        SettingRow::new(t("sync_interval"))
            .bind("sync_interval_minutes", cx)
            .setting(choice(
                "sync-interval",
                &current,
                options,
                ConfigModel::writer(|minutes| {
                    SettingsEdit::Library(LibraryEdit::SyncIntervalMinutes(minutes))
                }),
            ))
            .shown(cx),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_from_a_settings_file_is_offered_in_its_place() {
        assert_eq!(options(90), [0, 60, 90, 360, 720, 1440, 10080]);
        assert_eq!(options(60).len(), PRESETS.len());
    }
}
