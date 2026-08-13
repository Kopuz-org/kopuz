//! Track duration and embedded art from a file's leading bytes, for remote
//! libraries whose listing carries neither (WebDAV reports size and type only).
//!
//! Header-stated only: a container that does not state its length yields
//! `None` rather than an estimate, since a wrong duration breaks seeking more
//! visibly than a missing one does. Ogg states its length in the final page, so
//! it needs [`ogg_duration`] and a slice of the file's tail as well.

use std::io::Cursor;

use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, ReadOnlySource};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;

/// Opus granule positions count 48 kHz samples whatever the input rate was.
const OPUS_GRANULE_RATE: u64 = 48_000;

/// What a file's header states about the audio, as far as the head reaches.
#[derive(Debug, Default, PartialEq)]
pub struct HeadInfo {
    /// Whole seconds, absent unless the container states a length.
    pub duration_secs: Option<u64>,
    pub sample_rate: Option<u32>,
    /// Granule positions are counted at a fixed rate for Opus.
    pub is_opus: bool,
}

/// Read `head` (the first bytes of a file) for what its header states.
/// `extension` only hints the format; the bytes decide. Never panics: symphonia
/// can panic on malformed input, and a truncated head is malformed by nature.
pub fn read_head(head: &[u8], extension: Option<&str>) -> HeadInfo {
    std::panic::catch_unwind(|| read_head_inner(head, extension)).unwrap_or_default()
}

fn read_head_inner(head: &[u8], extension: Option<&str>) -> HeadInfo {
    let mut hint = Hint::new();
    if let Some(ext) = extension {
        hint.with_extension(ext);
    }

    // Unseekable with no length, so a reader that would derive a duration by
    // measuring the file reports none instead of measuring this fragment.
    let source = ReadOnlySource::new(Cursor::new(head));
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let Ok(format) = symphonia::default::get_probe().probe(
        &hint,
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) else {
        return HeadInfo::default();
    };

    let Some(track) = format.first_track(TrackType::Audio) else {
        return HeadInfo::default();
    };

    let audio = match &track.codec_params {
        Some(CodecParameters::Audio(audio)) => Some(audio),
        _ => None,
    };

    let duration_secs = track
        .duration
        .zip(track.time_base)
        .and_then(|(duration, base)| {
            let ticks = i64::try_from(duration.get()).ok()?;
            base.calc_time(Timestamp::new(ticks))
        })
        .map(|time| time.as_secs().max(0) as u64)
        .filter(|secs| *secs > 0);

    HeadInfo {
        duration_secs,
        sample_rate: audio.and_then(|audio| audio.sample_rate),
        is_opus: audio.is_some_and(|audio| audio.codec == CODEC_ID_OPUS),
    }
}

/// Cover art lifted out of a file header, with the extension its MIME type
/// implies (absent when the picture states none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedCover {
    pub bytes: Vec<u8>,
    pub extension: Option<String>,
}

/// What a file's leading bytes said about its cover art.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverProbe {
    Found(EmbeddedCover),
    /// Tags parsed, no picture in them. A longer read would say the same.
    None,
    /// Tags ran past the bytes given. A longer read may yet find art.
    Truncated,
}

/// Read the front cover out of `head`, a file's leading bytes; `extension` only
/// hints the format. Tags that trail the audio (an MP4 `moov` at the end) stay
/// `Truncated` however much is passed, so escalating callers need a ceiling.
pub fn probe_embedded_cover(head: &[u8], extension: Option<&str>) -> CoverProbe {
    std::panic::catch_unwind(|| probe_embedded_cover_inner(head, extension))
        .unwrap_or(CoverProbe::Truncated)
}

fn probe_embedded_cover_inner(head: &[u8], extension: Option<&str>) -> CoverProbe {
    // Properties are what would want the whole file; the tags read here sit at the front.
    let probe =
        || Probe::new(Cursor::new(head)).options(ParseOptions::new().read_properties(false));

    // Magic bytes lead; the extension covers a head too short to identify.
    let tagged = match probe().guess_file_type().map(Probe::read) {
        Ok(Ok(tagged)) => tagged,
        _ => {
            let Some(file_type) = extension.and_then(lofty::file::FileType::from_ext) else {
                return CoverProbe::Truncated;
            };
            match probe().set_file_type(file_type).read() {
                Ok(tagged) => tagged,
                Err(_) => return CoverProbe::Truncated,
            }
        }
    };

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let picture = match crate::metadata::extract_embedded_cover(&tagged, tag) {
        Some(picture) if !picture.data().is_empty() => picture,
        _ => return CoverProbe::None,
    };

    CoverProbe::Found(EmbeddedCover {
        bytes: picture.data().to_vec(),
        extension: picture
            .mime_type()
            .and_then(|mime| mime.ext())
            .map(str::to_string),
    })
}

/// Duration of an Ogg stream, whose length lives in the granule position of the
/// final page. `tail` is a slice of the file's last bytes and must be long
/// enough to contain a whole page header (64 KiB is ample). `None` when no page
/// header is found or the head stated no sample rate.
pub fn ogg_duration(head: &HeadInfo, tail: &[u8]) -> Option<u64> {
    let rate = if head.is_opus {
        OPUS_GRANULE_RATE
    } else {
        u64::from(head.sample_rate?)
    };
    if rate == 0 {
        return None;
    }

    let granule = last_granule_position(tail)?;
    let secs = granule / rate;
    (secs > 0).then_some(secs)
}

