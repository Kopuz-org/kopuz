//! Switching what the app plays from, and whether that source is reachable.
//!
//! The daemon owns the sources: a switch loads that server's stored
//! credentials into the active snapshot, which a client could not do through
//! `set_config` -- credential fields are exactly what that refuses to take.

use config::AppConfig;
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

/// Connection status of the active source, probed by the daemon on each switch.
pub fn use_connection_status() -> Memo<ConnStatus> {
    let api = crate::api::use_api();
    let sources = crate::sources::use_sources();
    let mut status = use_signal(|| ConnStatus::Connecting);
    use_effect(move || {
        let active = sources
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .find(|source| source.active);
        let Some(active) = active else {
            return;
        };
        status.set(ConnStatus::Connecting);
        let api = api.clone();
        spawn(async move {
            let state = match api.validate_source(active.id.clone()).await {
                Ok(api::SourceState::Online) => ConnStatus::Online,
                _ => ConnStatus::Offline,
            };
            // A switch away while this was in flight has its own probe; this answer is about a source no longer shown.
            let still_active = sources.peek().as_ref().is_some_and(|all| {
                all.iter()
                    .any(|source| source.active && source.id == active.id)
            });
            if still_active {
                status.set(state);
            }
        });
    });
    use_memo(move || *status.read())
}

/// Apply a source switch. Answers whether the source is usable without a
/// sign-in (stored credentials, or a source usable anonymously), so the caller can
/// launch a sign-in flow otherwise.
pub async fn apply_source_switch(mut config: Signal<AppConfig>, id: String) -> bool {
    let api = crate::api::consume_api();
    match api.switch_source(id).await {
        Ok(info) => {
            let usable = info.authenticated;
            // The daemon owns the config now, so pull its version back rather
            // than reconstructing the same edit locally.
            if let Ok(view) = api.config().await {
                config.set(view.config);
            }
            usable
        }
        Err(error) => {
            tracing::warn!(%error, "source switch failed");
            crate::toast::toast_error(&error.to_string());
            false
        }
    }
}

/// A fire-and-forget source switcher for the sidebar: switches (loading
/// credentials) without launching a sign-in flow -- the settings page owns that.
pub fn use_switch_source() -> impl Fn(String) + Clone {
    let config = use_context::<Signal<AppConfig>>();
    move |id: String| {
        spawn(async move {
            apply_source_switch(config, id).await;
        });
    }
}
