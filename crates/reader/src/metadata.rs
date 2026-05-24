use super::models::{Album, Library, Track};
use super::utils::{find_folder_cover, save_cover};
use lofty::file::TaggedFileExt;
use lofty::picture::{Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::ItemKey;
use lofty::{file::TaggedFile, probe::Probe, properties::FileProperties, tag::Tag};
use std::path::Path;
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey, Tag as SymphoniaTag, Value};
use symphonia::core::probe::Hint;

fn slugify_album_key(value: &str) -> String {
    value
        .to_lowercase()
        .replace(' ', "_")
        .replace(|c: char| !c.is_alphanumeric() && c != '_', "")
}

pub fn make_album_id(album: &str, grouping_key: &str) -> String {
    let normalized_album = album.trim();

    if !normalized_album.is_empty() {
        return format!("alb_{}", slugify_album_key(normalized_album));
    }

    let fallback = slugify_album_key(grouping_key);
    if fallback.is_empty() {
        "alb_unknown".to_string()
    } else {
        format!("alb_unknown_{fallback}")
    }
}

fn select_best_picture<'a>(pictures: &'a [Picture]) -> Option<&'a Picture> {
    pictures
        .iter()
        .find(|picture| picture.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())
}

pub fn extract_embedded_cover<'a>(
    tagged_file: &'a TaggedFile,
    tag: Option<&'a Tag>,
) -> Option<&'a Picture> {
    let candidate_tags = tag
        .into_iter()
        .chain(tagged_file.tags().iter())
        .collect::<Vec<_>>();

    candidate_tags
        .iter()
        .find_map(|tag| tag.get_picture_type(PictureType::CoverFront))
        .or_else(|| {
            candidate_tags
                .iter()
                .find_map(|tag| select_best_picture(tag.pictures()))
        })
}

