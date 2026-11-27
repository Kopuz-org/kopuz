use crate::*;

pub fn plugin_info_to_proto(value: &api::PluginInfo) -> PluginInfo {
    PluginInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        description: value.description.clone(),
        author: value.author.clone(),
        homepage: value.homepage.clone(),
        origin: match value.origin {
            api::PluginOrigin::Official => PluginOrigin::Official,
            api::PluginOrigin::Community => PluginOrigin::Community,
        } as i32,
        installed_version: value.installed_version.clone(),
        available_version: value.available_version.clone(),
        services: value.services.clone(),
        network: value.network.clone(),
        sandboxed: value.sandboxed,
        system: value.system,
        reads_folders: value.reads_folders,
        integration: value.integration,
        controls_playback: value.controls_playback,
        reads_library: value.reads_library,
        update: value.update.as_ref().map(|update| PluginUpdate {
            version: update.version.clone(),
            network: update.network.clone(),
            added_hosts: update.added_hosts.clone(),
        }),
    }
}

pub fn plugin_info_from_proto(value: &PluginInfo) -> api::PluginInfo {
    api::PluginInfo {
        id: value.id.clone(),
        name: value.name.clone(),
        description: value.description.clone(),
        author: value.author.clone(),
        homepage: value.homepage.clone(),
        // An origin this build does not know is not vouched for.
        origin: match PluginOrigin::try_from(value.origin) {
            Ok(PluginOrigin::Official) => api::PluginOrigin::Official,
            _ => api::PluginOrigin::Community,
        },
        installed_version: value.installed_version.clone(),
        available_version: value.available_version.clone(),
        services: value.services.clone(),
        network: value.network.clone(),
        sandboxed: value.sandboxed,
        system: value.system,
        reads_folders: value.reads_folders,
        integration: value.integration,
        controls_playback: value.controls_playback,
        reads_library: value.reads_library,
        update: value.update.as_ref().map(|update| api::PluginUpdate {
            version: update.version.clone(),
            network: update.network.clone(),
            added_hosts: update.added_hosts.clone(),
        }),
    }
}

pub fn plugin_catalog_to_proto(value: &api::PluginCatalog) -> PluginCatalog {
    PluginCatalog {
        plugins: value.plugins.iter().map(plugin_info_to_proto).collect(),
        marketplace_error: value.marketplace_error.clone(),
    }
}

pub fn plugin_catalog_from_proto(value: &PluginCatalog) -> api::PluginCatalog {
    api::PluginCatalog {
        plugins: value.plugins.iter().map(plugin_info_from_proto).collect(),
        marketplace_error: value.marketplace_error.clone(),
    }
}

fn action_place_to_proto(place: api::ActionPlace) -> ActionPlace {
    match place {
        api::ActionPlace::Palette => ActionPlace::Palette,
        api::ActionPlace::Track => ActionPlace::Track,
        api::ActionPlace::Album => ActionPlace::Album,
    }
}

/// A place this build does not know is left out rather than guessed.
fn action_place_from_proto(place: i32) -> Option<api::ActionPlace> {
    match ActionPlace::try_from(place).ok()? {
        ActionPlace::Palette => Some(api::ActionPlace::Palette),
        ActionPlace::Track => Some(api::ActionPlace::Track),
        ActionPlace::Album => Some(api::ActionPlace::Album),
        ActionPlace::Unspecified => None,
    }
}

pub fn plugin_actions_to_proto(actions: &[api::PluginAction]) -> PluginActionList {
    PluginActionList {
        actions: actions
            .iter()
            .map(|action| PluginAction {
                plugin: action.plugin.clone(),
                id: action.id.clone(),
                label: action.label.clone(),
                places: action
                    .places
                    .iter()
                    .map(|place| action_place_to_proto(*place) as i32)
                    .collect(),
                ask: action.ask.clone(),
            })
            .collect(),
    }
}

pub fn plugin_actions_from_proto(list: &PluginActionList) -> Vec<api::PluginAction> {
    list.actions
        .iter()
        .map(|action| api::PluginAction {
            plugin: action.plugin.clone(),
            id: action.id.clone(),
            label: action.label.clone(),
            places: action
                .places
                .iter()
                .filter_map(|place| action_place_from_proto(*place))
                .collect(),
            ask: action.ask.clone(),
        })
        .collect()
}

