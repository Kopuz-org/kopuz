//! The one place a source switch happens, shared by the sidebar source switcher
//! and the Settings "Switch" button so they behave identically. A switch keeps
//! `config.active_source` and `config.server` (the active server's connection
//! snapshot, which the source resolver reads for the URL + creds) consistent —
//! both set in a single `config.write()` so the active `MediaSource` rebuilds
//! exactly once, with the new server, and never on a stale connection.

use config::{AppConfig, Source};
use dioxus::prelude::*;
use server::source::{ActiveSource, AuthOutcome};

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
    let active_source = use_context::<Signal<ActiveSource>>();
    let config = use_context::<Signal<AppConfig>>();
    let mut status = use_signal(|| ConnStatus::Connecting);
    use_effect(move || {
        // Subscribe to the active source (rebuilds on switch); `peek` the config
        // so a volume/theme change doesn't trigger a re-validation.
        let src = active_source.read().clone();
        if config.peek().active_source.is_local() {
            status.set(ConnStatus::Online);
            return;
        }
        status.set(ConnStatus::Connecting);
        spawn(async move {
            let outcome = utils::offload(async move { src.validate().await }).await;
            status.set(match outcome {
                AuthOutcome::Valid => ConnStatus::Online,
                AuthOutcome::Expired | AuthOutcome::Unreachable => ConnStatus::Offline,
            });
        });
    });
    use_memo(move || *status.read())
}

/// Apply a source switch.
///
/// The daemon does the work: for a server it loads that server's stored
/// credentials into the active snapshot, which a caller could not do through
/// `set_config` -- credential fields are exactly what that refuses to take.
/// Answers whether the source is usable without a sign-in (stored creds, or
/// anonymous YT), so the caller can launch a sign-in flow otherwise.
pub async fn apply_source_switch(mut config: Signal<AppConfig>, source: Source) -> bool {
    let api = crate::api::consume_api();
    match api.switch_source(source).await {
        Ok(usable) => {
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

/// A fire-and-forget source switcher for the sidebar: switches (loading creds)
/// without launching a sign-in flow — the Settings page owns that.
pub fn use_switch_source() -> impl Fn(Source) + Clone {
    let config = use_context::<Signal<AppConfig>>();
    move |source: Source| {
        spawn(async move {
            apply_source_switch(config, source).await;
        });
    }
}
