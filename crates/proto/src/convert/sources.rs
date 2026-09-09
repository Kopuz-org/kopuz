use super::*;
use crate::*;

pub fn source_kind_to_proto(value: api::SourceKind) -> SourceKind {
    match value {
        api::SourceKind::Local => SourceKind::Local,
        api::SourceKind::LocalLibrary => SourceKind::LocalLibrary,
        api::SourceKind::Server => SourceKind::Server,
        api::SourceKind::Unknown => SourceKind::Unknown,
    }
}

pub fn source_kind_from_proto(value: i32) -> api::SourceKind {
    match SourceKind::try_from(value) {
        Ok(SourceKind::Local) => api::SourceKind::Local,
        Ok(SourceKind::LocalLibrary) => api::SourceKind::LocalLibrary,
        Ok(SourceKind::Server) => api::SourceKind::Server,
        Ok(SourceKind::Unknown) | Err(_) => api::SourceKind::Unknown,
    }
}

pub fn capabilities_to_proto(value: &api::SourceCapabilities) -> SourceCapabilities {
    use api::{AlbumPresentation, ArtistPresentation, FavoritesSyncMode, PlaylistCapability};
    SourceCapabilities {
        edit_tags: value.edit_tags,
        delete_from_disk: value.delete_from_disk,
        scan_folders: value.scan_folders,
        folders: value.folders,
        sync: value.sync,
        downloads: value.downloads,
        discover: value.discover,
        track_radio: value.track_radio,
        playlist_radio: value.playlist_radio,
        playlists: match value.playlists {
            PlaylistCapability::None => crate::PlaylistCapability::None,
            PlaylistCapability::AddRemove => crate::PlaylistCapability::AddRemove,
            PlaylistCapability::Reorder => crate::PlaylistCapability::Reorder,
        } as i32,
        artists: match value.artists {
            ArtistPresentation::Library => crate::ArtistPresentation::Library,
            ArtistPresentation::Remote => crate::ArtistPresentation::Remote,
        } as i32,
        albums: match value.albums {
            AlbumPresentation::Standard => crate::AlbumPresentation::Standard,
            AlbumPresentation::Remote => crate::AlbumPresentation::Remote,
        } as i32,
        favorites_sync: match value.favorites_sync {
            FavoritesSyncMode::Instant => crate::FavoritesSyncMode::FavoritesSyncInstant,
            FavoritesSyncMode::Paginated => crate::FavoritesSyncMode::FavoritesSyncPaginated,
        } as i32,
    }
}

pub fn capabilities_from_proto(value: Option<&SourceCapabilities>) -> api::SourceCapabilities {
    let Some(value) = value else {
        return api::SourceCapabilities::default();
    };
    api::SourceCapabilities {
        edit_tags: value.edit_tags,
        delete_from_disk: value.delete_from_disk,
        scan_folders: value.scan_folders,
        folders: value.folders,
        sync: value.sync,
        downloads: value.downloads,
        discover: value.discover,
        track_radio: value.track_radio,
        playlist_radio: value.playlist_radio,
        playlists: match crate::PlaylistCapability::try_from(value.playlists) {
            Ok(crate::PlaylistCapability::AddRemove) => api::PlaylistCapability::AddRemove,
            Ok(crate::PlaylistCapability::Reorder) => api::PlaylistCapability::Reorder,
            _ => api::PlaylistCapability::None,
        },
        artists: match crate::ArtistPresentation::try_from(value.artists) {
            Ok(crate::ArtistPresentation::Remote) => api::ArtistPresentation::Remote,
            _ => api::ArtistPresentation::Library,
        },
        albums: match crate::AlbumPresentation::try_from(value.albums) {
            Ok(crate::AlbumPresentation::Remote) => api::AlbumPresentation::Remote,
            _ => api::AlbumPresentation::Standard,
        },
        favorites_sync: match crate::FavoritesSyncMode::try_from(value.favorites_sync) {
            Ok(crate::FavoritesSyncMode::FavoritesSyncPaginated) => {
                api::FavoritesSyncMode::Paginated
            }
            _ => api::FavoritesSyncMode::Instant,
        },
    }
}

pub fn source_info_to_proto(value: &api::SourceInfo) -> SourceInfo {
    SourceInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        kind: source_kind_to_proto(value.kind) as i32,
        service: value
            .service
            .map(|service| music_service_to_proto(service) as i32),
        active: value.active,
        authenticated: value.authenticated,
        capabilities: Some(capabilities_to_proto(&value.capabilities)),
        url: value.url.clone(),
        browser: value.browser.clone(),
        anonymous: value.anonymous,
        storefront: value.storefront.clone(),
        language: value.language.clone(),
        directories: value.directories.clone(),
    }
}

pub fn source_info_from_proto(value: &SourceInfo) -> api::SourceInfo {
    api::SourceInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        kind: source_kind_from_proto(value.kind),
        service: value.service.and_then(music_service_from_proto),
        active: value.active,
        authenticated: value.authenticated,
        capabilities: capabilities_from_proto(value.capabilities.as_ref()),
        url: value.url.clone(),
        browser: value.browser.clone(),
        anonymous: value.anonymous,
        storefront: value.storefront.clone(),
        language: value.language.clone(),
        directories: value.directories.clone(),
    }
}

