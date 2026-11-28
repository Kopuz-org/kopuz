//! The play stats a Subsonic server keeps per account, in the host's terms.

/// Stars run 1 to 5; the server's 0 means unrated.
const MAX_STARS: u32 = 5;

const SECS_PER_DAY: i64 = 86_400;

/// `userRating` as the host's rating: none when unrated or out of range.
pub fn stars(user_rating: Option<u32>) -> Option<u8> {
    user_rating
        .filter(|stars| (1..=MAX_STARS).contains(stars))
        .and_then(|stars| u8::try_from(stars).ok())
}

/// Unix seconds of an ISO-8601 date-time such as `2026-11-20T18:04:11.123Z`
/// or `2026-11-20T19:04:11+01:00`. No offset reads as UTC. None if unparsable.
pub fn unix_secs(text: &str) -> Option<i64> {
    let (date, time) = text.trim().split_once(['T', 't', ' '])?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: i64 = date.next()?.parse().ok()?;
    let day: i64 = date.next()?.parse().ok()?;
    if date.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let (clock, offset) = split_offset(time)?;
    let mut clock = clock.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next()?.parse().ok()?;
    // The fraction is dropped: whole seconds are all the host stores.
    let second: i64 = clock.next()?.split(['.', ',']).next()?.parse().ok()?;
    if clock.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * SECS_PER_DAY + hour * 3600 + minute * 60 + second - offset)
}

/// The clock part and the offset from UTC in seconds.
fn split_offset(time: &str) -> Option<(&str, i64)> {
    if let Some(clock) = time.strip_suffix(['Z', 'z']) {
        return Some((clock, 0));
    }
    let Some(at) = time.rfind(['+', '-']) else {
        return Some((time, 0));
    };
    let (clock, zone) = time.split_at(at);
    let sign = if zone.starts_with('-') { -1 } else { 1 };
    let zone = &zone[1..];
    let (hours, minutes) = match zone.split_once(':') {
        Some(parts) => parts,
        None if zone.len() == 4 => zone.split_at(2),
        None => (zone, "0"),
    };
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    Some((clock, sign * (hours * 3600 + minutes * 60)))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_utc_with_and_without_fraction() {
        assert_eq!(unix_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_secs("2026-11-20T18:04:11Z"), Some(1_795_197_851));
        assert_eq!(unix_secs("2026-11-20T18:04:11.123Z"), Some(1_795_197_851));
    }

    #[test]
    fn applies_the_offset() {
        assert_eq!(
            unix_secs("2026-11-20T19:04:11+01:00"),
            unix_secs("2026-11-20T18:04:11Z")
        );
        assert_eq!(
            unix_secs("2026-11-20T13:04:11.5-05:00"),
            unix_secs("2026-11-20T18:04:11Z")
        );
        assert_eq!(
            unix_secs("2026-11-20T19:04:11+0100"),
            unix_secs("2026-11-20T18:04:11Z")
        );
    }

    #[test]
    fn no_offset_reads_as_utc() {
        assert_eq!(
            unix_secs("2026-11-20T18:04:11"),
            unix_secs("2026-11-20T18:04:11Z")
        );
    }

    #[test]
    fn handles_leap_days_and_dates_before_1970() {
        assert_eq!(unix_secs("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        assert_eq!(unix_secs("1969-12-31T23:59:59Z"), Some(-1));
    }

    #[test]
    fn garbage_is_none() {
        for text in [
            "",
            "yesterday",
            "2026-11-20",
            "2026-13-20T00:00:00Z",
            "2026-11-20T25:00:00Z",
        ] {
            assert_eq!(unix_secs(text), None, "{text}");
        }
    }

    #[test]
    fn zero_and_out_of_range_stars_are_none() {
        assert_eq!(stars(None), None);
        assert_eq!(stars(Some(0)), None);
        assert_eq!(stars(Some(6)), None);
        assert_eq!(stars(Some(4)), Some(4));
    }
}
