use super::models::{Album, CoverChange, Library, Track, TrackEdits, TrackId};
use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::ItemKey;
use lofty::{file::TaggedFile, probe::Probe, properties::FileProperties, tag::Tag};
use std::path::Path;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, RawValue, StandardTag, Tag as SymphoniaTag};
use symphonia::core::units::Timestamp;

pub(crate) struct ScannedTrack {
    pub track: Track,
    pub album: Album,
}

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

fn select_best_picture(pictures: &[Picture]) -> Option<&Picture> {
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
                .get_strings(ItemKey::TrackArtists)
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
        .and_then(|t| t.get_string(ItemKey::AlbumArtist))
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
        .and_then(|t| t.get_string(ItemKey::MusicBrainzReleaseId))
        .map(|s| s.to_string());

    let musicbrainz_recording_id = tag
        .and_then(|t| t.get_string(ItemKey::MusicBrainzRecordingId))
        .map(|s| s.to_string());

    let musicbrainz_track_id = tag
        .and_then(|t| t.get_string(ItemKey::MusicBrainzTrackId))
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
        id: TrackId::Local(track_path.to_path_buf()),
        cover: None,
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
        musicbrainz_recording_id,
        musicbrainz_track_id,
        playlist_item_id: None,
    }
}

pub(crate) fn read_metadata(track_path: &Path) -> Option<ScannedTrack> {
    let options = ParseOptions::new().read_cover_art(false);
    let tagged_file = match Probe::open(track_path).ok()?.options(options).read() {
        Ok(tagged_file) => tagged_file,
        Err(_) if is_matroska_audio(track_path) => {
            return read_with_symphonia(track_path);
        }
        Err(_) => return None,
    };
    let properties = tagged_file.properties();
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let track = extract_metadata(tag, properties, track_path);
    let album_artist = tag
        .and_then(|t| t.get_string(ItemKey::AlbumArtist))
        .map(|s| s.to_string())
        .unwrap_or_else(|| track.artist.clone());
    let genre = tag
        .and_then(|t| t.genre().map(|g| g.to_string()))
        .unwrap_or_else(|| "Unknown".to_string());
    let year = tag
        .and_then(|t| t.get_string(ItemKey::Year))
        .and_then(|s| s.get(..4).unwrap_or(s).parse::<u16>().ok())
        .unwrap_or(0);
    let album = Album {
        id: track.album_id.clone(),
        title: track.album.clone(),
        artist: album_artist,
        genre,
        year,
        cover_path: None,
        manual_cover: false,
    };

    Some(ScannedTrack { track, album })
}

pub fn read(track_path: &Path, _cover_cache: &Path, library: &mut Library) -> Option<Track> {
    let scanned = read_metadata(track_path)?;
    if !library
        .albums
        .iter()
        .any(|album| album.id == scanned.album.id)
    {
        library.albums.push(scanned.album);
    }
    library.add_track(scanned.track.clone());
    Some(scanned.track)
}

