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
        browse_folders: value.browse_folders,
        external_devices: value.external_devices,
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
        browse_folders: value.browse_folders,
        external_devices: value.external_devices,
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

pub fn sign_in_kind_to_proto(value: api::SignInKind) -> SignInKind {
    match value {
        api::SignInKind::None => SignInKind::None,
        api::SignInKind::Password => SignInKind::Password,
        api::SignInKind::Browser => SignInKind::Browser,
    }
}

pub fn sign_in_kind_from_proto(value: i32) -> api::SignInKind {
    match SignInKind::try_from(value) {
        Ok(SignInKind::Password) => api::SignInKind::Password,
        Ok(SignInKind::Browser) => api::SignInKind::Browser,
        Ok(SignInKind::None) | Ok(SignInKind::Unspecified) | Err(_) => api::SignInKind::None,
    }
}

pub fn service_info_to_proto(value: &api::ServiceInfo) -> ServiceInfo {
    ServiceInfo {
        id: value.id.clone(),
        name: Some(text_to_proto(&value.name)),
        icon: Some(icon_to_proto(&value.icon)),
        accent: value.accent.clone(),
        experimental: value.experimental,
        fields: value.fields.iter().map(field_spec_to_proto).collect(),
    }
}

pub fn service_info_from_proto(value: &ServiceInfo) -> api::ServiceInfo {
    api::ServiceInfo {
        id: value.id.clone(),
        name: value.name.as_ref().map(text_from_proto).unwrap_or_default(),
        icon: value.icon.as_ref().map(icon_from_proto).unwrap_or_default(),
        accent: value.accent.clone(),
        experimental: value.experimental,
        fields: value.fields.iter().map(field_spec_from_proto).collect(),
    }
}

pub fn service_ref_to_proto(value: &api::ServiceRef) -> ServiceRef {
    ServiceRef {
        id: value.id.clone(),
        name: Some(text_to_proto(&value.name)),
        icon: Some(icon_to_proto(&value.icon)),
        accent: value.accent.clone(),
    }
}

pub fn service_ref_from_proto(value: &ServiceRef) -> api::ServiceRef {
    api::ServiceRef {
        id: value.id.clone(),
        name: value.name.as_ref().map(text_from_proto).unwrap_or_default(),
        icon: value.icon.as_ref().map(icon_from_proto).unwrap_or_default(),
        accent: value.accent.clone(),
    }
}

pub fn draft_check_to_proto(value: &api::DraftCheck) -> DraftCheck {
    DraftCheck {
        sign_in: sign_in_kind_to_proto(value.sign_in) as i32,
        problems: value.problems.iter().map(problem_to_proto).collect(),
    }
}

pub fn draft_check_from_proto(value: &DraftCheck) -> api::DraftCheck {
    api::DraftCheck {
        sign_in: sign_in_kind_from_proto(value.sign_in),
        problems: value.problems.iter().map(problem_from_proto).collect(),
    }
}

pub fn source_info_to_proto(value: &api::SourceInfo) -> SourceInfo {
    SourceInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        kind: source_kind_to_proto(value.kind) as i32,
        service: value.service.as_ref().map(service_ref_to_proto),
        active: value.active,
        authenticated: value.authenticated,
        sign_in: sign_in_kind_to_proto(value.sign_in) as i32,
        capabilities: Some(capabilities_to_proto(&value.capabilities)),
        detail: value.detail.clone(),
        anonymous: value.anonymous,
        settings: value.settings.iter().map(field_spec_to_proto).collect(),
        directories: value.directories.clone(),
    }
}

pub fn source_info_from_proto(value: &SourceInfo) -> api::SourceInfo {
    api::SourceInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        kind: source_kind_from_proto(value.kind),
        service: value.service.as_ref().map(service_ref_from_proto),
        active: value.active,
        authenticated: value.authenticated,
        sign_in: sign_in_kind_from_proto(value.sign_in),
        capabilities: capabilities_from_proto(value.capabilities.as_ref()),
        detail: value.detail.clone(),
        anonymous: value.anonymous,
        settings: value.settings.iter().map(field_spec_from_proto).collect(),
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
        service: value.service.clone(),
        values: value.values.iter().map(field_value_to_proto).collect(),
        secrets: value.secrets.iter().map(field_value_to_proto).collect(),
    }
}

