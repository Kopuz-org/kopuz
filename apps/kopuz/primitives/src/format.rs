use api::schema::Text;
use fluent_bundle::FluentValue;
use gpui_kit::SharedString;

/// A translated string, ready to hand to an element.
pub fn t(key: &str) -> SharedString {
    i18n::t(key).into()
}

/// A translated string with Fluent arguments; a count goes in as a number so its plural selects.
pub fn t_with(key: &str, args: &[(&str, FluentValue<'_>)]) -> SharedString {
    // GPUI does no bidi, and wraps at Fluent's isolation marks, orphaning a ")" after a value.
    i18n::t_with(key, args)
        .replace(['\u{2068}', '\u{2069}'], "")
        .into()
}

/// Daemon-supplied text: a key is translated, a literal is shown as it came.
pub fn text(value: &Text) -> SharedString {
    match value {
        Text::Key(key) => t(key),
        Text::Literal(literal) => literal.clone().into(),
    }
}

/// `m:ss`, or `h:mm:ss` past an hour.
pub fn duration(ms: u64) -> SharedString {
    let secs = ms / 1000;
    let (hours, minutes, seconds) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}").into()
    } else {
        format!("{minutes}:{seconds:02}").into()
    }
}

/// A length that may be unknown, which reads blank rather than as a placeholder clock.
pub fn known_duration(ms: Option<u64>) -> SharedString {
    ms.map(duration).unwrap_or_default()
}

/// A listing's total length, as "1 hr 12 min" reads better than a clock there.
pub fn total_duration(ms: u64) -> SharedString {
    let minutes = ms / 60_000;
    if minutes >= 60 {
        format!("{} hr {} min", minutes / 60, minutes % 60).into()
    } else {
        format!("{minutes} min").into()
    }
}

/// A size in decimal units, the way the daemon's caps are set: "500 MB", "1.5 GB", never split across lines.
pub fn bytes(bytes: u64) -> SharedString {
    const GB: u64 = 1_000_000_000;
    const MB: u64 = 1_000_000;
    if bytes < GB {
        return format!("{}\u{a0}MB", (bytes + MB / 2) / MB).into();
    }
    let tenths = (bytes + GB / 20) / (GB / 10);
    match tenths % 10 {
        0 => format!("{}\u{a0}GB", tenths / 10).into(),
        rest => format!("{}.{rest}\u{a0}GB", tenths / 10).into(),
    }
}

/// A sample rate given in Hz as "44.1 kHz" or "48 kHz", blank when unknown.
pub fn sample_rate(hz: u32) -> SharedString {
    if hz == 0 {
        return SharedString::default();
    }
    let tenths = (hz + 50) / 100;
    match tenths % 10 {
        0 => format!("{}\u{a0}kHz", tenths / 10).into(),
        rest => format!("{}.{rest}\u{a0}kHz", tenths / 10).into(),
    }
}

/// "24-bit", blank for lossy audio or when unknown.
pub fn bit_depth(bits: u8) -> SharedString {
    match bits {
        0 => SharedString::default(),
        bits => format!("{bits}-bit").into(),
    }
}

/// "320 kbps", blank when unknown.
pub fn bitrate(kbps: u16) -> SharedString {
    match kbps {
        0 => SharedString::default(),
        kbps => format!("{kbps}\u{a0}kbps").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_quality_reads_in_its_units_and_blank_when_unknown() {
        assert_eq!(sample_rate(44_100), "44.1\u{a0}kHz");
        assert_eq!(sample_rate(48_000), "48\u{a0}kHz");
        assert_eq!(sample_rate(88_200), "88.2\u{a0}kHz");
        assert_eq!(sample_rate(0), "");
        assert_eq!(bit_depth(24), "24-bit");
        assert_eq!(bit_depth(0), "");
        assert_eq!(bitrate(320), "320\u{a0}kbps");
        assert_eq!(bitrate(0), "");
    }

    #[test]
    fn a_size_reads_in_the_largest_unit_it_fills() {
        assert_eq!(bytes(0), "0\u{a0}MB");
        assert_eq!(bytes(499_600_000), "500\u{a0}MB");
        assert_eq!(bytes(5_000_000_000), "5\u{a0}GB");
        assert_eq!(bytes(1_540_000_000), "1.5\u{a0}GB");
        assert_eq!(bytes(1_000_000_000_000), "1000\u{a0}GB");
    }

    #[test]
    fn an_argument_carries_no_isolation_marks_to_wrap_at() {
        let line = t_with(
            "offline_cache_used",
            &[("used", "1".into()), ("max", "2".into())],
        );
        assert!(!line.contains(['\u{2068}', '\u{2069}']));
        assert!(line.contains('1') && line.contains('2'));
    }

    #[test]
    fn an_unknown_length_reads_blank() {
        assert_eq!(known_duration(None), "");
        assert_eq!(known_duration(Some(65_000)), "1:05");
        assert_eq!(known_duration(Some(3_725_000)), "1:02:05");
    }
}
