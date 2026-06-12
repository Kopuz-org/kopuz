//! Minimal ISO-8601 / RFC-3339 date-time parsing for remote "date added"
//! timestamps (Jellyfin `DateCreated`, Subsonic `created`). Returns a Unix
//! timestamp in seconds. Kept dependency-free on purpose — the inputs come
//! from a small, well-known set of server formats, e.g.
//! `2023-01-15T12:34:56.0000000Z`, `2023-01-15T12:34:56.000Z`,
//! `2004-11-08T23:36:11Z`, or `2004-11-08T23:36:11`.

/// Parse an ISO-8601 / RFC-3339 timestamp into Unix seconds (UTC).
///
/// Only the date and time-of-day are used; fractional seconds and any
/// trailing `Z` are ignored, and a present numeric offset is applied.
/// Returns `None` if the string does not start with a parseable
/// `YYYY-MM-DDTHH:MM:SS` (or space-separated) prefix.
pub fn parse_iso8601_to_unix_secs(input: &str) -> Option<i64> {
    let s = input.trim();
    if s.len() < 19 {
        return None;
    }
    let bytes = s.as_bytes();
    // Expect `YYYY-MM-DDTHH:MM:SS` with `-`/`:` separators and `T` or ` `.
    let sep = bytes[10];
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || (sep != b'T' && sep != b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }

    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let minute: i64 = s.get(14..16)?.parse().ok()?;
    let second: i64 = s.get(17..19)?.parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let mut secs = days * 86_400 + hour * 3_600 + minute * 60 + second;

    // Apply an explicit numeric UTC offset (`+HH:MM` / `-HH:MM`) if present
    // after the seconds. A trailing `Z` (or nothing) means UTC.
    if let Some(offset_secs) = parse_trailing_offset(&s[19..]) {
        secs -= offset_secs;
    }

    Some(secs)
}

/// Days since the Unix epoch (1970-01-01) for a proleptic-Gregorian date.
/// Howard Hinnant's `days_from_civil` algorithm.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Parse a trailing timezone designator into an offset in seconds east of
/// UTC. Accepts `Z`, empty, `+HH:MM`, `-HH:MM`, `+HHMM`, `+HH`. Fractional
/// seconds before the designator are skipped.
fn parse_trailing_offset(rest: &str) -> Option<i64> {
    let rest = rest.trim();
    // Skip fractional seconds like `.0000000`.
    let rest = rest.strip_prefix('.').map_or(rest, |frac| {
        let non_digit = frac
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(frac.len());
        &frac[non_digit..]
    });
    if rest.is_empty() || rest == "Z" || rest == "z" {
        return Some(0);
    }
    let sign = match rest.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let body = &rest[1..];
    let digits: String = body.chars().filter(|c| c.is_ascii_digit()).collect();
    let (hh, mm) = match digits.len() {
        2 => (digits.parse::<i64>().ok()?, 0),
        4 => (digits[0..2].parse().ok()?, digits[2..4].parse().ok()?),
        _ => return None,
    };
    Some(sign * (hh * 3_600 + mm * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jellyfin_style_with_fractional_and_z() {
        // Arrange
        let input = "2023-01-15T12:34:56.0000000Z";
        // Act
        let secs = parse_iso8601_to_unix_secs(input);
        // Assert — 2023-01-15T12:34:56Z = 1673786096
        assert_eq!(secs, Some(1_673_786_096));
    }

    #[test]
    fn parses_subsonic_style_without_timezone() {
        let secs = parse_iso8601_to_unix_secs("2004-11-08T23:36:11");
        assert_eq!(secs, Some(1_099_956_971));
    }

    #[test]
    fn applies_positive_offset() {
        // 12:00:00+02:00 == 10:00:00Z
        let with_offset = parse_iso8601_to_unix_secs("2023-01-15T12:00:00+02:00");
        let utc = parse_iso8601_to_unix_secs("2023-01-15T10:00:00Z");
        assert_eq!(with_offset, utc);
    }

    #[test]
    fn epoch_is_zero() {
        assert_eq!(parse_iso8601_to_unix_secs("1970-01-01T00:00:00Z"), Some(0));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_iso8601_to_unix_secs("not-a-date"), None);
        assert_eq!(parse_iso8601_to_unix_secs(""), None);
        assert_eq!(parse_iso8601_to_unix_secs("2023/01/15 12:00:00"), None);
    }
}