/// Write `edits` back into the audio file's primary tag. Empty string fields
/// and `None` numbers remove the corresponding tag entry. Creates a primary
/// tag of the format's default type if the file has none. This rewrites the
/// file in place — there's no undo.
pub fn write_tags(track_path: &Path, edits: &TrackEdits) -> Result<(), String> {
    use lofty::config::WriteOptions;
    use lofty::file::AudioFile;

    let mut tagged = Probe::open(track_path)
        .map_err(|e| e.to_string())?
        .read()
        .map_err(|e| e.to_string())?;

    if tagged.primary_tag().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = tagged
        .primary_tag_mut()
        .ok_or_else(|| "no writable tag for this format".to_string())?;

    let title = edits.title.trim();
    if title.is_empty() {
        tag.remove_title();
    } else {
        tag.set_title(title.to_string());
    }

    let artist = edits.artist.trim();
    // Compare against the same representation the editor seeds from (structured
    // TrackArtists joined with ", ", else the single artist field). Only rewrite
    // the artist tags when it actually changed, so editing unrelated fields keeps
    // the structured multi-artist data intact.
    let existing_track_artists: Vec<String> = tag
        .get_strings(ItemKey::TrackArtists)
        .flat_map(|s| s.split(';').map(|a| a.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    let existing_artist_repr = if existing_track_artists.is_empty() {
        tag.artist().map(|a| a.to_string()).unwrap_or_default()
    } else {
        existing_track_artists.join(", ")
    };
    if artist != existing_artist_repr {
        if artist.is_empty() {
            tag.remove_artist();
        } else {
            tag.set_artist(artist.to_string());
        }
        // Drop the now-stale structured split; re-derived from `artist` on scan.
        tag.remove_key(ItemKey::TrackArtists);
    }

    let album = edits.album.trim();
    if album.is_empty() {
        tag.remove_album();
    } else {
        tag.set_album(album.to_string());
    }

    match edits.track_number {
        Some(n) => tag.set_track(n),
        None => tag.remove_track(),
    }
    match edits.disc_number {
        Some(n) => tag.set_disk(n),
        None => tag.remove_disk(),
    }

    match &edits.cover {
        CoverChange::Keep => {}
        CoverChange::Remove => {
            // Clear every embedded picture, not just CoverFront — read_cover()
            // falls back to any picture type, so a leftover would reappear.
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
            }
        }
        CoverChange::Set(bytes) => {
            let mut picture = Picture::from_reader(&mut &bytes[..]).map_err(|e| e.to_string())?;
            picture.set_pic_type(PictureType::CoverFront);
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
            }
            tag.push_picture(picture);
        }
    }

    tagged
        .save_to_path(track_path, WriteOptions::default())
        .map_err(|e| e.to_string())
}

/// Read the embedded front-cover picture (or best available) as raw bytes plus
/// its MIME type, for previewing in the metadata editor. `None` if the file has
/// no embedded artwork.
pub fn read_cover(track_path: &Path) -> Option<(Vec<u8>, String)> {
    let tagged = Probe::open(track_path).ok()?.read().ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let picture = extract_embedded_cover(&tagged, tag)?;
    let mime = picture
        .mime_type()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "image/jpeg".to_string());
    Some((picture.data().to_vec(), mime))
}

fn is_matroska_audio(track_path: &Path) -> bool {
    track_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mka"))
}

