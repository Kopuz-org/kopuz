pub mod api;
pub mod auth;

pub mod cenc;
pub mod progressive;
pub mod signin;
pub mod stream;
pub mod types;
pub mod widevine;

pub use api::AppleMusicApi;

use config::MusicService;
use reader::models::{Track, TrackId};

pub fn apple_music_id(adam_id: impl Into<String>) -> TrackId {
    TrackId::Server {
        service: MusicService::AppleMusic,
        item_id: adam_id.into(),
    }
}

pub fn artwork_url(template: &str, size: u32) -> String {
    template
        .replace("{w}", &size.to_string())
        .replace("{h}", &size.to_string())
}

/// At most `max` bytes of `s`, backed off to a character boundary.
///
/// For truncating a response body into a log line. Apple's bodies are UTF-8 and
/// routinely non-ASCII — any track title outside Latin-1 will do it — so a plain
/// `&body[..max]` can land inside a character and panic. These call sites are
/// all on error paths, so that panic would replace the diagnostic being written.
pub(crate) fn head(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Convert a catalog track response to a reader::Track.
pub fn track_from_song_data(song: &types::TrackData) -> Track {
    let cover = if !song.attributes.artwork.url.is_empty() {
        Some(artwork_url(&song.attributes.artwork.url, 600))
    } else {
        None
    };

    let artist = if !song.relationships.artists.data.is_empty() {
        song.relationships
            .artists
            .data
            .iter()
            .map(|a| {
                a.attributes
                    .as_ref()
                    .map(|att| att.name.as_str())
                    .unwrap_or("Unknown Artist")
            })
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        song.attributes.artist_name.clone()
    };

    let artists = if song.relationships.artists.data.is_empty() {
        vec![song.attributes.artist_name.clone()]
    } else {
        song.relationships
            .artists
            .data
            .iter()
            .map(|a| {
                a.attributes
                    .as_ref()
                    .map(|att| att.name.clone())
                    .unwrap_or_else(|| "Unknown Artist".to_string())
            })
            .collect()
    };

    let album_id = song
        .relationships
        .albums
        .data
        .first()
        .map(|a| format!("applemusic:{}", a.id))
        .unwrap_or_default();

    Track {
        id: apple_music_id(&song.id),
        cover,
        album_id,
        title: song.attributes.name.clone(),
        artist,
        album: song.attributes.album_name.clone(),
        duration: song.attributes.durationInMillis / 1000,
        khz: 0,
        bitrate: 0,
        track_number: Some(song.attributes.trackNumber),
        disc_number: Some(song.attributes.discNumber),
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists,
    }
}

/// Convert one row of a library playlist to a reader::Track.
///
/// A library playlist returns *library* songs: the resource id identifies that
/// row — it's what removal has to target, and the same song added twice is two
/// rows with two ids — while playback still needs the catalog Adam ID.
pub fn track_from_playlist_entry(song: &types::TrackData) -> Track {
    let mut track = track_from_song_data(song);
    if let Some(catalog_id) = song
        .attributes
        .playParams
        .as_ref()
        .and_then(|p| p.catalog_id.as_deref())
        .filter(|s| !s.is_empty())
    {
        track.id = apple_music_id(catalog_id);
    }
    track.playlist_item_id = Some(song.id.clone());
    track
}

/// Convert a library song resource to a reader::Track.
/// Uses playParams.catalogId (the Adam ID) when available, falling back to the
/// library ID. The web playback API requires Adam IDs, not library IDs.
pub fn track_from_library_song(song: &types::LibrarySongResource) -> Track {
    let cover = song
        .attributes
        .artwork
        .as_ref()
        .filter(|a| !a.url.is_empty())
        .map(|a| artwork_url(&a.url, 600));

    // Use catalogId (Adam ID) for playback — web playback API requires it.
    let playback_id = song
        .attributes
        .playParams
        .as_ref()
        .and_then(|p| p.catalog_id.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or(&song.id);

    tracing::debug!(
        "am.track_from_library_song: library_id={}, catalog_id={:?}, playback_id={}",
        song.id,
        song.attributes
            .playParams
            .as_ref()
            .and_then(|p| p.catalog_id.as_deref()),
        playback_id
    );

    Track {
        id: apple_music_id(playback_id),
        cover,
        album_id: String::new(),
        title: song.attributes.name.clone(),
        artist: song.attributes.artistName.clone(),
        album: song.attributes.albumName.clone(),
        duration: song.attributes.durationInMillis / 1000,
        khz: 0,
        bitrate: 0,
        track_number: Some(song.attributes.trackNumber),
        disc_number: Some(song.attributes.discNumber),
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec![song.attributes.artistName.clone()],
    }
}

/// Convert a library album resource to a reader::Album.
pub fn album_from_library(album: &types::LibraryAlbumResource) -> reader::Album {
    reader::Album {
        id: format!("applemusic:{}", album.id),
        title: album.attributes.name.clone(),
        artist: album.attributes.artistName.clone(),
        genre: album.attributes.genreNames.join(", "),
        year: album
            .attributes
            .releaseDate
            .split('-')
            .next()
            .and_then(|y| y.parse().ok())
            .unwrap_or(0),
        cover_path: album
            .attributes
            .artwork
            .as_ref()
            .filter(|a| !a.url.is_empty())
            .map(|a| {
                std::path::PathBuf::from(format!(
                    "applemusic:{}:{}",
                    album.id,
                    artwork_url(&a.url, 600)
                ))
            }),
        manual_cover: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A playlist row carries two ids and they aren't interchangeable: the
    /// library id addresses the row (removal needs it), the catalog id is what
    /// plays. Mapping the row id into `Track::id` produces tracks that can't be
    /// streamed; dropping it produces tracks that can't be removed.
    #[test]
    fn a_playlist_entry_keeps_the_row_id_and_plays_the_catalog_id() {
        let entry: types::TrackData = serde_json::from_value(serde_json::json!({
            "id": "i.rowid123",
            "type": "library-songs",
            "attributes": {
                "name": "Kool-Aid",
                "artistName": "Bring Me The Horizon",
                "albumName": "POST HUMAN: NeX GEn",
                "durationInMillis": 208_000,
                "playParams": { "id": "i.rowid123", "kind": "song", "catalogId": "1811922756" }
            }
        }))
        .expect("library song");

        let track = track_from_playlist_entry(&entry);
        assert_eq!(track.id.key(), "1811922756", "playback needs the Adam ID");
        assert_eq!(track.playlist_item_id.as_deref(), Some("i.rowid123"));
        assert_eq!(track.title, "Kool-Aid");
        assert_eq!(track.duration, 208);
    }

    /// A catalog song reached through a playlist has no `catalogId`; its own id
    /// already is one, so it must not be blanked.
    #[test]
    fn a_playlist_entry_without_a_catalog_id_keeps_its_own() {
        let entry: types::TrackData = serde_json::from_value(serde_json::json!({
            "id": "1811922756",
            "type": "songs",
            "attributes": { "name": "Kool-Aid" }
        }))
        .expect("catalog song");

        let track = track_from_playlist_entry(&entry);
        assert_eq!(track.id.key(), "1811922756");
        assert_eq!(track.playlist_item_id.as_deref(), Some("1811922756"));
    }
}
