use super::models::{Album, Library, Track};
use super::utils::{find_folder_cover, save_cover};
use lofty::prelude::*;
use lofty::tag::ItemKey;
use lofty::{probe::Probe, properties::FileProperties, tag::Tag};
use std::path::Path;

pub fn make_album_id(artist: &str, album: &str) -> String {
    format!(
        "alb_{}",
        format!("{artist}_{album}")
            .to_lowercase()
            .replace(' ', "_")
            .replace(|c: char| !c.is_alphanumeric() && c != '_', "")
    )
}

pub fn extract_embedded_cover(tag: Option<&Tag>) -> Option<Vec<u8>> {
    tag?.pictures().first().map(|pic| pic.data().to_vec())
}

pub fn extract_metadata(
    tag: Option<&Tag>,
    properties: &FileProperties,
    track_path: &Path,
) -> Track {
    let artist = tag
        .and_then(|t| t.artist().map(|a| a.to_string()))
        .unwrap_or_else(|| "Unknown Artist".to_string());

    let album_title = tag
        .and_then(|t| t.album().map(|a| a.to_string()))
        .unwrap_or_else(|| "Unknown Album".to_string());

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

    Track {
        path: track_path.to_path_buf(),
        album_id: make_album_id(&artist, &album_title),
        title,
        artist,
        album: album_title,
        khz: properties.sample_rate().unwrap_or(0),
        bitrate: properties.bit_depth().unwrap_or(0),
        duration: properties.duration().as_secs(),
        track_number: tag.and_then(|t| t.track()),
        disc_number: tag.and_then(|t| t.disk()),
        musicbrainz_release_id,
    }
}

pub fn read(track_path: &Path, cover_cache: &Path, library: &mut Library) -> Option<Track> {
    let tagged_file = Probe::open(track_path).ok()?.read().ok()?;
    let properties = tagged_file.properties();
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let track = extract_metadata(tag, properties, track_path);
    let album_id = track.album_id.clone();

    let album_exists = library.albums.iter().any(|a| a.id == album_id);

    if !album_exists {
        let mut cover = None;

        if let Some(bytes) = extract_embedded_cover(tag) {
            cover = save_cover(&album_id, &bytes, cover_cache).ok();
        } else if let Some(folder_cover) = find_folder_cover(track_path.parent()?) {
            cover = Some(folder_cover);
        }

        let genre = tag
            .and_then(|t| t.genre().map(|g| g.to_string()))
            .unwrap_or_else(|| "Unknown".to_string());
        let year = tag.and_then(|t| t.year()).unwrap_or(0) as u16;

        library.add_album(Album {
            id: album_id.clone(),
            title: track.album.clone(),
            artist: track.artist.clone(),
            genre,
            year,
            cover_path: cover,
        });
    }

    library.add_track(track.clone());
    Some(track)
}
