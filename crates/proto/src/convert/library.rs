use super::*;
use crate::*;

pub fn track_filter_to_proto(value: &api::TrackFilter) -> TrackFilter {
    TrackFilter {
        search: value.search.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        genre: value.genre.clone(),
        favorite: value.favorite,
        sort: track_sort_to_proto(&value.sort) as i32,
        sort_fields: sort_criteria_to_proto(&value.sort),
    }
}

pub fn track_filter_from_proto(value: &TrackFilter) -> api::TrackFilter {
    api::TrackFilter {
        search: value.search.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        genre: value.genre.clone(),
        favorite: value.favorite,
        sort: track_sort_from_proto(value.sort, &value.sort_fields),
    }
}

pub fn page_to_proto(value: api::Page) -> Page {
    Page {
        offset: value.offset,
        limit: value.limit,
    }
}

pub fn page_from_proto(value: Option<&Page>) -> api::Page {
    let value = value.cloned().unwrap_or_default();
    api::Page {
        offset: value.offset,
        limit: if value.limit == 0 {
            api::DEFAULT_PAGE_LIMIT
        } else {
            value.limit
        },
    }
}

pub fn track_info_to_proto(value: &api::TrackInfo) -> TrackInfo {
    TrackInfo {
        key: value.key.clone(),
        uid: value.uid.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        album_id: value.album_id.clone(),
        duration_ms: value.duration_ms,
        khz: value.khz,
        bitrate: u32::from(value.bitrate),
        track_number: value.track_number,
        disc_number: value.disc_number,
        kind: track_kind_to_proto(value.kind) as i32,
        seekable: value.seekable,
        offline: value.offline,
        service: value
            .service
            .map(|service| music_service_to_proto(service) as i32),
        artists: value.artists.clone(),
        musicbrainz_release_id: value.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: value.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: value.musicbrainz_track_id.clone(),
        playlist_item_id: value.playlist_item_id.clone(),
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
    }
}

pub fn track_info_from_proto(value: &TrackInfo) -> api::TrackInfo {
    api::TrackInfo {
        key: value.key.clone(),
        uid: value.uid.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        album_id: value.album_id.clone(),
        duration_ms: value.duration_ms,
        khz: value.khz,
        bitrate: value.bitrate.min(u32::from(u16::MAX)) as u16,
        track_number: value.track_number,
        disc_number: value.disc_number,
        kind: track_kind_from_proto(value.kind),
        seekable: value.seekable,
        offline: value.offline,
        service: value.service.and_then(music_service_from_proto),
        artists: value.artists.clone(),
        musicbrainz_release_id: value.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: value.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: value.musicbrainz_track_id.clone(),
        playlist_item_id: value.playlist_item_id.clone(),
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
    }
}

pub fn track_page_to_proto(value: &api::TrackPage) -> TrackPage {
    TrackPage {
        total: value.total,
        offset: value.offset,
        items: value.items.iter().map(track_info_to_proto).collect(),
    }
}

pub fn track_page_from_proto(value: &TrackPage) -> api::TrackPage {
    api::TrackPage {
        total: value.total,
        offset: value.offset,
        items: value.items.iter().map(track_info_from_proto).collect(),
    }
}

pub fn lyrics_to_proto(value: &api::LyricsView) -> Lyrics {
    Lyrics {
        plain: value.plain.clone(),
        synced: value
            .synced
            .iter()
            .map(|line| LyricLine {
                start_ms: line.start_ms,
                end_ms: line.end_ms,
                text: line.text.clone(),
                chunks: line
                    .chunks
                    .iter()
                    .map(|chunk| LyricChunk {
                        start_ms: chunk.start_ms,
                        text: chunk.text.clone(),
                    })
                    .collect(),
                parent_line_index: line.parent_line_index,
                background: line.background,
                opposite_turn: line.opposite_turn,
            })
            .collect(),
    }
}

