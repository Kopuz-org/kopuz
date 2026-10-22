//! Plugins and the marketplace. The daemon fetches, verifies and installs.

use dioxus::prelude::*;

use crate::api::{consume_api, use_api};
use crate::toast::toast_api_error;

/// Bump `changed` to ask again.
pub fn use_plugins(changed: Signal<u64>) -> Resource<Option<api::PluginCatalog>> {
    let api = use_api();
    use_resource(move || {
        let _ = changed();
        let api = api.clone();
        async move {
            api.plugins()
                .await
                .inspect_err(|error| tracing::warn!(%error, "listing plugins failed"))
                .ok()
        }
    })
}

pub fn install(id: String, mut busy: Signal<Option<String>>, mut changed: Signal<u64>) {
    let api = consume_api();
    busy.set(Some(id.clone()));
    spawn(async move {
        if let Err(error) = api.install_plugin(id.clone()).await {
            tracing::warn!(%error, %id, "installing a plugin failed");
            toast_api_error(&error);
        }
        busy.set(None);
        changed += 1;
    });
}

pub fn uninstall(id: String, mut busy: Signal<Option<String>>, mut changed: Signal<u64>) {
    let api = consume_api();
    busy.set(Some(id.clone()));
    spawn(async move {
        if let Err(error) = api.uninstall_plugin(id.clone()).await {
            tracing::warn!(%error, %id, "removing a plugin failed");
            toast_api_error(&error);
        }
        busy.set(None);
        changed += 1;
    });
}
