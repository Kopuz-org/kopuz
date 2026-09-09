use super::*;
use crate::*;

pub fn catalog_item_kind_to_proto(value: api::CatalogItemKind) -> CatalogItemKind {
    match value {
        api::CatalogItemKind::Track => CatalogItemKind::CatalogItemTrack,
        api::CatalogItemKind::Album => CatalogItemKind::CatalogItemAlbum,
        api::CatalogItemKind::Playlist => CatalogItemKind::CatalogItemPlaylist,
        api::CatalogItemKind::Artist => CatalogItemKind::CatalogItemArtist,
        api::CatalogItemKind::Mood => CatalogItemKind::CatalogItemMood,
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
        Ok(CatalogItemKind::CatalogItemUnknown) | Err(_) => api::CatalogItemKind::Unknown,
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
    }
}

pub fn catalog_shelf_to_proto(value: &api::CatalogShelf) -> CatalogShelf {
    CatalogShelf {
        title: value.title.clone(),
        strapline: value.strapline.clone(),
        items: value.items.iter().map(catalog_item_to_proto).collect(),
        more_ref: value.more_ref.clone(),
        list: value.list,
    }
}

pub fn catalog_shelf_from_proto(value: &CatalogShelf) -> api::CatalogShelf {
    api::CatalogShelf {
        title: value.title.clone(),
        strapline: value.strapline.clone(),
        items: value.items.iter().map(catalog_item_from_proto).collect(),
        more_ref: value.more_ref.clone(),
        list: value.list,
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
            })
            .collect(),
        pinned: value.pinned,
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
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
            })
            .collect(),
        pinned: value.pinned,
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
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
                list: true,
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
                }],
            }],
            continuation: Some("NEXT".into()),
        };
        assert_eq!(page, catalog_page_from_proto(&catalog_page_to_proto(&page)));

        let station = api::RadioStationInfo {
            id: "st-1".into(),
            name: "radio_station_name".into(),
            description: "radio_station_desc".into(),
            tags: vec!["jazz".into()],
            streams: vec![api::RadioStreamInfo {
                id: "hi".into(),
                name: "High".into(),
            }],
            pinned: true,
            artwork: None,
        };
        assert_eq!(
            station,
            radio_station_from_proto(&radio_station_to_proto(&station))
        );
    }
}