pub fn lyrics_from_proto(value: &Lyrics) -> api::LyricsView {
    api::LyricsView {
        plain: value.plain.clone(),
        synced: value
            .synced
            .iter()
            .map(|line| api::LyricLineView {
                start_ms: line.start_ms,
                end_ms: line.end_ms,
                text: line.text.clone(),
                chunks: line
                    .chunks
                    .iter()
                    .map(|chunk| api::LyricChunkView {
                        start_ms: chunk.start_ms,
                        text: chunk.text.clone(),
                    })
                    .collect(),
                parent_line_index: line.parent_line_index,
                background: line.background,
                opposite_turn: line.opposite_turn,
            })
            .collect(),
    }
}

pub fn stats_to_proto(value: &api::StatsView) -> Stats {
    Stats {
        listen_counts: value.listen_counts.clone().into_iter().collect(),
    }
}

pub fn stats_from_proto(value: &Stats) -> api::StatsView {
    api::StatsView {
        listen_counts: value.listen_counts.clone().into_iter().collect(),
    }
}

pub fn artwork_request_to_proto(value: &api::ArtworkRequest) -> ArtworkRequest {
    use artwork_request::Entity;
    let entity = match &value.target {
        api::ArtworkTarget::Track(key) => Entity::Track(key.clone()),
        api::ArtworkTarget::Album(id) => Entity::Album(id.clone()),
        api::ArtworkTarget::Artist(name) => Entity::Artist(name.clone()),
        api::ArtworkTarget::Playlist(id) => Entity::Playlist(id.clone()),
        api::ArtworkTarget::Catalog(id) => Entity::Catalog(id.clone()),
        api::ArtworkTarget::Station(id) => Entity::Station(id.clone()),
    };
    ArtworkRequest {
        entity: Some(entity),
        hq: value.hq,
    }
}

pub fn artwork_request_from_proto(value: &ArtworkRequest) -> Option<api::ArtworkRequest> {
    use artwork_request::Entity;
    let target = match value.entity.as_ref()? {
        Entity::Track(key) => api::ArtworkTarget::Track(key.clone()),
        Entity::Album(id) => api::ArtworkTarget::Album(id.clone()),
        Entity::Artist(name) => api::ArtworkTarget::Artist(name.clone()),
        Entity::Playlist(id) => api::ArtworkTarget::Playlist(id.clone()),
        Entity::Catalog(id) => api::ArtworkTarget::Catalog(id.clone()),
        Entity::Station(id) => api::ArtworkTarget::Station(id.clone()),
    };
    Some(api::ArtworkRequest {
        target,
        hq: value.hq,
    })
}

pub fn artwork_target_to_proto(value: &api::ArtworkTarget) -> ArtworkTarget {
    use artwork_target::Entity;
    let entity = match value {
        api::ArtworkTarget::Track(key) => Entity::Track(key.clone()),
        api::ArtworkTarget::Album(id) => Entity::Album(id.clone()),
        api::ArtworkTarget::Artist(name) => Entity::Artist(name.clone()),
        api::ArtworkTarget::Playlist(id) => Entity::Playlist(id.clone()),
        api::ArtworkTarget::Catalog(id) => Entity::Catalog(id.clone()),
        api::ArtworkTarget::Station(id) => Entity::Station(id.clone()),
    };
    ArtworkTarget {
        entity: Some(entity),
    }
}

pub fn artwork_target_from_proto(value: &ArtworkTarget) -> Option<api::ArtworkTarget> {
    use artwork_target::Entity;
    Some(match value.entity.as_ref()? {
        Entity::Track(key) => api::ArtworkTarget::Track(key.clone()),
        Entity::Album(id) => api::ArtworkTarget::Album(id.clone()),
        Entity::Artist(name) => api::ArtworkTarget::Artist(name.clone()),
        Entity::Playlist(id) => api::ArtworkTarget::Playlist(id.clone()),
        Entity::Catalog(id) => api::ArtworkTarget::Catalog(id.clone()),
        Entity::Station(id) => api::ArtworkTarget::Station(id.clone()),
    })
}

pub fn artwork_ref_to_proto(value: &api::ArtworkRef) -> ArtworkRef {
    ArtworkRef {
        target: Some(artwork_target_to_proto(&value.target)),
        version: value.version,
    }
}

