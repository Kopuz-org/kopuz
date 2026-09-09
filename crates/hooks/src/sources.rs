//! What is configured to play from, as the UI reads it.
//!
//! A frontend used to build a media source and ask it what it could do. It
//! reads rows now: the daemon owns the source, and `SourceCapabilities`
//! answers every "should this button exist?" question without anyone
//! branching on a service name.

use dioxus::prelude::*;

use crate::api::use_api;
use crate::db_reactivity::{Table, use_generations};

/// The configured sources, re-read when one changes or the active one moves.
pub fn use_sources() -> Resource<Vec<api::SourceInfo>> {
    let api = use_api();
    let gens = use_generations();
    use_resource(move || {
        let _ = gens.generation(Table::Servers);
        let api = api.clone();
        async move { api.sources().await.unwrap_or_default() }
    })
}

/// Provide the active source's capabilities to the tree.
///
/// One resource feeds a plain signal so every consumer reads it
/// synchronously while rendering, which is what the in-process handle used
/// to give them.
pub fn use_capabilities_provider() -> Signal<api::SourceCapabilities> {
    let sources = use_sources();
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
