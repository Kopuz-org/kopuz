#[cfg(not(target_os = "android"))]
pub const COLUMNS_VAXRY: &str =
    "40px minmax(200px, 2fr) minmax(150px, 1fr) minmax(150px, 1fr) 64px 40px";
#[cfg(not(target_os = "android"))]
pub const COLUMNS_NORMAL: &str =
    "20px minmax(200px, 2fr) minmax(150px, 1fr) minmax(150px, 1fr) 64px 40px";
#[cfg(not(target_os = "android"))]
pub const COLUMNS_VAXRY_ALBUM: &str = "40px minmax(200px, 2fr) minmax(150px, 1fr) 64px 40px";
#[cfg(not(target_os = "android"))]
pub const COLUMNS_NORMAL_ALBUM: &str = "20px minmax(200px, 2fr) minmax(150px, 1fr) 64px 40px";

#[cfg(target_os = "android")]
pub const COLUMNS_VAXRY: &str = "28px minmax(0, 2fr) minmax(0, 1fr) minmax(0, 1fr) 48px 28px";
#[cfg(target_os = "android")]
pub const COLUMNS_NORMAL: &str = "18px minmax(0, 2fr) minmax(0, 1fr) minmax(0, 1fr) 48px 28px";
#[cfg(target_os = "android")]
pub const COLUMNS_VAXRY_ALBUM: &str = "28px minmax(0, 2fr) minmax(0, 1fr) 48px 28px";
#[cfg(target_os = "android")]
pub const COLUMNS_NORMAL_ALBUM: &str = "18px minmax(0, 2fr) minmax(0, 1fr) 48px 28px";

/// The audio quality column, wide enough for "24-bit / 192 kHz".
const QUALITY_COLUMN: &str = "112px";

/// Whether track lists draw the quality column; never on Android, whose rows have no room for it.
pub fn shows_quality_column() -> bool {
    use dioxus::prelude::*;
    !cfg!(target_os = "android")
        && try_consume_context::<hooks::ui_prefs::UiPrefsStore>()
            .is_some_and(|store| store.prefs().read().show_quality_column)
}

/// A track grid's columns, with the quality column before the duration when it is shown.
pub fn track_columns(columns: &str, quality: bool) -> String {
    match columns.rsplitn(3, ' ').collect::<Vec<_>>()[..] {
        [menu, duration, rest] if quality => {
            format!("{rest} {QUALITY_COLUMN} {duration} {menu}")
        }
        _ => columns.to_string(),
    }
}

/// "24-bit / 96 kHz", either half alone when only it is known, blank when neither is.
pub fn quality_label(bit_depth: u8, hz: u32) -> String {
    let rate = match (hz + 50) / 100 {
        0 => None,
        tenths if tenths % 10 == 0 => Some(format!("{} kHz", tenths / 10)),
        tenths => Some(format!("{}.{} kHz", tenths / 10, tenths % 10)),
    };
    let depth = (bit_depth > 0).then(|| format!("{bit_depth}-bit"));
    match (depth, rate) {
        (Some(depth), Some(rate)) => format!("{depth} / {rate}"),
        (Some(half), None) | (None, Some(half)) => half,
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quality_column_sits_before_the_duration() {
        let columns = "20px minmax(200px, 2fr) minmax(150px, 1fr) 64px 40px";
        assert_eq!(track_columns(columns, false), columns);
        assert_eq!(
            track_columns(columns, true),
            "20px minmax(200px, 2fr) minmax(150px, 1fr) 112px 64px 40px"
        );
    }

    #[test]
    fn a_quality_label_shows_what_is_known() {
        assert_eq!(quality_label(24, 96_000), "24-bit / 96 kHz");
        assert_eq!(quality_label(16, 44_100), "16-bit / 44.1 kHz");
        assert_eq!(quality_label(0, 44_100), "44.1 kHz");
        assert_eq!(quality_label(24, 0), "24-bit");
        assert_eq!(quality_label(0, 0), "");
    }
}
