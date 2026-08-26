//! Reactive library query hooks backed by the frontend-neutral daemon API.

use std::path::PathBuf;
use std::sync::Arc;

use api::KopuzApi;
use db::{Page, Source, TrackFilter, TrackSort};
use dioxus::prelude::*;
use tracing::Instrument;
use utils::offload;

use crate::db_reactivity::{Table, use_generations};

#[derive(Clone, Default, PartialEq)]
pub struct WindowRows {
    pub offset: u32,
    pub rows: Vec<reader::Track>,
}

#[derive(Clone, Copy)]
pub struct TracksWindow {
    pub rows: Resource<WindowRows>,
    pub total: Resource<u32>,
}

pub fn page_to_api(value: Page) -> api::Page {
    api::Page {
        offset: value.offset,
        limit: value.limit,
    }
}

fn all() -> api::Page {
    api::Page {
        offset: 0,
        limit: u32::MAX,
    }
}

fn sort_key(sort: &TrackSort) -> Option<String> {
    match sort {
        TrackSort::ArtistAlbum => None,
        TrackSort::Title => Some("title".to_string()),
        TrackSort::Artist => Some("artist".to_string()),
        TrackSort::Album => Some("album".to_string()),
        TrackSort::DateAdded => Some("date_added".to_string()),
        TrackSort::PlayCount => Some("play_count".to_string()),
        TrackSort::Fields(fields) => serde_json::to_string(fields)
            .ok()
            .map(|json| format!("fields:{json}")),
    }
}

pub fn track_filter_to_api(value: &TrackFilter) -> api::TrackFilter {
    api::TrackFilter {
        search: (!value.search.is_empty()).then(|| value.search.clone()),
        sort: sort_key(&value.sort),
        ..Default::default()
    }
}

fn music_service(value: api::MusicService) -> Option<config::MusicService> {
    Some(match value {
        api::MusicService::Jellyfin => config::MusicService::Jellyfin,
        api::MusicService::Subsonic => config::MusicService::Subsonic,
        api::MusicService::Custom => config::MusicService::Custom,
        api::MusicService::YtMusic => config::MusicService::YtMusic,
        api::MusicService::AppleMusic => config::MusicService::AppleMusic,
        api::MusicService::SoundCloud => config::MusicService::SoundCloud,
        api::MusicService::Spotify => config::MusicService::Spotify,
        api::MusicService::Nextcloud => config::MusicService::Nextcloud,
        api::MusicService::Unknown => return None,
    })
}

fn hex(value: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn artwork_url(kind: &str, value: &str) -> String {
    let value = hex(value);
    if cfg!(target_os = "windows") {
        format!("http://artwork.dioxus.localhost/api?{kind}={value}")
    } else {
        format!("artwork://api?{kind}={value}")
    }
}

pub fn artist_artwork_url(name: &str) -> String {
    artwork_url("artist", name)
}

fn stored_artwork_ref(kind: &str, value: &str) -> String {
    format!("directurl:{}", artwork_url(kind, value))
}

pub fn track_from_api(value: api::TrackInfo) -> reader::Track {
    let id = value
        .service
        .and_then(music_service)
        .map(|service| reader::TrackId::Server {
            service,
            item_id: value.key.clone(),
        })
        .unwrap_or_else(|| reader::TrackId::Local(PathBuf::from(&value.key)));
    let duration = match value.kind {
        api::TrackKind::Radio => u64::MAX,
        api::TrackKind::Normal => value.duration_ms.unwrap_or_default() / 1000,
    };
    reader::Track {
        id,
        cover: value
            .artwork
            .as_ref()
            .map(|key| stored_artwork_ref("track", key)),
        album_id: value.album_id,
        title: value.title,
        artist: value.artist,
        album: value.album,
        duration,
        khz: value.khz,
        bitrate: value.bitrate,
        track_number: value.track_number,
        disc_number: value.disc_number,
        musicbrainz_release_id: value.musicbrainz_release_id,
        musicbrainz_recording_id: value.musicbrainz_recording_id,
        musicbrainz_track_id: value.musicbrainz_track_id,
        playlist_item_id: value.playlist_item_id,
        artists: value.artists,
    }
}

pub async fn save_track_edits(
    api: &dyn KopuzApi,
    key: String,
    edits: reader::TrackEdits,
) -> Result<api::TrackInfo, api::ApiError> {
    let updated = api
        .update_track_metadata(api::TrackMetadataPatch {
            key: key.clone(),
            title: Some(edits.title),
            artist: Some(edits.artist),
            album: Some(edits.album),
            track_number: edits.track_number,
            clear_track_number: edits.track_number.is_none(),
            disc_number: edits.disc_number,
            clear_disc_number: edits.disc_number.is_none(),
        })
        .await?;
    match edits.cover {
        reader::CoverChange::Keep => {}
        reader::CoverChange::Remove => {
            api.remove_artwork(api::ArtworkTarget::Track { key })
                .await?;
        }
        reader::CoverChange::Set(data) => {
            api.upload_artwork(api::ArtworkUpload {
                target: Some(api::ArtworkTarget::Track { key }),
                content_type: "application/octet-stream".to_string(),
                data,
            })
            .await?;
        }
    }
    Ok(updated)
}

pub fn album_from_api(value: api::AlbumInfo) -> reader::Album {
    reader::Album {
        id: value.id.clone(),
        title: value.title,
        artist: value.artist,
        genre: value.genre,
        year: value.year.min(u32::from(u16::MAX)) as u16,
        cover_path: value
            .artwork
            .map(|key| PathBuf::from(stored_artwork_ref("album", &key))),
        manual_cover: value.manual_artwork,
    }
}

fn api_or_default<T: Default>(result: Result<T, api::ApiError>, operation: &'static str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, operation, "frontend API query failed");
            T::default()
        }
    }
}

