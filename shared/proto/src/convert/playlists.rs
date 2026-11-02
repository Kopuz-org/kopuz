use super::*;
use crate::*;

pub fn playlist_info_to_proto(value: &api::PlaylistInfo) -> PlaylistInfo {
    PlaylistInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        track_keys: value.track_keys.clone(),
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
    }
}

pub fn playlist_info_from_proto(value: &PlaylistInfo) -> api::PlaylistInfo {
    api::PlaylistInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        track_keys: value.track_keys.clone(),
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
    }
}

pub fn playlist_folder_to_proto(value: &api::PlaylistFolderInfo) -> PlaylistFolderInfo {
    PlaylistFolderInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        playlist_ids: value.playlist_ids.clone(),
    }
}

pub fn playlist_folder_from_proto(value: &PlaylistFolderInfo) -> api::PlaylistFolderInfo {
    api::PlaylistFolderInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        playlist_ids: value.playlist_ids.clone(),
    }
}

pub fn playlist_catalog_to_proto(value: &api::PlaylistCatalog) -> PlaylistCatalog {
    PlaylistCatalog {
        playlists: value.playlists.iter().map(playlist_info_to_proto).collect(),
        folders: value.folders.iter().map(playlist_folder_to_proto).collect(),
    }
}

pub fn playlist_catalog_from_proto(value: &PlaylistCatalog) -> api::PlaylistCatalog {
    api::PlaylistCatalog {
        playlists: value
            .playlists
            .iter()
            .map(playlist_info_from_proto)
            .collect(),
        folders: value
            .folders
            .iter()
            .map(playlist_folder_from_proto)
            .collect(),
    }
}
