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
        api::ActionPlace::Artist => ActionPlace::Artist,
    }
}

/// A place this build does not know is left out rather than guessed.
fn action_place_from_proto(place: i32) -> Option<api::ActionPlace> {
    match ActionPlace::try_from(place).ok()? {
        ActionPlace::Palette => Some(api::ActionPlace::Palette),
        ActionPlace::Track => Some(api::ActionPlace::Track),
        ActionPlace::Album => Some(api::ActionPlace::Album),
        ActionPlace::Artist => Some(api::ActionPlace::Artist),
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
    let (track_keys, album_id, artist_key) = match target {
        api::ActionTarget::None => (Vec::new(), None, None),
        api::ActionTarget::Tracks(keys) => (keys, None, None),
        api::ActionTarget::Album(id) => (Vec::new(), Some(id), None),
        api::ActionTarget::Artist(key) => (Vec::new(), None, Some(key)),
    };
    RunPluginActionRequest {
        plugin,
        id,
        track_keys,
        album_id,
        input,
        artist_key,
    }
}

/// The request's target, plugin, id and input.
pub fn run_plugin_action_from_proto(
    request: RunPluginActionRequest,
) -> (String, String, api::ActionTarget, Option<String>) {
    let target = match (request.album_id, request.artist_key) {
        (Some(album), _) => api::ActionTarget::Album(album),
        (None, Some(artist)) => api::ActionTarget::Artist(artist),
        (None, None) if !request.track_keys.is_empty() => {
            api::ActionTarget::Tracks(request.track_keys)
        }
        (None, None) => api::ActionTarget::None,
    };
    (request.plugin, request.id, target, request.input)
}

/// A place this build does not know is left out rather than guessed.
fn panel_place_from_proto(place: i32) -> Option<api::PanelPlace> {
    match PanelPlace::try_from(place).ok()? {
        PanelPlace::Album => Some(api::PanelPlace::Album),
        PanelPlace::Artist => Some(api::PanelPlace::Artist),
        PanelPlace::Unspecified => None,
    }
}

pub fn plugin_panels_to_proto(panels: &[api::PluginPanel]) -> PluginPanelList {
    PluginPanelList {
        panels: panels
            .iter()
            .map(|panel| PluginPanel {
                plugin: panel.plugin.clone(),
                id: panel.id.clone(),
                title: panel.title.clone(),
                places: panel
                    .places
                    .iter()
                    .map(|place| match place {
                        api::PanelPlace::Album => PanelPlace::Album as i32,
                        api::PanelPlace::Artist => PanelPlace::Artist as i32,
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn plugin_panels_from_proto(list: &PluginPanelList) -> Vec<api::PluginPanel> {
    list.panels
        .iter()
        .map(|panel| api::PluginPanel {
            plugin: panel.plugin.clone(),
            id: panel.id.clone(),
            title: panel.title.clone(),
            places: panel
                .places
                .iter()
                .filter_map(|place| panel_place_from_proto(*place))
                .collect(),
        })
        .collect()
}

pub fn render_plugin_panel_to_proto(
    plugin: String,
    id: String,
    target: api::PanelTarget,
) -> RenderPluginPanelRequest {
    RenderPluginPanelRequest {
        plugin,
        id,
        target: Some(match target {
            api::PanelTarget::Album(album) => render_plugin_panel_request::Target::AlbumId(album),
            api::PanelTarget::Artist(artist) => {
                render_plugin_panel_request::Target::ArtistKey(artist)
            }
        }),
    }
}

/// The request's plugin, panel and target; a request naming no target is invalid.
pub fn render_plugin_panel_from_proto(
    request: RenderPluginPanelRequest,
) -> Option<(String, String, api::PanelTarget)> {
    let target = match request.target? {
        render_plugin_panel_request::Target::AlbumId(album) => api::PanelTarget::Album(album),
        render_plugin_panel_request::Target::ArtistKey(artist) => api::PanelTarget::Artist(artist),
    };
    Some((request.plugin, request.id, target))
}

pub fn panel_blocks_to_proto(blocks: &[api::PanelBlock]) -> PanelBlockList {
    PanelBlockList {
        blocks: blocks
            .iter()
            .map(|block| PanelBlock {
                kind: Some(match block {
                    api::PanelBlock::Heading(text) => panel_block::Kind::Heading(text.clone()),
                    api::PanelBlock::Text(text) => panel_block::Kind::Text(text.clone()),
                    api::PanelBlock::Tracks(keys) => {
                        panel_block::Kind::Tracks(PanelTracks { keys: keys.clone() })
                    }
                    api::PanelBlock::Action { label, action } => {
                        panel_block::Kind::Action(PanelActionButton {
                            label: label.clone(),
                            action: action.clone(),
                        })
                    }
                }),
            })
            .collect(),
    }
}

/// A block of a kind this build does not know is left out.
pub fn panel_blocks_from_proto(list: &PanelBlockList) -> Vec<api::PanelBlock> {
    list.blocks
        .iter()
        .filter_map(|block| {
            Some(match block.kind.as_ref()? {
                panel_block::Kind::Heading(text) => api::PanelBlock::Heading(text.clone()),
                panel_block::Kind::Text(text) => api::PanelBlock::Text(text.clone()),
                panel_block::Kind::Tracks(tracks) => api::PanelBlock::Tracks(tracks.keys.clone()),
                panel_block::Kind::Action(button) => api::PanelBlock::Action {
                    label: button.label.clone(),
                    action: button.action.clone(),
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_and_what_they_draw_survive_the_wire() {
        let panels = vec![api::PluginPanel {
            plugin: "remote".into(),
            id: "more".into(),
            title: "More by this artist".into(),
            places: vec![api::PanelPlace::Album, api::PanelPlace::Artist],
        }];
        assert_eq!(
            plugin_panels_from_proto(&plugin_panels_to_proto(&panels)),
            panels
        );
        for target in [
            api::PanelTarget::Album("al-1".into()),
            api::PanelTarget::Artist("miles davis".into()),
        ] {
            let request =
                render_plugin_panel_to_proto("remote".into(), "more".into(), target.clone());
            assert_eq!(
                render_plugin_panel_from_proto(request),
                Some(("remote".into(), "more".into(), target))
            );
        }
        let blocks = vec![
            api::PanelBlock::Heading("More".into()),
            api::PanelBlock::Text("by Miles".into()),
            api::PanelBlock::Tracks(vec!["a".into(), "b".into()]),
            api::PanelBlock::Action {
                label: "Play them all".into(),
                action: "more".into(),
            },
        ];
        assert_eq!(
            panel_blocks_from_proto(&panel_blocks_to_proto(&blocks)),
            blocks
        );
    }

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
            api::ActionTarget::Artist("miles davis".into()),
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
