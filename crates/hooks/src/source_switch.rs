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

/// Connection status of the active source, as the daemon's probes last found it.
pub fn use_connection_status() -> Memo<ConnStatus> {
    let sources = crate::sources::use_sources();
    use_memo(move || {
        let sources = sources.read();
        let active = sources.iter().flatten().find(|source| source.active);
        match active.and_then(|source| source.state) {
            None | Some(api::SourceState::Checking) => ConnStatus::Connecting,
            Some(api::SourceState::Online) => ConnStatus::Online,
            Some(api::SourceState::AuthExpired | api::SourceState::Offline) => ConnStatus::Offline,
        }
    })
}

/// Apply a source switch. Answers whether the source is usable without a
/// sign-in (stored credentials, or a source usable anonymously), so the caller can
/// launch a sign-in flow otherwise.
pub async fn apply_source_switch(config: Signal<AppConfig>, id: String) -> bool {
    let api = crate::api::consume_api();
    let baseline = try_consume_context::<crate::config_sync::ConfigBaseline>();
    match api.switch_source(id).await {
        Ok(info) => {
            crate::sources::show_active(&info);
            let usable = info.authenticated;
            // Read back now rather than on the event, so a caller sees the switched config on return.
            if let (Some(baseline), Ok(view)) = (baseline, api.config().await) {
                baseline.adopt(config, &view);
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
