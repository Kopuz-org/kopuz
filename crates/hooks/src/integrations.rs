//! Scrobblers, presence and the rest of what is configured per account.
//!
//! A session key is a credential, so it lives with the daemon: this asks what
//! is offered and whether each is connected, hands over what a person typed,
//! and never holds the answer. The web sign-in opens a browser, which is the
//! daemon's to do.

use dioxus::prelude::*;

use crate::api::{consume_api, use_api};
use crate::db_reactivity::{Table, use_generations};
use crate::toast::toast_error;

/// What this daemon can be connected to, and how each is configured.
pub fn use_integrations() -> Resource<Vec<api::IntegrationInfo>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Servers);
        let api = api.clone();
        async move { api.integrations().await.unwrap_or_default() }
    })
}

/// Answer one integration's fields. An empty secret is not a request to clear
/// one, so a field left alone stays as it is.
pub fn set_settings(id: String, values: Vec<api::FieldValue>, mut done: Signal<u64>) {
    let api = consume_api();
    spawn(async move {
        match api.set_integration_settings(id, values).await {
            Ok(_) => done += 1,
            Err(error) => {
                tracing::warn!(%error, "saving an integration setting failed");
                toast_error(&error.to_string());
            }
        }
    });
}

/// Run an integration's web sign-in in the daemon and keep what it returns.
pub fn authenticate(id: String, mut done: Signal<u64>) {
    let api = consume_api();
    spawn(async move {
        match api.authenticate_integration(id.clone()).await {
            Ok(_) => done += 1,
            Err(error) => {
                tracing::warn!(%error, %id, "connecting an integration failed");
                toast_error(&error.to_string());
            }
        }
    });
}
