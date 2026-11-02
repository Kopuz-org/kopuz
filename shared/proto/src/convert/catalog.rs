use super::*;
use crate::*;

pub fn catalog_item_kind_to_proto(value: api::CatalogItemKind) -> CatalogItemKind {
    match value {
        api::CatalogItemKind::Track => CatalogItemKind::CatalogItemTrack,
        api::CatalogItemKind::Album => CatalogItemKind::CatalogItemAlbum,
        api::CatalogItemKind::Playlist => CatalogItemKind::CatalogItemPlaylist,
        api::CatalogItemKind::Artist => CatalogItemKind::CatalogItemArtist,
        api::CatalogItemKind::Mood => CatalogItemKind::CatalogItemMood,
        api::CatalogItemKind::Podcast => CatalogItemKind::CatalogItemPodcast,
        api::CatalogItemKind::Episode => CatalogItemKind::CatalogItemEpisode,
        api::CatalogItemKind::Video => CatalogItemKind::CatalogItemVideo,
        api::CatalogItemKind::Page => CatalogItemKind::CatalogItemPage,
        api::CatalogItemKind::Unknown => CatalogItemKind::CatalogItemUnknown,
    }
}

pub fn catalog_item_kind_from_proto(value: i32) -> api::CatalogItemKind {
    match CatalogItemKind::try_from(value) {
        Ok(CatalogItemKind::CatalogItemTrack) => api::CatalogItemKind::Track,
        Ok(CatalogItemKind::CatalogItemAlbum) => api::CatalogItemKind::Album,
        Ok(CatalogItemKind::CatalogItemPlaylist) => api::CatalogItemKind::Playlist,
        Ok(CatalogItemKind::CatalogItemArtist) => api::CatalogItemKind::Artist,
        Ok(CatalogItemKind::CatalogItemMood) => api::CatalogItemKind::Mood,
        Ok(CatalogItemKind::CatalogItemPodcast) => api::CatalogItemKind::Podcast,
        Ok(CatalogItemKind::CatalogItemEpisode) => api::CatalogItemKind::Episode,
        Ok(CatalogItemKind::CatalogItemVideo) => api::CatalogItemKind::Video,
        Ok(CatalogItemKind::CatalogItemPage) => api::CatalogItemKind::Page,
        Ok(CatalogItemKind::CatalogItemUnknown) | Err(_) => api::CatalogItemKind::Unknown,
    }
}

pub fn shelf_layout_to_proto(value: api::ShelfLayout) -> ShelfLayout {
    match value {
        api::ShelfLayout::Carousel => ShelfLayout::Carousel,
        api::ShelfLayout::Grid => ShelfLayout::Grid,
        api::ShelfLayout::List => ShelfLayout::List,
        api::ShelfLayout::TrackGrid => ShelfLayout::TrackGrid,
        api::ShelfLayout::Hero => ShelfLayout::Hero,
    }
}

pub fn shelf_layout_from_proto(value: i32) -> api::ShelfLayout {
    match ShelfLayout::try_from(value) {
        Ok(ShelfLayout::Grid) => api::ShelfLayout::Grid,
        Ok(ShelfLayout::List) => api::ShelfLayout::List,
        Ok(ShelfLayout::TrackGrid) => api::ShelfLayout::TrackGrid,
        Ok(ShelfLayout::Hero) => api::ShelfLayout::Hero,
        Ok(ShelfLayout::Carousel) | Err(_) => api::ShelfLayout::Carousel,
    }
}

pub fn catalog_header_to_proto(value: api::CatalogHeader) -> CatalogHeader {
    match value {
        api::CatalogHeader::None => CatalogHeader::None,
        api::CatalogHeader::Title => CatalogHeader::Title,
        api::CatalogHeader::Detail => CatalogHeader::Detail,
        api::CatalogHeader::Artist => CatalogHeader::Artist,
    }
}

pub fn catalog_header_from_proto(value: i32) -> api::CatalogHeader {
    match CatalogHeader::try_from(value) {
        Ok(CatalogHeader::Title) => api::CatalogHeader::Title,
        Ok(CatalogHeader::Detail) => api::CatalogHeader::Detail,
        Ok(CatalogHeader::Artist) => api::CatalogHeader::Artist,
        Ok(CatalogHeader::None) | Err(_) => api::CatalogHeader::None,
    }
}