pub fn use_tracks_window(filter: Memo<TrackFilter>, page_value: Memo<Page>) -> TracksWindow {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();

    let rows = use_resource({
        let api = api.clone();
        move || {
            let _ = gens.generation(Table::Tracks);
            let (api, filter, page_value) = (api.clone(), filter(), page_value());
            let span = tracing::info_span!(
                "query.tracks_page",
                filter = ?filter,
                offset = page_value.offset,
                limit = page_value.limit,
                rows = tracing::field::Empty,
            );
            offload(
                async move {
                    let result = api_or_default(
                        api.tracks(track_filter_to_api(&filter), page_to_api(page_value))
                            .await,
                        "tracks",
                    );
                    tracing::Span::current().record("rows", result.items.len());
                    WindowRows {
                        offset: result.offset,
                        rows: result.items.into_iter().map(track_from_api).collect(),
                    }
                }
                .instrument(span),
            )
        }
    });

    let total = use_resource({
        let api = api.clone();
        move || {
            let _ = gens.generation(Table::Tracks);
            let (api, filter, page_value) = (api.clone(), filter(), page_value());
            offload(async move {
                api_or_default(
                    api.tracks(track_filter_to_api(&filter), page_to_api(page_value))
                        .await,
                    "tracks.total",
                )
                .total
            })
        }
    });

    TracksWindow { rows, total }
}

pub fn use_album_tracks(
    source: Memo<Source>,
    album_id: Memo<String>,
) -> Resource<Vec<reader::Track>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, _source, id) = (api.clone(), source(), album_id());
        offload(async move {
            if id.is_empty() {
                return Vec::new();
            }
            api_or_default(api.album_tracks(id, all()).await, "album_tracks")
                .items
                .into_iter()
                .map(track_from_api)
                .collect()
        })
    })
}

pub fn use_artist_tracks(
    source: Memo<Source>,
    artist: Memo<String>,
) -> Resource<Vec<reader::Track>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, _source, artist) = (api.clone(), source(), artist());
        offload(async move {
            api_or_default(api.artist_tracks(artist, all()).await, "artist_tracks")
                .items
                .into_iter()
                .map(track_from_api)
                .collect()
        })
    })
}

pub fn use_genre_tracks(source: Memo<Source>, genre: Memo<String>) -> Resource<Vec<reader::Track>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, _source, genre) = (api.clone(), source(), genre());
        offload(async move {
            if genre.is_empty() {
                return Vec::new();
            }
            api_or_default(api.genre_tracks(genre, all()).await, "genre_tracks")
                .items
                .into_iter()
                .map(track_from_api)
                .collect()
        })
    })
}

pub fn use_artist_sample_tracks(source: Memo<Source>, limit: u32) -> Resource<Vec<reader::Track>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, _source) = (api.clone(), source());
        offload(async move {
            api_or_default(
                api.artist_sample_tracks(api::Page { offset: 0, limit })
                    .await,
                "artist_sample_tracks",
            )
            .items
            .into_iter()
            .map(track_from_api)
            .collect()
        })
    })
}

pub fn use_top_genre(source: Memo<Source>) -> Resource<Option<String>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, _source) = (api.clone(), source());
        offload(async move { api_or_default(api.top_genre().await, "top_genre") })
    })
}

pub fn use_tracks_by_keys(
    source: Memo<Source>,
    keys: Memo<Vec<String>>,
) -> Resource<Vec<reader::Track>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, _source, keys) = (api.clone(), source(), keys());
        offload(async move {
            if keys.is_empty() {
                return Vec::new();
            }
            api_or_default(api.tracks_by_keys(keys).await, "tracks_by_keys")
                .into_iter()
                .map(track_from_api)
                .collect()
        })
    })
}