fn symphonia_tag_to_string(tag: &SymphoniaTag) -> Option<String> {
    match &tag.raw.value {
        RawValue::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        RawValue::StringList(values) => {
            let joined = values.join(", ");
            let joined = joined.trim();
            (!joined.is_empty()).then(|| joined.to_string())
        }
        RawValue::UnsignedInt(value) => Some(value.to_string()),
        RawValue::SignedInt(value) => Some(value.to_string()),
        RawValue::Float(value) => Some(value.to_string()),
        RawValue::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn find_symphonia_tag<'a>(
    tags: &'a [SymphoniaTag],
    matches_std: impl Fn(&StandardTag) -> bool,
    fallback_keys: &[&str],
) -> Option<&'a SymphoniaTag> {
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

fn read_with_symphonia(track_path: &Path) -> Option<ScannedTrack> {
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
    if let Ok(mut format) = symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        {
            let mut metadata = format.metadata();
            if let Some(revision) = metadata.skip_to_latest() {
                tags.extend(revision.media.tags.iter().cloned());
            }
        }

        if let Some(track_info) = format
            .tracks()
            .iter()
            .find(|track| {
                track
                    .codec_params
                    .as_ref()
                    .and_then(|p| p.audio())
                    .is_some()
            })
            .or_else(|| format.tracks().first())
        {
            if let Some(audio) = track_info.codec_params.as_ref().and_then(|p| p.audio()) {
                sample_rate = audio.sample_rate.unwrap_or(0);
            }
            duration = track_info
                .time_base
                .zip(track_info.num_frames)
                .and_then(|(time_base, n_frames)| {
                    time_base.calc_time(Timestamp::from(n_frames as i64))
                })
                .map(|time| time.as_secs_f64().ceil().max(0.0) as u64)
                .unwrap_or(0);
        }
    }

    let artist = find_symphonia_tag(&tags, |t| matches!(t, StandardTag::Artist(_)), &["ARTIST"])
        .and_then(symphonia_tag_to_string)
        .unwrap_or_else(|| "Unknown Artist".to_string());

    let album_title = find_symphonia_tag(&tags, |t| matches!(t, StandardTag::Album(_)), &["ALBUM"])
        .and_then(symphonia_tag_to_string);

    let album_artist = find_symphonia_tag(
        &tags,
        |t| matches!(t, StandardTag::AlbumArtist(_)),
        &["ALBUMARTIST"],
    )
    .and_then(symphonia_tag_to_string)
    .unwrap_or_else(|| artist.clone());

    let parent_path = track_path.parent().map(|p| p.to_string_lossy());
    let grouping_key = album_title
        .as_deref()
        .and_then(|title| (!title.trim().is_empty()).then_some(album_artist.as_str()))
        .or(parent_path.as_deref())
        .unwrap_or(&artist);

    let title = find_symphonia_tag(
        &tags,
        |t| matches!(t, StandardTag::TrackTitle(_)),
        &["TITLE"],
    )
    .and_then(symphonia_tag_to_string)
    .or_else(|| {
        track_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    })
    .unwrap_or_else(|| "Unknown Title".to_string());

    let bitrate_kbps = (file_size * 8)
        .checked_div(duration)
        .map(|bps| (bps / 1000).min(u16::MAX as u64) as u16)
        .unwrap_or(0);

    let track = Track {
        id: TrackId::Local(track_path.to_path_buf()),
        cover: None,
        album_id: make_album_id(album_title.as_deref().unwrap_or(""), grouping_key),
        title,
        artist: artist.clone(),
        artists: vec![artist.clone()],
        album: album_title.unwrap_or_else(|| "Unknown Album".to_string()),
        khz: sample_rate,
        bitrate: bitrate_kbps,
        duration,
        track_number: find_symphonia_tag(
            &tags,
            |t| matches!(t, StandardTag::TrackNumber(_)),
            &["TRACKNUMBER"],
        )
        .and_then(symphonia_tag_to_string)
        .and_then(|value| value.parse().ok()),
        disc_number: find_symphonia_tag(
            &tags,
            |t| matches!(t, StandardTag::DiscNumber(_)),
            &["DISCNUMBER"],
        )
        .and_then(symphonia_tag_to_string)
        .and_then(|value| value.parse().ok()),
        musicbrainz_release_id: find_symphonia_tag(
            &tags,
            |t| matches!(t, StandardTag::MusicBrainzAlbumId(_)),
            &["MUSICBRAINZ_ALBUMID"],
        )
        .and_then(symphonia_tag_to_string),
        musicbrainz_recording_id: find_symphonia_tag(
            &tags,
            |t| matches!(t, StandardTag::MusicBrainzRecordingId(_)),
            &["MUSICBRAINZ_TRACKID"],
        )
        .and_then(symphonia_tag_to_string),
        musicbrainz_track_id: find_symphonia_tag(
            &tags,
            |t| matches!(t, StandardTag::MusicBrainzTrackId(_)),
            &["MUSICBRAINZ_RELEASETRACKID"],
        )
        .and_then(symphonia_tag_to_string),
        playlist_item_id: None,
    };

    let genre = find_symphonia_tag(&tags, |t| matches!(t, StandardTag::Genre(_)), &["GENRE"])
        .and_then(symphonia_tag_to_string)
        .unwrap_or_else(|| "Unknown".to_string());
    let year = find_symphonia_tag(
        &tags,
        |t| matches!(t, StandardTag::ReleaseDate(_) | StandardTag::ReleaseYear(_)),
        &["DATE", "YEAR"],
    )
    .and_then(symphonia_tag_to_string)
    .and_then(|value| value.get(..4).and_then(|prefix| prefix.parse::<u16>().ok()))
    .unwrap_or(0);
    let album = Album {
        id: track.album_id.clone(),
        title: track.album.clone(),
        artist: album_artist,
        genre,
        year,
        cover_path: None,
        manual_cover: false,
    };

    Some(ScannedTrack { track, album })
}
