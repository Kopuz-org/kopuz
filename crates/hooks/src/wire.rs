//! Turning API rows into the models the UI renders.
//!
//! Pages still render `reader::Track` and `reader::Album`; only the source of
//! those rows changed. The one real difference is the cover: an API row says
//! whether artwork exists and names the entity it belongs to, because the
//! daemon may be another process and the credentials that sign a server cover
//! URL never leave it. That name becomes an `artwork://api?...` URL, which
//! `CoverRef::parse` reads as a self-contained embedded URL, so every
//! `server::cover::*` call site keeps working unchanged.

use api::{AlbumInfo, TrackInfo, TrackKind};
use reader::{Album, Track, TrackId};

/// The cover URL for an artwork ref, or `None` when the daemon has no cover
/// for the entity -- in which case the UI draws its placeholder and never
/// asks for an image that does not exist.
pub fn artwork_url(artwork: Option<&api::ArtworkRef>) -> Option<utils::CoverUrl> {
    let artwork = artwork?;
    Some(utils::format_entity_artwork_url(
        artwork.target.kind(),
        artwork.target.id(),
        artwork.version,
        false,
    ))
}

fn artwork_string(artwork: Option<&api::ArtworkRef>) -> Option<String> {
    artwork_url(artwork).map(|url| url.as_ref().to_string())
}

pub fn track_from_api(info: TrackInfo) -> Track {
    Track {
        // `uid` is exactly the "service:id" form TrackId round-trips through.
        id: TrackId::from_legacy_path(&info.uid),
        cover: artwork_string(info.artwork.as_ref()),
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
        musicbrainz_release_id: info.musicbrainz_release_id,
        musicbrainz_recording_id: info.musicbrainz_recording_id,
        musicbrainz_track_id: info.musicbrainz_track_id,
        playlist_item_id: info.playlist_item_id,
        artists: info.artists,
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
        cover_path: artwork_string(info.artwork.as_ref()).map(std::path::PathBuf::from),
        manual_cover: false,
    }
}

pub fn albums_from_api(items: Vec<AlbumInfo>) -> Vec<Album> {
    items.into_iter().map(album_from_api).collect()
}

/// The playlist catalog as the views render it. `image_tag` stays empty on
/// purpose: the daemon already walked the explicit cover, the server's tag
/// and the first track's art, so the resolved ref is the whole answer and the
/// fallback chain is not repeated here.
pub fn playlist_store_from_api(catalog: api::PlaylistCatalog) -> reader::PlaylistStore {
    reader::PlaylistStore {
        playlists: catalog
            .playlists
            .into_iter()
            .map(|playlist| reader::models::Playlist {
                cover_path: artwork_string(playlist.artwork.as_ref()).map(std::path::PathBuf::from),
                image_tag: None,
                id: playlist.id,
                name: playlist.name,
                tracks: playlist.track_keys,
            })
            .collect(),
        folders: catalog
            .folders
            .into_iter()
            .map(|folder| reader::models::PlaylistFolder {
                id: folder.id,
                name: folder.name,
                playlist_ids: folder.playlist_ids,
            })
            .collect(),
    }
}