pub fn catalog_chip_to_proto(value: &api::CatalogChip) -> CatalogChip {
    CatalogChip {
        id: value.id.clone(),
        label: value.label.clone(),
        selected: value.selected,
    }
}

pub fn catalog_chip_from_proto(value: &CatalogChip) -> api::CatalogChip {
    api::CatalogChip {
        id: value.id.clone(),
        label: value.label.clone(),
        selected: value.selected,
    }
}

pub fn rating_to_proto(value: api::Rating) -> Rating {
    match value {
        api::Rating::None => Rating::None,
        api::Rating::Like => Rating::Like,
        api::Rating::Dislike => Rating::Dislike,
    }
}

/// `None` for a rating the peer did not set or this build does not know.
pub fn rating_from_proto(value: i32) -> Option<api::Rating> {
    match Rating::try_from(value) {
        Ok(Rating::None) => Some(api::Rating::None),
        Ok(Rating::Like) => Some(api::Rating::Like),
        Ok(Rating::Dislike) => Some(api::Rating::Dislike),
        Ok(Rating::Unspecified) | Err(_) => None,
    }
}

pub fn playlist_privacy_to_proto(value: api::PlaylistPrivacy) -> PlaylistPrivacy {
    match value {
        api::PlaylistPrivacy::Private => PlaylistPrivacy::Private,
        api::PlaylistPrivacy::Unlisted => PlaylistPrivacy::Unlisted,
        api::PlaylistPrivacy::Public => PlaylistPrivacy::Public,
    }
}

pub fn playlist_privacy_from_proto(value: i32) -> Option<api::PlaylistPrivacy> {
    match PlaylistPrivacy::try_from(value) {
        Ok(PlaylistPrivacy::Private) => Some(api::PlaylistPrivacy::Private),
        Ok(PlaylistPrivacy::Unlisted) => Some(api::PlaylistPrivacy::Unlisted),
        Ok(PlaylistPrivacy::Public) => Some(api::PlaylistPrivacy::Public),
        Ok(PlaylistPrivacy::Unspecified) | Err(_) => None,
    }
}

pub fn catalog_actions_to_proto(value: &api::CatalogActions) -> CatalogActions {
    CatalogActions {
        rate_ref: value.rate_ref.clone(),
        rating: value.rating.map(|rating| rating_to_proto(rating) as i32),
        save_ref: value.save_ref.clone(),
        saved: value.saved,
        follow_ref: value.follow_ref.clone(),
        followed: value.followed,
        history_token: value.history_token.clone(),
    }
}

pub fn catalog_actions_from_proto(value: Option<&CatalogActions>) -> api::CatalogActions {
    let Some(value) = value else {
        return api::CatalogActions::default();
    };
    api::CatalogActions {
        rate_ref: value.rate_ref.clone(),
        rating: value.rating.and_then(rating_from_proto),
        save_ref: value.save_ref.clone(),
        saved: value.saved,
        follow_ref: value.follow_ref.clone(),
        followed: value.followed,
        history_token: value.history_token.clone(),
    }
}

pub fn catalog_item_to_proto(value: &api::CatalogItem) -> CatalogItem {
    CatalogItem {
        kind: catalog_item_kind_to_proto(value.kind) as i32,
        id: value.id.clone(),
        title: value.title.clone(),
        subtitle: value.subtitle.clone(),
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
        track: value.track.as_ref().map(track_info_to_proto),
        accent: value.accent.clone(),
        actions: Some(catalog_actions_to_proto(&value.actions)),
        explicit: value.explicit,
        web_url: value.web_url.clone(),
    }
}

pub fn catalog_item_from_proto(value: &CatalogItem) -> api::CatalogItem {
    api::CatalogItem {
        kind: catalog_item_kind_from_proto(value.kind),
        id: value.id.clone(),
        title: value.title.clone(),
        subtitle: value.subtitle.clone(),
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
        track: value.track.as_ref().map(track_info_from_proto),
        accent: value.accent.clone(),
        actions: catalog_actions_from_proto(value.actions.as_ref()),
        explicit: value.explicit,
        web_url: value.web_url.clone(),
    }
}