pub fn use_recently_played(source: Memo<Source>) -> Resource<Vec<reader::Track>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Recents);
        let (api, _source) = (api.clone(), source());
        offload(async move {
            api_or_default(
                api.recent_tracks(api::Page {
                    offset: 0,
                    limit: 50,
                })
                .await,
                "recent_tracks",
            )
            .items
            .into_iter()
            .map(track_from_api)
            .collect()
        })
    })
}

pub fn use_album(source: Memo<Source>, album_id: Memo<String>) -> Resource<Option<reader::Album>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (api, _source, id) = (api.clone(), source(), album_id());
        offload(async move {
            if id.is_empty() {
                return None;
            }
            match api.album(id).await {
                Ok(value) => Some(album_from_api(value)),
                Err(error) if error.code == api::ErrorCode::NotFound => None,
                Err(error) => {
                    tracing::warn!(%error, operation = "album", "frontend API query failed");
                    None
                }
            }
        })
    })
}

pub fn use_artists(source: Memo<Source>) -> Resource<Vec<(String, u32)>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let (api, _source) = (api.clone(), source());
        offload(async move {
            api_or_default(api.artists(all()).await, "artists")
                .items
                .into_iter()
                .map(|artist| (artist.name, artist.track_count))
                .collect()
        })
    })
}

pub fn use_active_source() -> Memo<config::Source> {
    let config = use_context::<Signal<config::AppConfig>>();
    use_memo(move || config.read().active_source.clone())
}

pub fn use_playlists() -> Resource<reader::PlaylistStore> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    let source = use_active_source();
    use_resource(move || {
        let _ = gens.generation(Table::Playlists);
        let _ = gens.generation(Table::Folders);
        let (api, _source) = (api.clone(), source());
        offload(async move {
            let catalog = api_or_default(api.playlists().await, "playlists");
            reader::PlaylistStore {
                playlists: catalog
                    .playlists
                    .into_iter()
                    .map(|playlist| {
                        let artwork = playlist
                            .artwork
                            .map(|key| stored_artwork_ref("playlist", &key));
                        reader::models::Playlist {
                            id: playlist.id,
                            name: playlist.name,
                            tracks: playlist.track_keys,
                            image_tag: (!playlist.manual_artwork)
                                .then(|| artwork.clone())
                                .flatten(),
                            cover_path: playlist
                                .manual_artwork
                                .then(|| artwork.map(PathBuf::from))
                                .flatten(),
                        }
                    })
                    .collect(),
                folders: catalog
                    .folders
                    .into_iter()
                    .map(|folder| reader::PlaylistFolder {
                        id: folder.id,
                        name: folder.name,
                        playlist_ids: folder.playlist_ids,
                    })
                    .collect(),
            }
        })
    })
}

pub fn use_artist_images() -> Resource<db::ArtistImages> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Tracks);
        let api = api.clone();
        offload(async move {
            let artists = api_or_default(api.artists(all()).await, "artist_images");
            let mut overrides = std::collections::HashMap::new();
            let mut photos = std::collections::HashMap::new();
            for artist in artists.items {
                if artist.artwork.is_none() {
                    continue;
                }
                let normalized = utils::artist::normalize_artist_key(&artist.name);
                let url = artwork_url("artist", &artist.name);
                if artist.manual_artwork {
                    overrides.insert(
                        normalized,
                        PathBuf::from(stored_artwork_ref("artist", &artist.name)),
                    );
                } else {
                    photos.insert(normalized, reader::ArtistImageRef::Remote(url));
                }
            }
            (overrides, photos)
        })
    })
}

pub fn use_albums(source: Memo<Source>) -> Resource<Vec<reader::Album>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Albums);
        let (api, _source) = (api.clone(), source());
        offload(async move {
            api_or_default(
                api.albums(api::AlbumFilter::default(), all()).await,
                "albums",
            )
            .items
            .into_iter()
            .map(album_from_api)
            .collect()
        })
    })
}

pub fn use_cover_resolver(max_width: u32) -> impl Fn(&reader::Track) -> Option<utils::CoverUrl> {
    let config = use_context::<Signal<config::AppConfig>>();
    move |track: &reader::Track| ::server::cover::track(&config.read(), track, max_width)
}

pub fn use_favorites() -> Resource<Vec<String>> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Favorites);
        let api = api.clone();
        offload(async move { api_or_default(api.favorites().await, "favorites").refs })
    })
}

pub fn use_track_is_favorite(track_value: Memo<Option<reader::Track>>) -> Memo<bool> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let gens = use_generations();
    let resource = use_resource(move || {
        let _ = gens.generation(Table::Favorites);
        let (api, track_value) = (api.clone(), track_value());
        offload(async move {
            let Some(track_value) = track_value else {
                return false;
            };
            let key = track_value.id.key().into_owned();
            if key.trim().is_empty() {
                return false;
            }
            api_or_default(api.favorites().await, "favorite")
                .refs
                .contains(&key)
        })
    });
    use_memo(move || resource.read().unwrap_or(false))
}