pub fn run_plugin_action_to_proto(
    plugin: String,
    id: String,
    target: api::ActionTarget,
    input: Option<String>,
) -> RunPluginActionRequest {
    let (track_keys, album_id) = match target {
        api::ActionTarget::None => (Vec::new(), None),
        api::ActionTarget::Tracks(keys) => (keys, None),
        api::ActionTarget::Album(id) => (Vec::new(), Some(id)),
    };
    RunPluginActionRequest {
        plugin,
        id,
        track_keys,
        album_id,
        input,
    }
}

/// The request's target, plugin, id and input.
pub fn run_plugin_action_from_proto(
    request: RunPluginActionRequest,
) -> (String, String, api::ActionTarget, Option<String>) {
    let target = match (request.album_id, request.track_keys.is_empty()) {
        (Some(album), _) => api::ActionTarget::Album(album),
        (None, false) => api::ActionTarget::Tracks(request.track_keys),
        (None, true) => api::ActionTarget::None,
    };
    (request.plugin, request.id, target, request.input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_and_a_run_survive_the_wire() {
        let actions = vec![api::PluginAction {
            plugin: "remote".into(),
            id: "find".into(),
            label: "Play what the library finds for…".into(),
            places: vec![api::ActionPlace::Palette, api::ActionPlace::Track],
            ask: Some("Words".into()),
        }];
        assert_eq!(
            plugin_actions_from_proto(&plugin_actions_to_proto(&actions)),
            actions
        );
        for target in [
            api::ActionTarget::None,
            api::ActionTarget::Tracks(vec!["a".into(), "b".into()]),
            api::ActionTarget::Album("al-1".into()),
        ] {
            let request = run_plugin_action_to_proto(
                "remote".into(),
                "find".into(),
                target.clone(),
                Some("blue".into()),
            );
            assert_eq!(
                run_plugin_action_from_proto(request),
                ("remote".into(), "find".into(), target, Some("blue".into()))
            );
        }
    }

    #[test]
    fn a_catalog_survives_the_wire() {
        let catalog = api::PluginCatalog {
            plugins: vec![
                api::PluginInfo {
                    id: "subsonic".into(),
                    name: "Subsonic".into(),
                    origin: api::PluginOrigin::Official,
                    installed_version: Some("0.1.0".into()),
                    services: vec!["subsonic".into(), "custom".into()],
                    network: vec!["$server".into()],
                    ..Default::default()
                },
                api::PluginInfo {
                    id: "bandcamp".into(),
                    name: "Bandcamp".into(),
                    description: "Buy and stream".into(),
                    author: "someone".into(),
                    homepage: Some("https://example.test".into()),
                    origin: api::PluginOrigin::Community,
                    sandboxed: true,
                    installed_version: None,
                    available_version: Some("0.2.0".into()),
                    services: vec!["bandcamp".into()],
                    network: vec!["*.bandcamp.com".into()],
                    update: None,
                    system: false,
                    reads_folders: true,
                    integration: false,
                    controls_playback: false,
                    reads_library: false,
                },
                api::PluginInfo {
                    id: "webhook".into(),
                    origin: api::PluginOrigin::Community,
                    sandboxed: true,
                    available_version: Some("0.1.0".into()),
                    network: vec!["$server".into()],
                    integration: true,
                    controls_playback: true,
                    reads_library: true,
                    ..Default::default()
                },
                api::PluginInfo {
                    id: "radio".into(),
                    origin: api::PluginOrigin::Community,
                    sandboxed: true,
                    system: true,
                    installed_version: Some("1.0.0".into()),
                    network: vec!["a.test".into()],
                    update: Some(api::PluginUpdate {
                        version: "1.1.0".into(),
                        network: vec!["a.test".into(), "b.test".into()],
                        added_hosts: vec!["b.test".into()],
                    }),
                    ..Default::default()
                },
            ],
            marketplace_error: Some("offline".into()),
        };
        assert_eq!(
            plugin_catalog_from_proto(&plugin_catalog_to_proto(&catalog)),
            catalog
        );
    }

    #[test]
    fn an_unknown_origin_reads_as_community() {
        let wire = PluginInfo {
            origin: 99,
            ..Default::default()
        };
        assert_eq!(
            plugin_info_from_proto(&wire).origin,
            api::PluginOrigin::Community
        );
    }
}
