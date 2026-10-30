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

#[cfg(test)]
mod tests {
    use super::*;

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