pub fn catalog_shelf_to_proto(value: &api::CatalogShelf) -> CatalogShelf {
    CatalogShelf {
        title: value.title.clone(),
        strapline: value.strapline.clone(),
        items: value.items.iter().map(catalog_item_to_proto).collect(),
        more_ref: value.more_ref.clone(),
        layout: shelf_layout_to_proto(value.layout) as i32,
        more_kind: catalog_item_kind_to_proto(value.more_kind) as i32,
        continuation: value.continuation.clone(),
        search_filter: value.search_filter.clone(),
    }
}

pub fn catalog_shelf_from_proto(value: &CatalogShelf) -> api::CatalogShelf {
    api::CatalogShelf {
        title: value.title.clone(),
        strapline: value.strapline.clone(),
        items: value.items.iter().map(catalog_item_from_proto).collect(),
        more_ref: value.more_ref.clone(),
        layout: shelf_layout_from_proto(value.layout),
        more_kind: catalog_item_kind_from_proto(value.more_kind),
        continuation: value.continuation.clone(),
        search_filter: value.search_filter.clone(),
    }
}

pub fn catalog_page_to_proto(value: &api::CatalogPage) -> CatalogPage {
    CatalogPage {
        shelves: value.shelves.iter().map(catalog_shelf_to_proto).collect(),
        continuation: value.continuation.clone(),
    }
}

pub fn catalog_page_from_proto(value: &CatalogPage) -> api::CatalogPage {
    api::CatalogPage {
        shelves: value.shelves.iter().map(catalog_shelf_from_proto).collect(),
        continuation: value.continuation.clone(),
    }
}

pub fn catalog_detail_request_to_proto(value: &api::CatalogDetailRequest) -> CatalogDetailRequest {
    CatalogDetailRequest {
        kind: catalog_item_kind_to_proto(value.kind) as i32,
        id: value.id.clone(),
        continuation: value.continuation.clone(),
    }
}

pub fn catalog_detail_request_from_proto(
    value: &CatalogDetailRequest,
) -> api::CatalogDetailRequest {
    api::CatalogDetailRequest {
        kind: catalog_item_kind_from_proto(value.kind),
        id: value.id.clone(),
        continuation: value.continuation.clone(),
    }
}

pub fn catalog_detail_to_proto(value: &api::CatalogDetail) -> CatalogDetail {
    CatalogDetail {
        kind: catalog_item_kind_to_proto(value.kind) as i32,
        id: value.id.clone(),
        title: value.title.clone(),
        subtitle: value.subtitle.clone(),
        description: value.description.clone(),
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
        playback_id: value.playback_id.clone(),
        year: value.year.clone(),
        tracks: value.tracks.iter().map(track_info_to_proto).collect(),
        shelves: value.shelves.iter().map(catalog_shelf_to_proto).collect(),
        continuation: value.continuation.clone(),
        artist_key: value.artist_key.as_ref().map(ToString::to_string),
        header: catalog_header_to_proto(value.header) as i32,
        chips: value.chips.iter().map(catalog_chip_to_proto).collect(),
        actions: Some(catalog_actions_to_proto(&value.actions)),
        privacy: value
            .privacy
            .map(|privacy| playlist_privacy_to_proto(privacy) as i32),
        release_kind: release_kind_to_proto(value.release_kind) as i32,
        monthly_listeners: value.monthly_listeners.clone(),
        shuffle_radio: value.shuffle_radio.clone(),
        mix_radio: value.mix_radio.clone(),
        owner: value.owner.clone(),
        plays: value.plays.clone(),
        web_url: value.web_url.clone(),
    }
}

pub fn release_kind_to_proto(value: Option<api::ReleaseKind>) -> ReleaseKind {
    match value {
        None => ReleaseKind::Unspecified,
        Some(api::ReleaseKind::Album) => ReleaseKind::Album,
        Some(api::ReleaseKind::Single) => ReleaseKind::Single,
        Some(api::ReleaseKind::Ep) => ReleaseKind::Ep,
        Some(api::ReleaseKind::Audiobook) => ReleaseKind::Audiobook,
    }
}

pub fn release_kind_from_proto(value: i32) -> Option<api::ReleaseKind> {
    match ReleaseKind::try_from(value).unwrap_or(ReleaseKind::Unspecified) {
        ReleaseKind::Unspecified => None,
        ReleaseKind::Album => Some(api::ReleaseKind::Album),
        ReleaseKind::Single => Some(api::ReleaseKind::Single),
        ReleaseKind::Ep => Some(api::ReleaseKind::Ep),
        ReleaseKind::Audiobook => Some(api::ReleaseKind::Audiobook),
    }
}

