//! How covers are looked up, as the daemon publishes it.
//!
//! Which providers exist and in what order they are tried belongs to the
//! daemon, which does the looking up; a settings page renders the field list
//! and sends back what was picked.

use dioxus::prelude::*;

use crate::api::{consume_api, use_api};

pub fn use_settings(reload: Signal<u64>) -> Resource<Vec<api::FieldSpec>> {
    let api = use_api();
    use_resource(move || {
        let _ = reload();
        let api = api.clone();
        async move { api.artwork_settings().await.unwrap_or_default() }
    })
}

pub fn set(value: api::FieldValue, mut done: Signal<u64>) {
    let api = consume_api();
    spawn(async move {
        match api.set_artwork_settings(vec![value]).await {
            Ok(_) => done += 1,
            Err(error) => {
                tracing::warn!(%error, "saving a cover setting failed");
                crate::toast::toast_error(&error.to_string());
            }
        }
    });
}
