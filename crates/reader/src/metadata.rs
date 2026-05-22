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
        });
    }

    library.add_track(track.clone());
    let _ = cover_cache;
    Some(track)
}