/// A ref whose target failed to decode is no ref at all: a client would
/// otherwise ask for artwork it cannot name.
pub fn artwork_ref_from_proto(value: &ArtworkRef) -> Option<api::ArtworkRef> {
    Some(api::ArtworkRef {
        target: artwork_target_from_proto(value.target.as_ref()?)?,
        version: value.version,
    })
}

pub fn music_service_to_proto(value: ::config::MusicService) -> MusicService {
    match value {
        ::config::MusicService::Jellyfin => MusicService::Jellyfin,
        ::config::MusicService::Subsonic => MusicService::Subsonic,
        ::config::MusicService::Custom => MusicService::Custom,
        ::config::MusicService::YtMusic => MusicService::YtMusic,
        ::config::MusicService::AppleMusic => MusicService::AppleMusic,
        ::config::MusicService::SoundCloud => MusicService::Soundcloud,
        ::config::MusicService::Spotify => MusicService::Spotify,
        ::config::MusicService::Nextcloud => MusicService::Nextcloud,
    }
}

/// `None` for both the explicit unknown and any value this build predates --
/// a service it cannot name is one it cannot branch on anyway.
pub fn music_service_from_proto(value: i32) -> Option<::config::MusicService> {
    Some(match MusicService::try_from(value).ok()? {
        MusicService::Unknown => return None,
        MusicService::Jellyfin => ::config::MusicService::Jellyfin,
        MusicService::Subsonic => ::config::MusicService::Subsonic,
        MusicService::Custom => ::config::MusicService::Custom,
        MusicService::YtMusic => ::config::MusicService::YtMusic,
        MusicService::AppleMusic => ::config::MusicService::AppleMusic,
        MusicService::Soundcloud => ::config::MusicService::SoundCloud,
        MusicService::Spotify => ::config::MusicService::Spotify,
        MusicService::Nextcloud => ::config::MusicService::Nextcloud,
    })
}

pub fn album_info_to_proto(value: &api::AlbumInfo) -> AlbumInfo {
    AlbumInfo {
        id: value.id.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        genre: value.genre.clone(),
        year: value.year as u32,
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
    }
}

pub fn album_info_from_proto(value: &AlbumInfo) -> api::AlbumInfo {
    api::AlbumInfo {
        id: value.id.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        genre: value.genre.clone(),
        year: value.year as u16,
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
    }
}

pub fn album_page_to_proto(value: &api::AlbumPage) -> AlbumPage {
    AlbumPage {
        albums: value.albums.iter().map(album_info_to_proto).collect(),
        total: value.total,
    }
}

pub fn album_page_from_proto(value: &AlbumPage) -> api::AlbumPage {
    api::AlbumPage {
        albums: value.albums.iter().map(album_info_from_proto).collect(),
        total: value.total,
    }
}

pub fn artist_info_to_proto(value: &api::ArtistInfo) -> ArtistInfo {
    ArtistInfo {
        name: value.name.clone(),
        track_count: value.track_count,
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
    }
}

pub fn artist_info_from_proto(value: &ArtistInfo) -> api::ArtistInfo {
    api::ArtistInfo {
        name: value.name.clone(),
        track_count: value.track_count,
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
    }
}

pub fn artist_page_to_proto(value: &api::ArtistPage) -> ArtistPage {
    ArtistPage {
        artists: value.artists.iter().map(artist_info_to_proto).collect(),
        total: value.total,
    }
}

pub fn artist_page_from_proto(value: &ArtistPage) -> api::ArtistPage {
    api::ArtistPage {
        artists: value.artists.iter().map(artist_info_from_proto).collect(),
        total: value.total,
    }
}

pub fn search_results_to_proto(value: &api::SearchResults) -> SearchResults {
    SearchResults {
        tracks: value.tracks.iter().map(track_info_to_proto).collect(),
        albums: value.albums.iter().map(album_info_to_proto).collect(),
    }
}

pub fn search_results_from_proto(value: &SearchResults) -> api::SearchResults {
    api::SearchResults {
        tracks: value.tracks.iter().map(track_info_from_proto).collect(),
        albums: value.albums.iter().map(album_info_from_proto).collect(),
    }
}
