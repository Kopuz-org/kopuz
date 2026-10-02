//! What is configured to play from, as the UI reads it.
//!
//! A frontend used to build a media source and ask it what it could do. It
//! reads rows now: the daemon owns the source, and `SourceCapabilities`
//! answers every "should this button exist?" question without anyone
//! branching on a service name.

use dioxus::prelude::*;

use crate::api::use_api;
use crate::db_reactivity::{Table, use_generations};

/// The configured sources every reader shares, so a switch shows everywhere at once.
#[derive(Clone, Copy)]
struct SourceRows(Signal<Option<Vec<api::SourceInfo>>>);

/// Fetch the configured sources once for the tree, re-read when one changes or the active one moves.
fn use_sources_provider() -> Signal<Option<Vec<api::SourceInfo>>> {
    let api = use_api();
    let gens = use_generations();
    let fetched = use_resource(move || {
        let _ = gens.generation(Table::Servers);
        let api = api.clone();
        async move { api.sources().await.unwrap_or_default() }
    });
    let mut rows = use_context_provider(|| SourceRows(Signal::new(None))).0;
    use_effect(move || {
        let next = fetched.read().clone();
        if next.is_some() && *rows.peek() != next {
            rows.set(next);
        }
    });
    rows
}

/// The configured sources; `None` until the first answer arrives.
pub fn use_sources() -> Signal<Option<Vec<api::SourceInfo>>> {
    use_context::<SourceRows>().0
}

/// Show a source the daemon just switched to as the active one now, ahead of the refetch that confirms it.
pub fn show_active(switched: &api::SourceInfo) {
    let Some(SourceRows(mut rows)) = try_consume_context::<SourceRows>() else {
        return;
    };
    let mut rows = rows.write();
    let Some(rows) = rows.as_mut() else {
        return;
    };
    for row in rows.iter_mut() {
        if row.id == switched.id {
            *row = switched.clone();
        }
        row.active = row.id == switched.id;
    }
}

/// Provide the source list and the active source's capabilities to the tree.
///
/// One resource feeds plain signals so every consumer reads them
/// synchronously while rendering, which is what the in-process handle used
/// to give them.
pub fn use_capabilities_provider() -> Signal<api::SourceCapabilities> {
    let sources = use_sources_provider();
    let mut caps = use_context_provider(|| Signal::new(api::SourceCapabilities::default()));
    use_effect(move || {
        let next = sources
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .find(|source| source.active)
            .map(|source| source.capabilities)
            .unwrap_or_default();
        if *caps.peek() != next {
            caps.set(next);
        }
    });
    caps
}

/// The active source's capabilities.
pub fn use_capabilities() -> Signal<api::SourceCapabilities> {
    use_context::<Signal<api::SourceCapabilities>>()
}

/// The active source's row, for a settings page that needs its name or URL.
pub fn use_active_source_info() -> Memo<Option<api::SourceInfo>> {
    let sources = use_sources();
    use_memo(move || {
        sources
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .find(|source| source.active)
    })
}

/// Every service this daemon can be pointed at, and the form that adds one.
pub fn use_services() -> Resource<Vec<api::ServiceInfo>> {
    let api = use_api();
    use_resource(move || {
        let api = api.clone();
        async move { api.services().await.unwrap_or_default() }
    })
}

/// Answer one of a source's own options. The daemon decides where the value
/// lives -- the server row or the settings -- so this only carries it there.
pub fn set_source_settings(id: String, values: Vec<api::FieldValue>) {
    let api = crate::api::consume_api();
    spawn(async move {
        if let Err(error) = api.set_source_settings(id, values).await {
            tracing::warn!(%error, "saving a source setting failed");
            crate::toast::toast_error(&error.to_string());
        }
    });
}