/// Granule position of the last complete page header in `tail`.
fn last_granule_position(tail: &[u8]) -> Option<u64> {
    // "OggS", version, header type, then the granule position.
    const GRANULE_AT: usize = 6;
    const HEADER_LEN: usize = GRANULE_AT + 8;

    tail.windows(HEADER_LEN)
        .rev()
        .find(|window| window.starts_with(b"OggS"))
        .map(|window| {
            let mut granule = [0u8; 8];
            granule.copy_from_slice(&window[GRANULE_AT..HEADER_LEN]);
            u64::from_le_bytes(granule)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An Ogg page header carrying `granule`, with a token payload after it.
    fn ogg_page(granule: u64) -> Vec<u8> {
        let mut page = b"OggS\0\0".to_vec();
        page.extend_from_slice(&granule.to_le_bytes());
        page.extend_from_slice(b"rest-of-the-page");
        page
    }

    #[test]
    fn last_granule_position_takes_the_final_page() {
        let mut stream = ogg_page(44_100);
        stream.extend(ogg_page(88_200));
        assert_eq!(last_granule_position(&stream), Some(88_200));
    }

    #[test]
    fn last_granule_position_needs_a_whole_header() {
        assert_eq!(last_granule_position(b""), None);
        assert_eq!(last_granule_position(b"OggS\0\0short"), None);
        assert_eq!(last_granule_position(b"no pages here at all"), None);
    }

    #[test]
    fn ogg_duration_divides_granule_by_the_rate() {
        let vorbis = HeadInfo {
            duration_secs: None,
            sample_rate: Some(44_100),
            is_opus: false,
        };
        assert_eq!(ogg_duration(&vorbis, &ogg_page(44_100 * 3)), Some(3));

        // Opus counts granules at 48 kHz whatever the stream rate says.
        let opus = HeadInfo {
            duration_secs: None,
            sample_rate: Some(24_000),
            is_opus: true,
        };
        assert_eq!(ogg_duration(&opus, &ogg_page(48_000 * 5)), Some(5));
    }

    #[test]
    fn ogg_duration_none_without_a_rate_or_a_page() {
        let no_rate = HeadInfo::default();
        assert_eq!(ogg_duration(&no_rate, &ogg_page(44_100)), None);

        let rated = HeadInfo {
            duration_secs: None,
            sample_rate: Some(44_100),
            is_opus: false,
        };
        assert_eq!(ogg_duration(&rated, b"not an ogg stream"), None);
        // Under a second of audio is reported as unknown, not as zero.
        assert_eq!(ogg_duration(&rated, &ogg_page(1)), None);
    }

    /// The tag blocks of a FLAC file: STREAMINFO, then a PICTURE block if
    /// `art` is given. No audio follows, which is all a ranged read gets.
    fn flac(art: Option<&[u8]>) -> Vec<u8> {
        use lofty::picture::{MimeType, Picture, PictureInformation, PictureType};

        fn push_block(file: &mut Vec<u8>, kind: u8, last: bool, body: &[u8]) {
            file.push(if last { 0x80 | kind } else { kind });
            file.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
            file.extend_from_slice(body);
        }

        let mut stream_info = vec![0u8; 34];
        stream_info[0..2].copy_from_slice(&4096u16.to_be_bytes());
        stream_info[2..4].copy_from_slice(&4096u16.to_be_bytes());

        let mut file = b"fLaC".to_vec();
        push_block(&mut file, 0, art.is_none(), &stream_info);

        if let Some(art) = art {
            let picture = Picture::unchecked(art.to_vec())
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Jpeg)
                .build();
            let block = picture.as_flac_bytes(PictureInformation::default(), false);
            push_block(&mut file, 6, true, &block);
        }

        file
    }

    #[test]
    fn probe_embedded_cover_lifts_art_out_of_a_header() {
        let art = b"\xff\xd8\xff\xe0 pretend jpeg";
        let CoverProbe::Found(cover) = probe_embedded_cover(&flac(Some(art)), Some("flac")) else {
            panic!("expected art");
        };
        assert_eq!(cover.bytes, art);
        assert_eq!(cover.extension.as_deref(), Some("jpg"));

        // The magic bytes are enough; no extension needs to be supplied.
        assert!(matches!(
            probe_embedded_cover(&flac(Some(art)), None),
            CoverProbe::Found(_)
        ));
    }

    #[test]
    fn probe_embedded_cover_separates_no_art_from_too_few_bytes() {
        assert_eq!(
            probe_embedded_cover(&flac(None), Some("flac")),
            CoverProbe::None
        );

        // Cut mid picture block, so the art may still be there past the end.
        let head = flac(Some(b"\xff\xd8\xff\xe0 art"));
        assert_eq!(
            probe_embedded_cover(&head[..20], Some("flac")),
            CoverProbe::Truncated
        );
    }

    #[test]
    fn probe_embedded_cover_ignores_junk_without_panicking() {
        assert_eq!(
            probe_embedded_cover(b"", Some("mp3")),
            CoverProbe::Truncated
        );
        assert_eq!(
            probe_embedded_cover(b"not audio at all", None),
            CoverProbe::Truncated
        );
    }

    #[test]
    fn read_head_ignores_junk_without_panicking() {
        assert_eq!(read_head(b"", None), HeadInfo::default());
        assert_eq!(
            read_head(b"not audio at all", Some("mp3")),
            HeadInfo::default()
        );
    }
}