pub fn local_draft_to_proto(value: &api::LocalSourceDraft) -> LocalSourceDraft {
    LocalSourceDraft {
        id: value.id.clone(),
        name: value.name.clone(),
        directories: value.directories.clone(),
    }
}

pub fn local_draft_from_proto(value: &LocalSourceDraft) -> api::LocalSourceDraft {
    api::LocalSourceDraft {
        id: value.id.clone(),
        name: value.name.clone(),
        directories: value.directories.clone(),
    }
}

pub fn server_draft_to_proto(value: &api::ServerDraft) -> ServerDraft {
    ServerDraft {
        id: value.id.clone(),
        name: value.name.clone(),
        url: value.url.clone(),
        service: music_service_to_proto(value.service) as i32,
        browser: value.browser.clone(),
        anonymous: value.anonymous,
        storefront: value.storefront.clone(),
        language: value.language.clone(),
    }
}

pub fn server_draft_from_proto(value: &ServerDraft) -> api::ServerDraft {
    api::ServerDraft {
        id: value.id.clone(),
        name: value.name.clone(),
        url: value.url.clone(),
        service: music_service_from_proto(value.service).unwrap_or_default(),
        browser: value.browser.clone(),
        anonymous: value.anonymous,
        storefront: value.storefront.clone(),
        language: value.language.clone(),
    }
}

pub fn credential_provision_to_proto(value: &api::CredentialProvision) -> CredentialProvision {
    CredentialProvision {
        server_id: value.server_id.clone(),
        secret: value.secret.clone(),
        user_id: value.user_id.clone(),
        browser: value.browser.clone(),
    }
}

pub fn credential_provision_from_proto(value: &CredentialProvision) -> api::CredentialProvision {
    api::CredentialProvision {
        server_id: value.server_id.clone(),
        secret: value.secret.clone(),
        user_id: value.user_id.clone(),
        browser: value.browser.clone(),
    }
}

pub fn integration_kind_to_proto(value: api::IntegrationKind) -> IntegrationKind {
    match value {
        api::IntegrationKind::ListenBrainz => IntegrationKind::IntegrationListenbrainz,
        api::IntegrationKind::LastFm => IntegrationKind::IntegrationLastfm,
        api::IntegrationKind::LibreFm => IntegrationKind::IntegrationLibrefm,
        api::IntegrationKind::Unknown => IntegrationKind::IntegrationUnknown,
    }
}

pub fn integration_kind_from_proto(value: i32) -> api::IntegrationKind {
    match IntegrationKind::try_from(value) {
        Ok(IntegrationKind::IntegrationListenbrainz) => api::IntegrationKind::ListenBrainz,
        Ok(IntegrationKind::IntegrationLastfm) => api::IntegrationKind::LastFm,
        Ok(IntegrationKind::IntegrationLibrefm) => api::IntegrationKind::LibreFm,
        Ok(IntegrationKind::IntegrationUnknown) | Err(_) => api::IntegrationKind::Unknown,
    }
}

pub fn integration_status_to_proto(value: &api::IntegrationStatus) -> IntegrationStatus {
    IntegrationStatus {
        kind: integration_kind_to_proto(value.kind) as i32,
        configured: value.configured,
    }
}

pub fn integration_status_from_proto(value: &IntegrationStatus) -> api::IntegrationStatus {
    api::IntegrationStatus {
        kind: integration_kind_from_proto(value.kind),
        configured: value.configured,
    }
}

pub fn integration_provision_to_proto(value: &api::IntegrationProvision) -> IntegrationProvision {
    IntegrationProvision {
        kind: integration_kind_to_proto(value.kind) as i32,
        token: value.token.clone(),
        api_key: value.api_key.clone(),
        api_secret: value.api_secret.clone(),
        session_key: value.session_key.clone(),
    }
}

pub fn integration_provision_from_proto(value: &IntegrationProvision) -> api::IntegrationProvision {
    api::IntegrationProvision {
        kind: integration_kind_from_proto(value.kind),
        token: value.token.clone(),
        api_key: value.api_key.clone(),
        api_secret: value.api_secret.clone(),
        session_key: value.session_key.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source row is what a settings page renders, so every field of it has
    /// to survive the wire -- and none of them may be a credential.
    #[test]
    fn a_source_row_round_trips_without_carrying_a_secret() {
        let info = api::SourceInfo {
            id: "jellyfin-1".into(),
            name: "Home".into(),
            kind: api::SourceKind::Server,
            service: Some(::config::MusicService::Jellyfin),
            active: true,
            authenticated: true,
            capabilities: api::SourceCapabilities {
                edit_tags: false,
                delete_from_disk: false,
                sync: true,
                downloads: true,
                playlists: api::PlaylistCapability::Reorder,
                artists: api::ArtistPresentation::Library,
                albums: api::AlbumPresentation::Standard,
                favorites_sync: api::FavoritesSyncMode::Paginated,
                ..Default::default()
            },
            url: Some("https://jelly.example".into()),
            browser: None,
            anonymous: false,
            storefront: Some("us".into()),
            language: Some("en".into()),
            directories: vec!["/Music".into()],
        };
        assert_eq!(info, source_info_from_proto(&source_info_to_proto(&info)));
    }
}