pub fn catalog_detail_from_proto(value: &CatalogDetail) -> api::CatalogDetail {
    api::CatalogDetail {
        kind: catalog_item_kind_from_proto(value.kind),
        id: value.id.clone(),
        title: value.title.clone(),
        subtitle: value.subtitle.clone(),
        description: value.description.clone(),
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
        playback_id: value.playback_id.clone(),
        year: value.year.clone(),
        tracks: value.tracks.iter().map(track_info_from_proto).collect(),
        shelves: value.shelves.iter().map(catalog_shelf_from_proto).collect(),
        continuation: value.continuation.clone(),
        artist_key: value.artist_key.clone(),
        header: catalog_header_from_proto(value.header),
        chips: value.chips.iter().map(catalog_chip_from_proto).collect(),
        actions: catalog_actions_from_proto(value.actions.as_ref()),
        privacy: value.privacy.and_then(playlist_privacy_from_proto),
        release_kind: release_kind_from_proto(value.release_kind),
        monthly_listeners: value.monthly_listeners.clone(),
        shuffle_radio: value.shuffle_radio.clone(),
        mix_radio: value.mix_radio.clone(),
        owner: value.owner.clone(),
        plays: value.plays.clone(),
        web_url: value.web_url.clone(),
    }
}

pub fn for_you_to_proto(value: &api::ForYou) -> ForYou {
    let outcome = match value {
        api::ForYou::Mix(tracks) => for_you::Outcome::Mix(TrackList {
            items: tracks.iter().map(track_info_to_proto).collect(),
        }),
        api::ForYou::Unavailable(reason) => {
            let reason = match reason {
                api::ForYouUnavailable::NoTrackRadio => ForYouUnavailable::NoTrackRadio,
                api::ForYouUnavailable::NoLikes => ForYouUnavailable::NoLikes,
            };
            for_you::Outcome::Unavailable(reason as i32)
        }
    };
    ForYou {
        outcome: Some(outcome),
    }
}

pub fn for_you_from_proto(value: &ForYou) -> api::ForYou {
    match &value.outcome {
        Some(for_you::Outcome::Mix(list)) => {
            api::ForYou::Mix(list.items.iter().map(track_info_from_proto).collect())
        }
        Some(for_you::Outcome::Unavailable(reason)) => {
            api::ForYou::Unavailable(match ForYouUnavailable::try_from(*reason) {
                Ok(ForYouUnavailable::NoTrackRadio) => api::ForYouUnavailable::NoTrackRadio,
                // A reason this build does not know still means there is no row to show.
                Ok(ForYouUnavailable::NoLikes | ForYouUnavailable::Unspecified) | Err(_) => {
                    api::ForYouUnavailable::NoLikes
                }
            })
        }
        None => api::ForYou::Mix(Vec::new()),
    }
}

pub fn discover_rows_to_proto(value: &api::DiscoverRows) -> DiscoverRows {
    DiscoverRows {
        mixes: value.mixes.iter().map(catalog_item_to_proto).collect(),
        never_played: value.never_played.iter().map(album_info_to_proto).collect(),
        forgotten_favorites: value
            .forgotten_favorites
            .iter()
            .map(track_info_to_proto)
            .collect(),
    }
}

pub fn discover_rows_from_proto(value: &DiscoverRows) -> api::DiscoverRows {
    api::DiscoverRows {
        mixes: value.mixes.iter().map(catalog_item_from_proto).collect(),
        never_played: value
            .never_played
            .iter()
            .map(album_info_from_proto)
            .collect(),
        forgotten_favorites: value
            .forgotten_favorites
            .iter()
            .map(track_info_from_proto)
            .collect(),
    }
}

pub fn radio_station_to_proto(value: &api::RadioStationInfo) -> RadioStationInfo {
    RadioStationInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        description: value.description.clone(),
        tags: value.tags.clone(),
        streams: value
            .streams
            .iter()
            .map(|stream| RadioStreamInfo {
                id: stream.id.clone(),
                name: stream.name.clone(),
                icon: stream.icon.clone(),
            })
            .collect(),
        pinned: value.pinned,
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
        icon: value.icon.clone(),
    }
}

