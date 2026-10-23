//! Every setting the palette can name, and the Settings category it sits in.

/// One reachable setting: the i18n key it is titled with, and the category slug that opens it.
pub struct Entry {
    pub key: &'static str,
    pub slug: &'static str,
}

const fn entry(key: &'static str, slug: &'static str) -> Entry {
    Entry { key, slug }
}

/// The nine category panels, so one is reachable even when it holds no named row.
pub static CATEGORIES: &[Entry] = &[
    entry("settings_general", "general"),
    entry("settings_customization", "appearance"),
    entry("settings_interface", "interface"),
    entry("settings_library", "library"),
    entry("settings_connectivity", "connectivity"),
    entry("settings_offline", "offline"),
    entry("settings_metadata", "metadata"),
    entry("settings_player", "player"),
    entry("settings_tools", "tools"),
];

/// Every row the settings crates draw, in the order their panels draw them.
pub static ROWS: &[Entry] = &[
    entry("auto_check_updates", "general"),
    entry("minimize_to_tray", "general"),
    entry("back_behavior", "general"),
    entry("language", "appearance"),
    entry("appearance", "appearance"),
    entry("cover_art_background", "appearance"),
    entry("custom_background", "appearance"),
    entry("custom_font", "appearance"),
    entry("cover_art_darkening", "appearance"),
    entry("cover_art_blur", "appearance"),
    entry("lyrics_depth_blur", "appearance"),
    entry("lyrics_depth_blur_strength", "appearance"),
    entry("reduce_animations", "appearance"),
    entry("fullscreen_use_player_bar", "appearance"),
    entry("show_source_toggle", "appearance"),
    entry("show_row_images", "appearance"),
    entry("player_bar_position", "appearance"),
    entry("interface_motion", "interface"),
    entry("interface_accent", "interface"),
    entry("interface_ambient", "interface"),
    entry("interface_scale", "interface"),
    entry("interface_font", "interface"),
    entry("interface_skin", "interface"),
    entry("palette_history", "interface"),
    entry("palette_recents", "interface"),
    entry("palette_current_track_first", "interface"),
    entry("download_quality", "offline"),
    entry("prefer_local_lyrics", "metadata"),
    entry("enable_musixmatch_lyrics", "metadata"),
    entry("lyrics_offset_auto", "metadata"),
    entry("lyrics_offset", "metadata"),
    entry("crossfade", "player"),
    entry("volume_scroll_step", "player"),
    entry("channel_mode", "player"),
    entry("device_change_behavior", "player"),
    entry("sample_rate_mode", "player"),
    entry("equalizer", "player"),
    entry("enable_tracing", "tools"),
];

#[cfg(test)]
mod tests {
    use super::{CATEGORIES, ROWS};

    /// Where a settings row can be built, so a new one cannot quietly go unreachable.
    const SOURCES: &[&str] = &[
        include_str!("../../settings_appearance/src/lib.rs"),
        include_str!("../../settings_interface/src/general.rs"),
        include_str!("../../settings_interface/src/skin.rs"),
        include_str!("../../settings_playback/src/lib.rs"),
        include_str!("prefs.rs"),
    ];

    /// How far past a row call the label is looked for; further than that is the next call.
    const REACH: usize = 240;

    /// The i18n key each settings row is titled with, as its source spells it.
    fn titles(source: &str) -> Vec<&str> {
        let mut found = Vec::new();
        for marker in ["row(", "SettingRow::new("] {
            for (at, _) in source.match_indices(marker) {
                let before = &source[..at];
                // `path_row(` and friends only end in the marker; they build no row of their own.
                let joined = before
                    .chars()
                    .next_back()
                    .is_some_and(|held| held.is_alphanumeric() || held == '_');
                if joined || before.trim_end().ends_with("fn") {
                    continue;
                }
                let window = &source[at..source.len().min(at + REACH)];
                let Some(open) = window.find("t(\"") else {
                    continue;
                };
                let rest = &window[open + 3..];
                let Some(close) = rest.find('"') else {
                    continue;
                };
                found.push(&rest[..close]);
            }
        }
        found
    }

    #[test]
    fn every_settings_row_is_reachable_by_name() {
        for source in SOURCES {
            for key in titles(source) {
                assert!(
                    ROWS.iter().any(|entry| entry.key == key),
                    "{key} is a settings row the palette cannot reach"
                );
            }
        }
    }

    /// Keys reported upstream but not landed yet; every one must still be absent from `en.ftl`.
    const PENDING: &[&str] = &[];

    #[test]
    fn every_named_setting_is_translated() {
        for entry in ROWS.iter().chain(CATEGORIES) {
            if PENDING.contains(&entry.key) {
                continue;
            }
            assert_ne!(
                i18n::t(entry.key),
                entry.key,
                "{} names a key en.ftl does not have",
                entry.key
            );
        }
    }

    #[test]
    fn every_pending_key_is_still_missing() {
        for key in PENDING {
            assert_eq!(
                i18n::t(key),
                *key,
                "{key} landed in en.ftl; drop it from PENDING"
            );
        }
    }

    #[test]
    fn every_setting_lands_in_a_category_that_exists() {
        for entry in ROWS {
            assert!(
                CATEGORIES.iter().any(|held| held.slug == entry.slug),
                "{} points at the category {}, which is not one",
                entry.key,
                entry.slug
            );
        }
    }
}
