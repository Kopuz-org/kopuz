//! ReplayGain tag extraction.
//!
//! The values ride in the container's own metadata, so they are read off the
//! probed stream rather than the library database: local files, direct-played
//! server files and downloaded copies all go through the same path, and a
//! service that strips or rewrites tags is reflected truthfully.

use config::ReplayGainInfo;
use symphonia::core::formats::FormatReader;
use symphonia::core::meta::{RawValue, StandardTag, Tag};

/// R128 gains are Q7.8 fixed point relative to -23 LUFS; ReplayGain 2.0
/// targets -18 LUFS, so the two scales differ by a constant 5 dB.
const R128_REFERENCE_OFFSET_DB: f32 = 5.0;

pub(crate) fn from_format(format: &mut dyn FormatReader) -> ReplayGainInfo {
    let mut metadata = format.metadata();
    let Some(revision) = metadata.skip_to_latest() else {
        return ReplayGainInfo::default();
    };
    from_tags(&revision.media.tags)
}

fn from_tags(tags: &[Tag]) -> ReplayGainInfo {
    let mut info = ReplayGainInfo {
        track_gain_db: gain_db(
            tags,
            |t| matches!(t, StandardTag::ReplayGainTrackGain(_)),
            &["REPLAYGAIN_TRACK_GAIN"],
        ),
        track_peak: peak(
            tags,
            |t| matches!(t, StandardTag::ReplayGainTrackPeak(_)),
            &["REPLAYGAIN_TRACK_PEAK"],
        ),
        album_gain_db: gain_db(
            tags,
            |t| matches!(t, StandardTag::ReplayGainAlbumGain(_)),
            &["REPLAYGAIN_ALBUM_GAIN"],
        ),
        album_peak: peak(
            tags,
            |t| matches!(t, StandardTag::ReplayGainAlbumPeak(_)),
            &["REPLAYGAIN_ALBUM_PEAK"],
        ),
    };

    // Opus files tagged by opusenc/rsgain carry R128 instead; symphonia has no
    // standard tag for it, so match the raw key.
    if info.track_gain_db.is_none() {
        info.track_gain_db = r128_gain_db(tags, "R128_TRACK_GAIN");
    }
    if info.album_gain_db.is_none() {
        info.album_gain_db = r128_gain_db(tags, "R128_ALBUM_GAIN");
    }

    info
}

fn gain_db(
    tags: &[Tag],
    matches_std: impl Fn(&StandardTag) -> bool,
    fallback_keys: &[&str],
) -> Option<f32> {
    find(tags, matches_std, fallback_keys)
        .and_then(tag_value)
        .as_deref()
        .and_then(parse_gain_db)
}

fn peak(
    tags: &[Tag],
    matches_std: impl Fn(&StandardTag) -> bool,
    fallback_keys: &[&str],
) -> Option<f32> {
    find(tags, matches_std, fallback_keys)
        .and_then(tag_value)
        .as_deref()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|peak| peak.is_finite() && *peak > 0.0)
}

fn r128_gain_db(tags: &[Tag], key: &str) -> Option<f32> {
    find(tags, |_| false, &[key])
        .and_then(tag_value)
        .as_deref()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .map(|q78| q78 as f32 / 256.0 + R128_REFERENCE_OFFSET_DB)
}

fn find<'a>(
    tags: &'a [Tag],
    matches_std: impl Fn(&StandardTag) -> bool,
    fallback_keys: &[&str],
) -> Option<&'a Tag> {
    tags.iter()
        .find(|tag| tag.std.as_ref().is_some_and(&matches_std))
        .or_else(|| {
            tags.iter().find(|tag| {
                fallback_keys
                    .iter()
                    .any(|key| tag.raw.key.eq_ignore_ascii_case(key))
            })
        })
}

fn tag_value(tag: &Tag) -> Option<String> {
    match &tag.raw.value {
        RawValue::String(value) => Some(value.to_string()),
        RawValue::StringList(values) => values.first().cloned(),
        RawValue::Float(value) => Some(value.to_string()),
        RawValue::SignedInt(value) => Some(value.to_string()),
        RawValue::UnsignedInt(value) => Some(value.to_string()),
        _ => None,
    }
}

/// Gains are written as `-7.06 dB`, `+3.2 dB` or bare numbers depending on the
/// scanner; some also wrap them in whitespace or quotes.
fn parse_gain_db(value: &str) -> Option<f32> {
    let value = value.trim().trim_matches('"');
    let numeric = value
        .strip_suffix("dB")
        .or_else(|| value.strip_suffix("DB"))
        .or_else(|| value.strip_suffix("db"))
        .unwrap_or(value)
        .trim();
    numeric.parse::<f32>().ok().filter(|gain| gain.is_finite())
}

#[cfg(test)]
mod tests {
    use super::{from_tags, parse_gain_db};
    use symphonia::core::meta::{RawTag, RawValue, StandardTag, Tag};

    fn std_tag(key: &str, value: &str, std: StandardTag) -> Tag {
        Tag::new_std(
            RawTag {
                key: key.to_string(),
                value: RawValue::from(value),
                sub_fields: None,
            },
            std,
        )
    }

    fn raw_tag(key: &str, value: &str) -> Tag {
        Tag::new(RawTag {
            key: key.to_string(),
            value: RawValue::from(value),
            sub_fields: None,
        })
    }

    #[test]
    fn parses_suffixed_and_bare_gains() {
        assert_eq!(parse_gain_db("-7.06 dB"), Some(-7.06));
        assert_eq!(parse_gain_db("+3.2 DB"), Some(3.2));
        assert_eq!(parse_gain_db("\"-1.5\""), Some(-1.5));
        assert_eq!(parse_gain_db("not a gain"), None);
    }

    #[test]
    fn reads_track_and_album_values() {
        let value = std::sync::Arc::new("-7.06 dB".to_string());
        let peak = std::sync::Arc::new("0.98".to_string());
        let tags = vec![
            std_tag(
                "REPLAYGAIN_TRACK_GAIN",
                "-7.06 dB",
                StandardTag::ReplayGainTrackGain(value),
            ),
            std_tag(
                "REPLAYGAIN_TRACK_PEAK",
                "0.98",
                StandardTag::ReplayGainTrackPeak(peak),
            ),
            raw_tag("REPLAYGAIN_ALBUM_GAIN", "-5.00 dB"),
        ];

        let info = from_tags(&tags);
        assert_eq!(info.track_gain_db, Some(-7.06));
        assert_eq!(info.track_peak, Some(0.98));
        assert_eq!(info.album_gain_db, Some(-5.0));
        assert_eq!(info.album_peak, None);
    }

    #[test]
    fn falls_back_to_r128_with_the_reference_offset() {
        let tags = vec![raw_tag("R128_TRACK_GAIN", "-1280")];
        let info = from_tags(&tags);
        assert_eq!(info.track_gain_db, Some(0.0));
    }
}