pub fn radio_station_from_proto(value: &RadioStationInfo) -> api::RadioStationInfo {
    api::RadioStationInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        description: value.description.clone(),
        tags: value.tags.clone(),
        streams: value
            .streams
            .iter()
            .map(|stream| api::RadioStreamInfo {
                id: stream.id.clone(),
                name: stream.name.clone(),
                icon: stream.icon.clone(),
            })
            .collect(),
        pinned: value.pinned,
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
        icon: value.icon.clone(),
    }
}

pub fn artwork_change_to_proto(value: &api::ArtworkChange) -> Option<ArtworkChange> {
    let change = match value {
        api::ArtworkChange::Keep => return None,
        api::ArtworkChange::Remove => artwork_change::Change::Remove(Unit {}),
        api::ArtworkChange::Set(bytes) => artwork_change::Change::Set(bytes.clone()),
    };
    Some(ArtworkChange {
        change: Some(change),
    })
}

pub fn artwork_change_from_proto(value: Option<&ArtworkChange>) -> api::ArtworkChange {
    match value.and_then(|value| value.change.as_ref()) {
        None => api::ArtworkChange::Keep,
        Some(artwork_change::Change::Remove(_)) => api::ArtworkChange::Remove,
        Some(artwork_change::Change::Set(bytes)) => api::ArtworkChange::Set(bytes.clone()),
    }
}

pub fn track_patch_to_proto(value: &api::TrackMetadataPatch) -> TrackMetadataPatch {
    TrackMetadataPatch {
        key: value.key.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        track_number: value.track_number,
        clear_track_number: value.clear_track_number,
        disc_number: value.disc_number,
        clear_disc_number: value.clear_disc_number,
        cover: artwork_change_to_proto(&value.cover),
    }
}

pub fn track_patch_from_proto(value: &TrackMetadataPatch) -> api::TrackMetadataPatch {
    api::TrackMetadataPatch {
        key: value.key.clone(),
        title: value.title.clone(),
        artist: value.artist.clone(),
        album: value.album.clone(),
        track_number: value.track_number,
        clear_track_number: value.clear_track_number,
        disc_number: value.disc_number,
        clear_disc_number: value.clear_disc_number,
        cover: artwork_change_from_proto(value.cover.as_ref()),
    }
}

pub fn artwork_upload_to_proto(value: &api::ArtworkUpload) -> ArtworkUpload {
    ArtworkUpload {
        target: Some(artwork_target_to_proto(&value.target)),
        content_type: value.content_type.clone(),
        bytes: value.bytes.clone(),
    }
}

