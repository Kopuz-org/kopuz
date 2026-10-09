use super::*;
use crate::*;

pub fn playlist_info_to_proto(value: &api::PlaylistInfo) -> PlaylistInfo {
    PlaylistInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        track_keys: value.track_keys.clone(),
        artwork: value.artwork.as_ref().map(artwork_ref_to_proto),
        capability: value
            .capability
            .map(|capability| playlist_capability_to_proto(capability) as i32),
    }
}

pub fn playlist_info_from_proto(value: &PlaylistInfo) -> api::PlaylistInfo {
    api::PlaylistInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        track_keys: value.track_keys.clone(),
        artwork: value.artwork.as_ref().and_then(artwork_ref_from_proto),
        capability: value.capability.map(playlist_capability_from_proto),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An unset capability is "the source decides", which a default of
    /// `None` on the wire would turn into "takes no edits".
    #[test]
    fn a_playlist_capability_survives_the_wire_unset_included() {
        for capability in [
            None,
            Some(api::PlaylistCapability::None),
            Some(api::PlaylistCapability::Reorder),
        ] {
            let info = api::PlaylistInfo {
                id: "PLx".into(),
                capability,
                ..Default::default()
            };
            assert_eq!(
                playlist_info_from_proto(&playlist_info_to_proto(&info)),
                info
            );
        }
    }
}
