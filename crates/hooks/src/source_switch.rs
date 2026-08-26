//! The one place a source switch happens, shared by the sidebar source switcher
//! and the Settings "Switch" button so they behave identically. A switch keeps
//! `config.active_source` and `config.server` (the active server's connection
//! snapshot, which the source resolver reads for the URL + creds) consistent —
//! both set in a single `config.write()` so the active `MediaSource` rebuilds
//! exactly once, with the new server, and never on a stale connection.

use std::sync::Arc;

use api::KopuzApi;
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
/// (no auth); a server runs `validate()` on each switch.
pub fn use_connection_status() -> Memo<ConnStatus> {
    let api = use_context::<Arc<dyn KopuzApi>>();
    let config = use_context::<Signal<AppConfig>>();
    let mut status = use_signal(|| ConnStatus::Connecting);
    use_effect(move || {
        // Subscribe to the active source (rebuilds on switch); `peek` the config
        // so a volume/theme change doesn't trigger a re-validation.
        let source = config.read().active_source.clone();
        if source.is_local() {
            status.set(ConnStatus::Online);
            return;
        }
        status.set(ConnStatus::Connecting);
        let api = api.clone();
        spawn(async move {
            let outcome = api.validate_source(source.as_str().to_string()).await;
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

/// Apply a source switch. For a server it loads the stored creds from the DB (so
/// the connection is the new server's, not a leftover one) and writes
/// `active_source` and `server` together; for Local it clears the server snapshot.
/// Returns whether the source is usable without a sign-in (stored creds, or
/// anonymous YT), so the caller can launch a sign-in flow otherwise.
pub async fn apply_source_switch(api: Arc<dyn KopuzApi>, source: Source) -> bool {
    let source_key = source.as_str().to_string();
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

/// A fire-and-forget source switcher for the sidebar: switches (loading creds)
/// without launching a sign-in flow — the Settings page owns that.
pub fn use_switch_source() -> impl Fn(Source) + Clone {
    let api = use_context::<Arc<dyn KopuzApi>>();
    move |source: Source| {
        let api = api.clone();
        spawn(async move {
            apply_source_switch(api, source).await;
        });
    }
}
