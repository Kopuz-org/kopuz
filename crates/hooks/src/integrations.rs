//! Scrobbling services, as the UI sees them.
//!
//! A session key is a credential, so it lives with the daemon: this asks
//! whether a service is connected and tells it to connect, and never holds
//! the answer. The web sign-in opens a browser, which is the daemon's to do.

use dioxus::prelude::*;

use crate::api::{consume_api, use_api};
use crate::db_reactivity::{Table, use_generations};
use crate::toast::toast_error;

/// Which services are configured, re-read when one changes.
pub fn use_integrations() -> Resource<Vec<api::IntegrationStatus>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Servers);
        let api = api.clone();
        async move { api.integrations().await.unwrap_or_default() }
    })
}

/// Whether one service is connected, for a settings row.
pub fn is_configured(
    statuses: &Resource<Vec<api::IntegrationStatus>>,
    kind: api::IntegrationKind,
) -> bool {
    statuses
        .read()
        .clone()
        .unwrap_or_default()
        .iter()
        .any(|status| status.kind == kind && status.configured)
}

/// Hand over a credential the person typed.
pub fn provision(provision: api::IntegrationProvision, mut done: Signal<u64>) {
    let api = consume_api();
    spawn(async move {
        match api.provision_integration(provision).await {
            Ok(_) => done += 1,
            Err(error) => {
                tracing::warn!(%error, "saving a scrobbling credential failed");
                toast_error(&error.to_string());
            }
        }
    });
}

/// Run a service's web sign-in in the daemon and keep what it returns.
pub fn authenticate(kind: api::IntegrationKind, mut done: Signal<u64>) {
    let api = consume_api();
    spawn(async move {
        match api.authenticate_integration(kind).await {
            Ok(_) => done += 1,
            Err(error) => {
                tracing::warn!(%error, ?kind, "connecting a scrobbling service failed");
                toast_error(&error.to_string());
            }
        }
    });
}
