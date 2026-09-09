//! Switching what the app plays from, and whether that source is reachable.
//!
//! The daemon owns the sources: a switch loads that server's stored
//! credentials into the active snapshot, which a client could not do through
//! `set_config` -- credential fields are exactly what that refuses to take.

use config::{AppConfig, Source};
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
/// (no auth); a server is probed by the daemon on each switch.
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
        if active.kind != api::SourceKind::Server {
            status.set(ConnStatus::Online);
            return;
        }
        status.set(ConnStatus::Connecting);
        let api = api.clone();
        spawn(async move {
            status.set(match api.validate_source(active.id).await {
                Ok(api::SourceState::Online) => ConnStatus::Online,
                _ => ConnStatus::Offline,
            });
        });
    });
    use_memo(move || *status.read())
}

/// Apply a source switch. Answers whether the source is usable without a
/// sign-in (stored credentials, or anonymous YouTube Music), so the caller can
/// launch a sign-in flow otherwise.
pub async fn apply_source_switch(mut config: Signal<AppConfig>, source: Source) -> bool {
    let api = crate::api::consume_api();
    match api.switch_source(source.as_str().to_string()).await {
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
pub fn use_switch_source() -> impl Fn(Source) + Clone {
    let config = use_context::<Signal<AppConfig>>();
    move |source: Source| {
        spawn(async move {
            apply_source_switch(config, source).await;
        });
    }
}
