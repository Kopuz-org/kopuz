use super::*;
use crate::*;

pub fn track_filter_to_proto(value: &api::TrackFilter) -> TrackFilter {
    TrackFilter {
        search: value.search.clone(),
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
        format: value.format.clone(),
        musicbrainz_release_id: value.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: value.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: value.musicbrainz_track_id.clone(),
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
        credits: value.credits.iter().map(artist_credit_to_proto).collect(),
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
        format: value.format.clone(),
        musicbrainz_release_id: value.musicbrainz_release_id.clone(),
        musicbrainz_recording_id: value.musicbrainz_recording_id.clone(),
        musicbrainz_track_id: value.musicbrainz_track_id.clone(),
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
        credits: value.credits.iter().map(artist_credit_from_proto).collect(),
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
        api::ArtworkTarget::Artist(artist) => Entity::ArtistKey(artist.to_string()),
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
        Entity::ArtistKey(key) => api::ArtworkTarget::Artist(key.clone()),
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
        api::ArtworkTarget::Artist(artist) => Entity::ArtistKey(artist.to_string()),
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
        Entity::ArtistKey(key) => api::ArtworkTarget::Artist(key.clone()),
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

pub fn album_info_to_proto(value: &api::AlbumInfo) -> AlbumInfo {
    AlbumInfo {
        id: value.id.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        genre: value.genre.clone(),
        year: value.year as u32,
        artist_key: value.artist_key.as_ref().map(ToString::to_string),
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
        artist_key: value.artist_key.clone(),
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
        key: value.key.to_string(),
        name: value.name.clone(),
        track_count: value.track_count,
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
    }
}

pub fn artist_info_from_proto(value: &ArtistInfo) -> api::ArtistInfo {
    api::ArtistInfo {
        key: value.key.clone(),
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

pub fn search_request_to_proto(value: &api::SearchRequest) -> SearchRequest {
    SearchRequest {
        query: value.query.clone(),
        filter: value.filter.clone(),
        continuation: value.continuation.clone(),
    }
}

pub fn search_request_from_proto(value: &SearchRequest) -> api::SearchRequest {
    api::SearchRequest {
        query: value.query.clone(),
        filter: value.filter.clone(),
        continuation: value.continuation.clone(),
    }
}

pub fn search_results_to_proto(value: &api::SearchResults) -> SearchResults {
    SearchResults {
        tracks: value.tracks.iter().map(track_info_to_proto).collect(),
        albums: value.albums.iter().map(album_info_to_proto).collect(),
        shelves: value.shelves.iter().map(catalog_shelf_to_proto).collect(),
        continuation: value.continuation.clone(),
        correction: value.correction.clone(),
    }
}

pub fn search_results_from_proto(value: &SearchResults) -> api::SearchResults {
    api::SearchResults {
        tracks: value.tracks.iter().map(track_info_from_proto).collect(),
        albums: value.albums.iter().map(album_info_from_proto).collect(),
        shelves: value.shelves.iter().map(catalog_shelf_from_proto).collect(),
        continuation: value.continuation.clone(),
        correction: value.correction.clone(),
    }
}

pub fn search_suggestion_to_proto(value: &api::SearchSuggestion) -> SearchSuggestion {
    SearchSuggestion {
        text: value.text.clone(),
        from_history: value.from_history,
        item: value.item.as_ref().map(catalog_item_to_proto),
    }
}

pub fn search_suggestion_from_proto(value: &SearchSuggestion) -> api::SearchSuggestion {
    api::SearchSuggestion {
        text: value.text.clone(),
        from_history: value.from_history,
        item: value.item.as_ref().map(catalog_item_from_proto),
    }
}

pub fn artist_credit_to_proto(value: &api::ArtistCredit) -> ArtistCredit {
    ArtistCredit {
        name: value.name.clone(),
        key: value.key.as_ref().map(ToString::to_string),
    }
}

pub fn artist_credit_from_proto(value: &ArtistCredit) -> api::ArtistCredit {
    api::ArtistCredit {
        name: value.name.clone(),
        key: value.key.clone(),
    }
}

pub fn refresh_artists_to_proto(value: &[String]) -> RefreshArtistArtworkRequest {
    RefreshArtistArtworkRequest {
        keys: value.to_vec(),
    }
}

pub fn refresh_artists_from_proto(value: &RefreshArtistArtworkRequest) -> Vec<String> {
    value.keys.clone()
}

pub fn artist_tracks_request_to_proto(artist: &str, page: api::Page) -> ArtistTracksRequest {
    ArtistTracksRequest {
        key: artist.to_string(),
        page: Some(page_to_proto(page)),
    }
}

pub fn artist_detail_to_proto(value: &api::ArtistDetail) -> ArtistDetail {
    ArtistDetail {
        info: Some(artist_info_to_proto(&value.info)),
        albums: value.albums.iter().map(album_info_to_proto).collect(),
    }
}

/// A detail without its artist is malformed, and says so rather than naming nobody.
pub fn artist_detail_from_proto(value: &ArtistDetail) -> Option<api::ArtistDetail> {
    Some(api::ArtistDetail {
        info: artist_info_from_proto(value.info.as_ref()?),
        albums: value.albums.iter().map(album_info_from_proto).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A filtered search carries its filter and continuation out, and its
    /// shelves and correction back, on top of the plain search's rows.
    #[test]
    fn a_search_and_its_suggestions_round_trip() {
        let request = api::SearchRequest {
            query: "daft punk".into(),
            filter: Some("songs".into()),
            continuation: Some("search||4qmF".into()),
        };
        assert_eq!(
            request,
            search_request_from_proto(&search_request_to_proto(&request))
        );
        assert_eq!(
            api::SearchRequest::new("q"),
            search_request_from_proto(&search_request_to_proto(&api::SearchRequest::new("q")))
        );

        let results = api::SearchResults {
            shelves: vec![api::CatalogShelf {
                title: "Songs".into(),
                layout: api::ShelfLayout::List,
                search_filter: Some("songs".into()),
                ..Default::default()
            }],
            continuation: Some("next".into()),
            correction: Some("daft punk".into()),
            ..Default::default()
        };
        assert_eq!(
            results,
            search_results_from_proto(&search_results_to_proto(&results))
        );

        for suggestion in [
            api::SearchSuggestion {
                text: "daft punk".into(),
                from_history: true,
                item: None,
            },
            api::SearchSuggestion {
                text: "Daft Punk".into(),
                from_history: false,
                item: Some(api::CatalogItem {
                    kind: api::CatalogItemKind::Artist,
                    id: "UC".into(),
                    title: "Daft Punk".into(),
                    ..Default::default()
                }),
            },
        ] {
            assert_eq!(
                suggestion,
                search_suggestion_from_proto(&search_suggestion_to_proto(&suggestion))
            );
        }
    }

    /// A track row is what every listing renders, down to the file details a
    /// row shows, so all of it has to survive the wire.
    #[test]
    fn a_track_row_round_trips() {
        let track = api::TrackInfo {
            key: "k".into(),
            uid: "local:k".into(),
            title: "t".into(),
            artist: "a".into(),
            album: "al".into(),
            album_id: "al-1".into(),
            duration_ms: Some(223_000),
            khz: 44,
            bitrate: 320,
            track_number: Some(3),
            disc_number: Some(1),
            kind: api::TrackKind::Normal,
            seekable: true,
            offline: false,
            format: Some("FLAC".into()),
            musicbrainz_release_id: Some("mbr".into()),
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            artwork: Some(api::ArtworkRef {
                target: api::ArtworkTarget::Track("k".into()),
                version: 9,
            }),
            credits: vec![
                api::ArtistCredit {
                    name: "a".into(),
                    key: Some(String::from("UC-a")),
                },
                api::ArtistCredit {
                    name: "b".into(),
                    key: None,
                },
            ],
        };
        assert_eq!(track, track_info_from_proto(&track_info_to_proto(&track)));
    }

    #[test]
    fn every_artist_reference_carries_its_key_across() {
        let key = String::from("ar-1");
        let target = api::ArtworkTarget::Artist(key.clone());
        assert_eq!(
            artwork_target_from_proto(&artwork_target_to_proto(&target)),
            Some(target)
        );
        let keys = [key.clone(), String::from("5f0c1e2d9a8b4c3d")];
        assert_eq!(
            refresh_artists_from_proto(&refresh_artists_to_proto(&keys)),
            keys
        );

        let detail = api::ArtistDetail {
            info: api::ArtistInfo {
                key: key.clone(),
                name: "Ada".into(),
                track_count: 2,
                artwork: None,
            },
            albums: vec![api::AlbumInfo {
                id: "al".into(),
                artist_key: Some(key),
                ..Default::default()
            }],
        };
        assert_eq!(
            artist_detail_from_proto(&artist_detail_to_proto(&detail)),
            Some(detail)
        );
    }

    #[test]
    fn a_detail_naming_no_artist_is_malformed() {
        let sent = ArtistDetail {
            info: None,
            albums: Vec::new(),
        };
        assert_eq!(artist_detail_from_proto(&sent), None);
    }
}