pub fn server_draft_from_proto(value: &ServerDraft) -> api::ServerDraft {
    api::ServerDraft {
        id: value.id.clone(),
        name: value.name.clone(),
        service: value.service.clone(),
        values: value.values.iter().map(field_value_from_proto).collect(),
        secrets: value.secrets.iter().map(field_value_from_proto).collect(),
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

pub fn connect_kind_to_proto(value: api::ConnectKind) -> ConnectKind {
    match value {
        api::ConnectKind::None => ConnectKind::None,
        api::ConnectKind::WebSignIn => ConnectKind::WebSignIn,
    }
}

pub fn connect_kind_from_proto(value: i32) -> api::ConnectKind {
    match ConnectKind::try_from(value) {
        Ok(ConnectKind::WebSignIn) => api::ConnectKind::WebSignIn,
        Ok(ConnectKind::None) | Ok(ConnectKind::Unspecified) | Err(_) => api::ConnectKind::None,
    }
}

pub fn integration_info_to_proto(value: &api::IntegrationInfo) -> IntegrationInfo {
    IntegrationInfo {
        id: value.id.clone(),
        name: Some(text_to_proto(&value.name)),
        icon: Some(icon_to_proto(&value.icon)),
        configured: value.configured,
        connect: connect_kind_to_proto(value.connect) as i32,
        fields: value.fields.iter().map(field_spec_to_proto).collect(),
    }
}

pub fn integration_info_from_proto(value: &IntegrationInfo) -> api::IntegrationInfo {
    api::IntegrationInfo {
        id: value.id.clone(),
        name: value.name.as_ref().map(text_from_proto).unwrap_or_default(),
        icon: value.icon.as_ref().map(icon_from_proto).unwrap_or_default(),
        configured: value.configured,
        connect: connect_kind_from_proto(value.connect),
        fields: value.fields.iter().map(field_spec_from_proto).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jellyfin() -> api::ServiceRef {
        api::ServiceRef {
            id: "jellyfin".into(),
            name: api::Text::literal("Jellyfin"),
            icon: api::Icon::Class("ph-cloud".into()),
            accent: "#aa5cc3".into(),
        }
    }

    fn url_field() -> api::FieldSpec {
        api::FieldSpec {
            key: "url".into(),
            label: api::Text::key("settings-server-url"),
            kind: api::FieldKind::Url,
            required: true,
            value: Some("https://jelly.example".into()),
            ..Default::default()
        }
    }

    /// A source row is what a settings page renders, so every field of it has
    /// to survive the wire -- and none of them may be a credential.
    #[test]
    fn a_source_row_round_trips_without_carrying_a_secret() {
        let info = api::SourceInfo {
            id: "jellyfin-1".into(),
            name: "Home".into(),
            kind: api::SourceKind::Server,
            service: Some(jellyfin()),
            active: true,
            authenticated: true,
            sign_in: api::SignInKind::Password,
            capabilities: api::SourceCapabilities {
                edit_tags: false,
                delete_from_disk: false,
                sync: true,
                downloads: true,
                browse_folders: true,
                external_devices: false,
                playlists: api::PlaylistCapability::Reorder,
                artists: api::ArtistPresentation::Library,
                albums: api::AlbumPresentation::Standard,
                favorites_sync: api::FavoritesSyncMode::Paginated,
                ..Default::default()
            },
            detail: Some("https://jelly.example".into()),
            anonymous: false,
            settings: vec![url_field()],
            directories: vec!["/Music".into()],
        };
        assert_eq!(info, source_info_from_proto(&source_info_to_proto(&info)));
    }

    /// The form a client renders to add a server, and the answers it sends
    /// back -- secrets included, since that direction is write-only.
    #[test]
    fn a_service_and_its_draft_round_trip() {
        let service = api::ServiceInfo {
            id: "jellyfin".into(),
            name: api::Text::literal("Jellyfin"),
            icon: api::Icon::Svg("M0 0h24v24H0z".into()),
            accent: "#aa5cc3".into(),
            experimental: false,
            fields: vec![
                url_field(),
                api::FieldSpec {
                    key: "password".into(),
                    label: api::Text::key("settings-password"),
                    kind: api::FieldKind::Secret,
                    ..Default::default()
                },
            ],
        };
        assert_eq!(
            service,
            service_info_from_proto(&service_info_to_proto(&service))
        );

        let draft = api::ServerDraft {
            id: Some("jellyfin-1".into()),
            name: "Home".into(),
            service: "jellyfin".into(),
            values: vec![api::FieldValue::new("url", "https://jelly.example")],
            secrets: vec![api::FieldValue::new("password", "hunter2")],
        };
        assert_eq!(
            draft,
            server_draft_from_proto(&server_draft_to_proto(&draft))
        );

        let check = api::DraftCheck {
            sign_in: api::SignInKind::Browser,
            problems: vec![api::Problem::on("url", api::Text::key("error-url-empty"))],
        };
        assert_eq!(check, draft_check_from_proto(&draft_check_to_proto(&check)));
    }

    /// A sign-in kind this build cannot name is one it cannot drive: treat it
    /// as nothing to do rather than guessing a flow.
    #[test]
    fn an_unknown_sign_in_kind_is_none() {
        assert_eq!(sign_in_kind_from_proto(0), api::SignInKind::None);
        assert_eq!(sign_in_kind_from_proto(404), api::SignInKind::None);
    }

    /// An integration row is rendered by the same code as a service, so it
    /// carries the same field list -- and, like a source, never a credential.
    #[test]
    fn an_integration_row_round_trips_for_every_connect_kind() {
        for connect in [api::ConnectKind::None, api::ConnectKind::WebSignIn] {
            let info = api::IntegrationInfo {
                id: "listenbrainz".into(),
                name: api::Text::literal("ListenBrainz"),
                icon: api::Icon::Class("ph-broadcast".into()),
                configured: true,
                connect,
                fields: vec![
                    url_field(),
                    api::FieldSpec {
                        key: "token".into(),
                        label: api::Text::key("settings-token"),
                        kind: api::FieldKind::Secret,
                        ..Default::default()
                    },
                ],
            };
            assert_eq!(
                info,
                integration_info_from_proto(&integration_info_to_proto(&info))
            );
        }
    }

    /// A connect flow this build cannot name is one it cannot run: offer no
    /// button rather than guessing at one.
    #[test]
    fn an_unknown_connect_kind_is_none() {
        assert_eq!(connect_kind_from_proto(0), api::ConnectKind::None);
        assert_eq!(connect_kind_from_proto(404), api::ConnectKind::None);
    }
}