pub fn extract_metadata(
    tag: Option<&Tag>,
    properties: &FileProperties,
    track_path: &Path,
) -> Track {
    let artist = tag
        .and_then(|t| t.artist().map(|a| a.to_string()))
        .unwrap_or_else(|| "Unknown Artist".to_string());

    let artists: Vec<String> = tag
        .map(|t| {
            let from_tag: Vec<String> = t
                .get_strings(&ItemKey::TrackArtists)
                .flat_map(|s| s.split(';').map(|a| a.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect();
            if !from_tag.is_empty() {
                from_tag
            } else if artist.contains(';') {
                artist
                    .split(';')
                    .map(|a| a.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                vec![artist.clone()]
            }
        })
        .unwrap_or_else(|| vec![artist.clone()]);

    let album_title = tag.and_then(|t| t.album().map(|a| a.to_string()));

    let album_artist = tag
        .and_then(|t| t.get_string(&ItemKey::AlbumArtist))
        .map(|s| s.to_string());

    let parent_path = track_path.parent().map(|p| p.to_string_lossy());
    let grouping_key = album_artist
        .as_deref()
        .or(parent_path.as_deref())
        .unwrap_or(&artist);

    let title = tag
        .and_then(|t| t.title().map(|t| t.to_string()))
        .or_else(|| {
            track_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "Unknown Title".to_string());

    let musicbrainz_release_id = tag
        .and_then(|t| t.get_string(&ItemKey::MusicBrainzReleaseId))
        .map(|s| s.to_string());

    let sample_rate = properties.sample_rate().unwrap_or(0);
    let file_size = std::fs::metadata(track_path)
        .ok()
        .map(|m| m.len())
        .unwrap_or(0);
    let _bitdepth = properties.bit_depth().unwrap_or(0);
    let duration_secs = properties.duration().as_secs().max(1);
    let bitrate_kbps = ((file_size * 8) / duration_secs / 1000).min(u16::MAX as u64) as u16;

    Track {
        path: track_path.to_path_buf(),
        album_id: make_album_id(album_title.as_deref().unwrap_or(""), grouping_key),
        title,
        artist,
        artists,
        album: album_title.unwrap_or_else(|| "Unknown Album".to_string()),
        khz: sample_rate,
        bitrate: bitrate_kbps,
        duration: properties.duration().as_secs()
            + u64::from(properties.duration().subsec_nanos() > 0),
        track_number: tag.and_then(|t| t.track()),
        disc_number: tag.and_then(|t| t.disk()),
        musicbrainz_release_id,
        playlist_item_id: None,
    }
}

pub fn read(track_path: &Path, cover_cache: &Path, library: &mut Library) -> Option<Track> {
    let tagged_file = match Probe::open(track_path).ok()?.read() {
        Ok(tagged_file) => tagged_file,
        Err(_) if is_matroska_audio(track_path) => {
            return read_with_symphonia(track_path, cover_cache, library);
        }
        Err(_) => return None,
    };
    let properties = tagged_file.properties();
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let track = extract_metadata(tag, properties, track_path);
    let album_id = track.album_id.clone();

    let album_artist = tag
        .and_then(|t| t.get_string(&ItemKey::AlbumArtist))
        .map(|s| s.to_string())
        .unwrap_or_else(|| track.artist.clone());

    let album = library.albums.iter().find(|a| a.id == album_id);
    let album_exists = album.is_some();
    let needs_cover = album.and_then(|album| album.cover_path.as_ref()).is_none();
    let mut cover = None;

    if needs_cover {
        if let Some(picture) = extract_embedded_cover(&tagged_file, tag) {
            let extension = picture.mime_type().and_then(|mime_type| mime_type.ext());
            cover = save_cover(&album_id, picture.data(), extension, cover_cache).ok();
        } else if let Some(folder_cover) = track_path.parent().and_then(find_folder_cover) {
            cover = Some(folder_cover);
        }
    }

    if !album_exists || cover.is_some() {
        let genre = tag
            .and_then(|t| t.genre().map(|g| g.to_string()))
            .unwrap_or_else(|| "Unknown".to_string());

        let year = tag.and_then(|t| t.year()).unwrap_or(0) as u16;

        library.add_album(Album {
            id: album_id.clone(),
            title: track.album.clone(),
            artist: album_artist,
            genre,
            year,
            cover_path: cover,
            manual_cover: false,
        });
    }

    library.add_track(track.clone());
    Some(track)
}

fn is_matroska_audio(track_path: &Path) -> bool {
    track_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mka"))
}

fn symphonia_tag_to_string(tag: &SymphoniaTag) -> Option<String> {
    match &tag.value {
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Value::UnsignedInt(value) => Some(value.to_string()),
        Value::SignedInt(value) => Some(value.to_string()),
        Value::Float(value) => Some(value.to_string()),
        Value::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn find_symphonia_tag<'a>(
    tags: &'a [SymphoniaTag],
    std_key: StandardTagKey,
    fallback_keys: &[&str],
) -> Option<&'a SymphoniaTag> {
    tags.iter()
        .find(|tag| tag.std_key == Some(std_key))
        .or_else(|| {
            tags.iter().find(|tag| {
                fallback_keys
                    .iter()
                    .any(|key| tag.key.eq_ignore_ascii_case(key))
            })
        })
}

fn read_with_symphonia(
    track_path: &Path,
    cover_cache: &Path,
    library: &mut Library,
) -> Option<Track> {
    let file = std::fs::File::open(track_path).ok()?;
    let file_size = file.metadata().ok().map(|m| m.len()).unwrap_or(0);

    let mut hint = Hint::new();
    if let Some(ext) = track_path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(ext);
    }

    let mut tags = Vec::new();
    let mut sample_rate = 0;
    let mut duration = 0;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    if let Ok(mut probed) = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
        if let Some(mut metadata) = probed.metadata.get() {
            let revision = metadata
                .skip_to_latest()
                .map(|revision| revision.clone())
                .or_else(|| metadata.current().map(|revision| revision.clone()));
            if let Some(revision) = revision {
                tags.extend(revision.tags().iter().cloned());
            }
        }

        let mut format = probed.format;
        {
            let mut metadata = format.metadata();
            let revision = metadata
                .skip_to_latest()
                .map(|revision| revision.clone())
                .or_else(|| metadata.current().map(|revision| revision.clone()));
            if let Some(revision) = revision {
                tags.extend(revision.tags().iter().cloned());
            }
        }

        if let Some(track_info) = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
            .or_else(|| format.tracks().first())
        {
            let codec_params = &track_info.codec_params;
            sample_rate = codec_params.sample_rate.unwrap_or(0);
            duration = codec_params
                .time_base
                .zip(codec_params.n_frames)
                .map(|(time_base, n_frames)| {
                    let time = time_base.calc_time(n_frames);
                    time.seconds + u64::from(time.frac > 0.0)
                })
                .unwrap_or(0);
        }
    }

    let artist = find_symphonia_tag(&tags, StandardTagKey::Artist, &["ARTIST"])
        .and_then(symphonia_tag_to_string)
        .unwrap_or_else(|| "Unknown Artist".to_string());

    let album_title = find_symphonia_tag(&tags, StandardTagKey::Album, &["ALBUM"])
        .and_then(symphonia_tag_to_string);

    let album_artist = find_symphonia_tag(&tags, StandardTagKey::AlbumArtist, &["ALBUMARTIST"])
        .and_then(symphonia_tag_to_string)
        .unwrap_or_else(|| artist.clone());

    let parent_path = track_path.parent().map(|p| p.to_string_lossy());
    let grouping_key = album_title
        .as_deref()
        .and_then(|title| (!title.trim().is_empty()).then_some(album_artist.as_str()))
        .or(parent_path.as_deref())
        .unwrap_or(&artist);

    let title = find_symphonia_tag(&tags, StandardTagKey::TrackTitle, &["TITLE"])
        .and_then(symphonia_tag_to_string)
        .or_else(|| {
            track_path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "Unknown Title".to_string());

    let bitrate_kbps = if duration > 0 {
        ((file_size * 8) / duration / 1000).min(u16::MAX as u64) as u16
    } else {
        0
    };

    let track = Track {
        path: track_path.to_path_buf(),
        album_id: make_album_id(album_title.as_deref().unwrap_or(""), grouping_key),
        title,
        artist: artist.clone(),
        artists: vec![artist.clone()],
        album: album_title.unwrap_or_else(|| "Unknown Album".to_string()),
        khz: sample_rate,
        bitrate: bitrate_kbps,
        duration,
        track_number: find_symphonia_tag(&tags, StandardTagKey::TrackNumber, &["TRACKNUMBER"])
            .and_then(symphonia_tag_to_string)
            .and_then(|value| value.parse().ok()),
        disc_number: find_symphonia_tag(&tags, StandardTagKey::DiscNumber, &["DISCNUMBER"])
            .and_then(symphonia_tag_to_string)
            .and_then(|value| value.parse().ok()),
        musicbrainz_release_id: find_symphonia_tag(
            &tags,
            StandardTagKey::MusicBrainzAlbumId,
            &["MUSICBRAINZ_ALBUMID"],
        )
        .and_then(symphonia_tag_to_string),
        playlist_item_id: None,
    };

    let album_id = track.album_id.clone();
    let album = library.albums.iter().find(|a| a.id == album_id);
    let album_exists = album.is_some();
    let needs_cover = album.and_then(|album| album.cover_path.as_ref()).is_none();
    let cover = if needs_cover {
        track_path.parent().and_then(find_folder_cover)
    } else {
        None
    };

    if !album_exists || cover.is_some() {
        let genre = find_symphonia_tag(&tags, StandardTagKey::Genre, &["GENRE"])
            .and_then(symphonia_tag_to_string)
            .unwrap_or_else(|| "Unknown".to_string());
        let year = find_symphonia_tag(&tags, StandardTagKey::Date, &["DATE", "YEAR"])
            .and_then(symphonia_tag_to_string)
            .and_then(|value| value.get(..4).and_then(|prefix| prefix.parse::<u16>().ok()))
            .unwrap_or(0);

        library.add_album(Album {
            id: album_id.clone(),
            title: track.album.clone(),
            artist: album_artist,
            genre,
            year,
            cover_path: cover,
            manual_cover: false,
        });
    }

    library.add_track(track.clone());
    let _ = cover_cache;
    Some(track)
}

#[cfg(test)]
mod tests {
    use super::{
        extract_embedded_cover, extract_metadata, is_matroska_audio, make_album_id,
        select_best_picture, slugify_album_key, symphonia_tag_to_string,
    };
    use lofty::file::{FileType, TaggedFile};
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::prelude::Accessor;
    use lofty::properties::FileProperties;
    use lofty::tag::Tag;
    use lofty::tag::{ItemKey, TagType};
    use std::path::Path;
    use std::time::Duration;
    use symphonia::core::meta::{Tag as SymphoniaTag, Value as SymphoniaValue};

    fn picture(pic_type: PictureType, data: &[u8]) -> Picture {
        Picture::new_unchecked(pic_type, Some(MimeType::Png), None, data.to_vec())
    }

    fn sample_tag() -> Tag {
        Tag::new(TagType::Id3v2)
    }

    #[test]
    fn slugify_album_key_normalizes_whitespace_and_punctuation() {
        assert_eq!(
            slugify_album_key("Alohaii - Patchwork!"),
            "alohaii__patchwork"
        );
        assert_eq!(slugify_album_key(""), "");
        assert_eq!(slugify_album_key("!@#$%^&*()"), "");
        assert_eq!(slugify_album_key("Tëst-Ïng"), "tëstïng");
        assert_eq!(slugify_album_key("123 456"), "123_456");
    }

    #[test]
    fn make_album_id_prefers_album_name_and_falls_back_to_grouping_key() {
        assert_eq!(make_album_id("Patchwork", "ignored"), "alb_patchwork");
        assert_eq!(
            make_album_id("   ", "Alohalii / Local Library"),
            "alb_unknown_alohalii__local_library"
        );
        assert_eq!(make_album_id("", "!!!"), "alb_unknown");
        assert_eq!(make_album_id("", ""), "alb_unknown");
    }

    #[test]
    fn select_best_picture_prefers_cover_front_and_falls_back_to_first() {
        let other = picture(PictureType::Other, b"other");
        let front = picture(PictureType::CoverFront, b"front");

        let pictures = [other.clone(), front.clone()];
        let selected = select_best_picture(&pictures).unwrap();
        assert_eq!(selected.pic_type(), PictureType::CoverFront);

        let selected = select_best_picture(std::slice::from_ref(&other)).unwrap();
        assert_eq!(selected.pic_type(), PictureType::Other);

        assert!(select_best_picture(&[]).is_none());
    }

    #[test]
    fn extract_embedded_cover_prefers_cover_front_across_candidate_tags() {
        let mut primary = sample_tag();
        primary.push_picture(picture(PictureType::Other, b"first"));

        let mut secondary = sample_tag();
        secondary.push_picture(picture(PictureType::CoverFront, b"front"));

        let tagged = TaggedFile::new(
            FileType::Mpeg,
            FileProperties::default(),
            vec![secondary, primary.clone()],
        );

        let selected = extract_embedded_cover(&tagged, Some(&primary)).unwrap();
        assert_eq!(selected.pic_type(), PictureType::CoverFront);
    }

    #[test]
    fn extract_metadata_falls_back_to_filename_and_unknowns_without_tag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Safe Room.flac");
        std::fs::write(&path, b"12345678").unwrap();

        let properties = FileProperties::new(
            Duration::from_millis(1500),
            None,
            None,
            Some(44_100),
            None,
            None,
            None,
        );

        let track = extract_metadata(None, &properties, &path);

        assert_eq!(track.title, "Safe Room");
        assert_eq!(track.artist, "Unknown Artist");
        assert_eq!(track.artists, vec!["Unknown Artist"]);
        assert_eq!(track.album, "Unknown Album");
        assert_eq!(track.khz, 44_100);
        assert_eq!(track.duration, 2);
        assert!(track.album_id.starts_with("alb_unknown_tmp"));
    }

    #[test]
    fn extract_metadata_uses_track_artists_and_album_artist_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.mp3");
        std::fs::write(&path, vec![0_u8; 32_000]).unwrap();

        let mut tag = sample_tag();
        tag.set_artist(String::from("A; B"));
        tag.set_title(String::from("Patchwork"));
        tag.set_album(String::from("Album Name"));
        tag.insert_text(ItemKey::TrackArtists, "A ; B ; C".to_string());
        tag.insert_text(ItemKey::AlbumArtist, "Album Artist".to_string());
        tag.insert_text(ItemKey::MusicBrainzReleaseId, "mbid-123".to_string());

        let properties = FileProperties::new(
            Duration::from_secs(2),
            None,
            None,
            Some(48_000),
            None,
            None,
            None,
        );

        let track = extract_metadata(Some(&tag), &properties, &path);

        assert_eq!(track.title, "Patchwork");
        assert_eq!(track.artist, "A; B");
        assert_eq!(track.artists, vec!["A", "B", "C"]);
        assert_eq!(track.album, "Album Name");
        assert_eq!(track.album_id, "alb_album_name");
        assert_eq!(track.musicbrainz_release_id.as_deref(), Some("mbid-123"));
        assert_eq!(track.khz, 48_000);
        assert_eq!(track.bitrate, 128);
    }

    #[test]
    fn extract_metadata_splits_semicolon_artist_when_track_artists_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("track.mp3");
        std::fs::write(&path, b"1234").unwrap();

        let mut tag = sample_tag();
        tag.set_artist(String::from("A; B"));

        let track = extract_metadata(Some(&tag), &FileProperties::default(), &path);
        assert_eq!(track.artists, vec!["A", "B"]);
    }

    #[test]
    fn is_matroska_audio_matches_mka_case_insensitively() {
        assert!(is_matroska_audio(Path::new("/music/test.mka")));
        assert!(is_matroska_audio(Path::new("/music/test.MKA")));
        assert!(!is_matroska_audio(Path::new("/music/test.mkv")));
        assert!(!is_matroska_audio(Path::new("/music/test.flac")));
        // Edge cases
        assert!(!is_matroska_audio(Path::new("/music/mka")));
        assert!(!is_matroska_audio(Path::new("")));
        assert!(!is_matroska_audio(Path::new("/music/.mka")));
    }

    #[test]
    fn test_symphonia_tag_to_string() {
        let tag = SymphoniaTag::new(
            Some(symphonia::core::meta::StandardTagKey::Artist),
            "ARTIST",
            SymphoniaValue::String(" Test Artist ".to_string()),
        );
        assert_eq!(
            symphonia_tag_to_string(&tag),
            Some("Test Artist".to_string())
        );

        let tag_empty = SymphoniaTag::new(
            Some(symphonia::core::meta::StandardTagKey::Artist),
            "ARTIST",
            SymphoniaValue::String("   ".to_string()),
        );
        assert_eq!(symphonia_tag_to_string(&tag_empty), None);

        let tag_int = SymphoniaTag::new(
            Some(symphonia::core::meta::StandardTagKey::TrackNumber),
            "TRACK",
            SymphoniaValue::UnsignedInt(10),
        );
        assert_eq!(symphonia_tag_to_string(&tag_int), Some("10".to_string()));

        let tag_float = SymphoniaTag::new(None, "BPM", SymphoniaValue::Float(120.5));
        assert_eq!(
            symphonia_tag_to_string(&tag_float),
            Some("120.5".to_string())
        );

        let tag_bool = SymphoniaTag::new(None, "COMPILATION", SymphoniaValue::Boolean(true));
        assert_eq!(symphonia_tag_to_string(&tag_bool), Some("true".to_string()));

        let tag_binary =
            SymphoniaTag::new(None, "BINARY", SymphoniaValue::Binary(vec![1, 2, 3].into()));
        assert_eq!(symphonia_tag_to_string(&tag_binary), None);
    }
}
