//! Turning API rows into the models the UI renders.
//!
//! Pages still render `reader::Track` and `reader::Album`; only the source of
//! those rows changed. The one real difference is the cover: an API row names
//! the entity its artwork belongs to instead of a path, because the daemon may
//! be another process and the credentials that sign a server cover URL never
//! leave it. That name becomes an `artwork://api?...` URL, which parses as an
//! ordinary embedded-cover ref, so every `server::cover::*` call site keeps
//! working unchanged.

use api::{AlbumInfo, ArtworkTarget, TrackInfo, TrackKind};
use reader::{Album, Track, TrackId};

/// The cover URL for an artwork target, or `None` when the daemon has no
/// cover for the entity.
pub fn artwork_url(target: Option<&ArtworkTarget>) -> Option<String> {
    let target = target?;
    let (kind, id) = match target {
        ArtworkTarget::Track(key) => ("track", key),
        ArtworkTarget::Album(id) => ("album", id),
        ArtworkTarget::Artist(name) => ("artist", name),
    };
    Some(
        utils::format_entity_artwork_url(kind, id, false)
            .as_ref()
            .to_string(),
    )
}

pub fn track_from_api(info: TrackInfo) -> Track {
    let cover = artwork_url(Some(&ArtworkTarget::Track(info.key.clone())));
    Track {
        // `uid` is exactly the "service:id" form TrackId round-trips through.
        id: TrackId::from_legacy_path(&info.uid),
        cover,
        album_id: info.album_id,
        title: info.title,
        artist: info.artist,
        album: info.album,
        // The wire says "radio" instead of carrying the duration sentinel.
        duration: match info.kind {
            TrackKind::Radio => u64::MAX,
            TrackKind::Normal => info.duration_ms.unwrap_or_default() / 1000,
        },
        khz: info.khz,
        bitrate: info.bitrate,
        track_number: info.track_number,
        disc_number: info.disc_number,
        musicbrainz_release_id: None,
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: Vec::new(),
    }
}

pub fn tracks_from_api(items: Vec<TrackInfo>) -> Vec<Track> {
    items.into_iter().map(track_from_api).collect()
}

pub fn album_from_api(info: AlbumInfo) -> Album {
    Album {
        id: info.id,
        title: info.title,
        artist: info.artist,
        genre: info.genre,
        year: info.year,
        cover_path: None,
        manual_cover: false,
    }
}

pub fn albums_from_api(items: Vec<AlbumInfo>) -> Vec<Album> {
    items.into_iter().map(album_from_api).collect()
}
