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

#[cfg(test)]
mod tests {
    use super::*;

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