pub fn artwork_upload_from_proto(value: &ArtworkUpload) -> Option<api::ArtworkUpload> {
    Some(api::ArtworkUpload {
        target: artwork_target_from_proto(value.target.as_ref()?)?,
        content_type: value.content_type.clone(),
        bytes: value.bytes.clone(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_and_radio_rows_round_trip() {
        let page = api::CatalogPage {
            shelves: vec![api::CatalogShelf {
                title: "Listen again".into(),
                strapline: Some("for you".into()),
                more_ref: Some("MORE".into()),
                layout: api::ShelfLayout::List,
                items: vec![api::CatalogItem {
                    kind: api::CatalogItemKind::Album,
                    id: "MPRE1".into(),
                    title: "An album".into(),
                    subtitle: Some("An artist".into()),
                    artwork: Some(api::ArtworkRef {
                        target: api::ArtworkTarget::Catalog("MPRE1".into()),
                        version: 77,
                    }),
                    track: None,
                    accent: None,
                    actions: api::CatalogActions {
                        rate_ref: Some("playlist:OLAK1".into()),
                        rating: Some(api::Rating::Dislike),
                        save_ref: Some("playlist:OLAK1".into()),
                        saved: Some(true),
                        follow_ref: None,
                        followed: None,
                        history_token: Some("HISTORY".into()),
                    },
                    explicit: true,
                    web_url: Some("https://example.com/MPRE1".into()),
                }],
                ..Default::default()
            }],
            continuation: Some("NEXT".into()),
        };
        assert_eq!(page, catalog_page_from_proto(&catalog_page_to_proto(&page)));

        let detail = api::CatalogDetail {
            kind: api::CatalogItemKind::Page,
            id: "FEmusic_moods".into(),
            title: "Moods".into(),
            actions: api::CatalogActions {
                follow_ref: Some("UCabc".into()),
                followed: Some(false),
                rating: Some(api::Rating::None),
                ..Default::default()
            },
            privacy: Some(api::PlaylistPrivacy::Unlisted),
            header: api::CatalogHeader::Title,
            chips: vec![api::CatalogChip {
                id: "FEmusic_home?p".into(),
                label: "Relax".into(),
                selected: true,
            }],
            shelves: vec![api::CatalogShelf {
                title: "Genres".into(),
                layout: api::ShelfLayout::Grid,
                more_ref: Some("FEmusic_moods_and_genres".into()),
                more_kind: api::CatalogItemKind::Page,
                continuation: Some("MORE".into()),
                items: [
                    api::CatalogItemKind::Podcast,
                    api::CatalogItemKind::Episode,
                    api::CatalogItemKind::Video,
                    api::CatalogItemKind::Page,
                ]
                .into_iter()
                .map(|kind| api::CatalogItem {
                    kind,
                    id: format!("{kind:?}"),
                    accent: Some("#ff8800".into()),
                    ..Default::default()
                })
                .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            detail,
            catalog_detail_from_proto(&catalog_detail_to_proto(&detail))
        );

        let album = api::CatalogDetail {
            kind: api::CatalogItemKind::Album,
            id: "MPRE1".into(),
            title: "An album".into(),
            description: Some("About it".into()),
            release_kind: Some(api::ReleaseKind::Ep),
            monthly_listeners: Some("1M monthly audience".into()),
            owner: Some("Someone".into()),
            plays: Some("2M views".into()),
            web_url: Some("https://example.com/MPRE1".into()),
            header: api::CatalogHeader::Detail,
            ..Default::default()
        };
        assert_eq!(
            album,
            catalog_detail_from_proto(&catalog_detail_to_proto(&album))
        );
        for layout in [
            api::ShelfLayout::Carousel,
            api::ShelfLayout::Grid,
            api::ShelfLayout::List,
            api::ShelfLayout::TrackGrid,
            api::ShelfLayout::Hero,
        ] {
            assert_eq!(
                layout,
                shelf_layout_from_proto(shelf_layout_to_proto(layout) as i32)
            );
        }
        for header in [
            api::CatalogHeader::None,
            api::CatalogHeader::Title,
            api::CatalogHeader::Detail,
            api::CatalogHeader::Artist,
        ] {
            assert_eq!(
                header,
                catalog_header_from_proto(catalog_header_to_proto(header) as i32)
            );
        }

        let station = api::RadioStationInfo {
            id: "st-1".into(),
            name: "radio_station_name".into(),
            description: "radio_station_desc".into(),
            tags: vec!["jazz".into()],
            streams: vec![api::RadioStreamInfo {
                id: "hi".into(),
                name: "High".into(),
                icon: Some("fa-solid fa-tower-broadcast".into()),
            }],
            pinned: true,
            artwork: None,
            icon: "fa-solid fa-radio".into(),
        };
        assert_eq!(
            station,
            radio_station_from_proto(&radio_station_to_proto(&station))
        );
    }

    #[test]
    fn for_you_round_trips() {
        let track = super::super::fixtures::sample_state()
            .track
            .expect("the sample state plays a track");
        for value in [
            api::ForYou::Mix(vec![track.clone(), track]),
            api::ForYou::Mix(Vec::new()),
            api::ForYou::Unavailable(api::ForYouUnavailable::NoTrackRadio),
            api::ForYou::Unavailable(api::ForYouUnavailable::NoLikes),
        ] {
            assert_eq!(value, for_you_from_proto(&for_you_to_proto(&value)));
        }
    }

    #[test]
    fn discover_rows_round_trip() {
        let track = super::super::fixtures::sample_state()
            .track
            .expect("the sample state plays a track");
        let rows = api::DiscoverRows {
            mixes: vec![api::CatalogItem {
                kind: api::CatalogItemKind::Playlist,
                id: "RDTMAK5uy_mix".into(),
                title: "My Supermix".into(),
                subtitle: Some("Ada, Boris".into()),
                ..Default::default()
            }],
            never_played: vec![api::AlbumInfo {
                id: "al-1".into(),
                title: "Unheard".into(),
                artist: "Ada".into(),
                year: 2001,
                ..Default::default()
            }],
            forgotten_favorites: vec![track],
        };
        for value in [rows, api::DiscoverRows::default()] {
            assert_eq!(
                value,
                discover_rows_from_proto(&discover_rows_to_proto(&value))
            );
        }
    }
}
