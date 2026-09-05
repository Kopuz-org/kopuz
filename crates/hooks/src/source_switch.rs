//! The shared source-switch path for the sidebar and Settings. The daemon owns
//! source configuration and credentials, while the frontend mirrors the
//! resulting source list and capabilities.

use std::sync::Arc;

use api::KopuzApi;
use dioxus::prelude::*;

/// Live connection status of the active source, for the switcher's indicator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnStatus {
    /// Verifying auth / reaching the server (the loading state).
    Connecting,
    /// Verified and reachable.
    Online,
    /// Unreachable, or auth expired/invalid.
    Offline,
}

/// Connection status of the active source: local libraries are always Online
/// (no auth); a server runs `validate()` on each switch.
pub fn use_connection_status() -> Memo<ConnStatus> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let sources = use_context::<Signal<Vec<api::SourceInfo>>>();
    let mut status = use_signal(|| ConnStatus::Connecting);
    use_effect(move || {
        let active = sources.read().iter().find(|source| source.active).cloned();
        let Some(source) = active else {
            status.set(ConnStatus::Connecting);
            return;
        };
        if source.kind != api::SourceKind::Server {
            status.set(ConnStatus::Online);
            return;
        }
        status.set(ConnStatus::Connecting);
        let api = api.clone();
        spawn(async move {
            let outcome = api.validate_source(source.id).await;
            status.set(match outcome {
                Ok(api::SourceState::Online) => ConnStatus::Online,
                Ok(api::SourceState::Offline | api::SourceState::AuthExpired) | Err(_) => {
                    ConnStatus::Offline
                }
            });
        });
    });
    use_memo(move || *status.read())
}

/// Apply a source switch and return whether the selected source is already
/// authenticated, so the caller can launch a sign-in flow when needed.
pub async fn apply_source_switch(api: Arc<dyn KopuzApi>, source_key: String) -> bool {
    match api.switch_source(source_key.clone()).await {
        Ok(info) => {
            tracing::info!(target: "kopuz::source", source = %source_key, "source switched");
            info.authenticated
        }
        Err(error) => {
            tracing::warn!(target: "kopuz::source", source = %source_key, %error, "source switch failed");
            false
        }
    }
}

/// A fire-and-forget source switcher for the sidebar. Settings owns sign-in.
pub fn use_switch_source() -> impl Fn(String) + Clone {
    let api = use_context::<Arc<dyn KopuzApi>>();
    move |source: String| {
        let api = api.clone();
        spawn(async move {
            apply_source_switch(api, source).await;
        });
    }
}
