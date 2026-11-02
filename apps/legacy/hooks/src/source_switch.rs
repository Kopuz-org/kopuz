//! Switching what the app plays from, and whether that source is reachable.
//!
//! The daemon owns the sources: a switch loads that server's stored
//! credentials into the active snapshot, which no settings edit can reach.

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
pub async fn apply_source_switch(id: String) -> bool {
    let api = crate::api::consume_api();
    let handle = try_consume_context::<crate::settings::SettingsHandle>();
    match api.switch_source(id).await {
        Ok(info) => {
            crate::sources::show_active(&info);
            crate::ui_prefs::mark_source_chosen();
            let usable = info.authenticated;
            // Read back now rather than on the event, so a caller sees the switched config on return.
            if let (Some(handle), Ok(view)) = (handle, api.config().await) {
                handle.adopt(view);
            }
            usable
        }
        Err(error) => {
            tracing::warn!(%error, "source switch failed");
            crate::toast::toast_api_error(&error);
            false
        }
    }
}

/// A fire-and-forget source switcher for the sidebar: switches (loading
/// credentials) without launching a sign-in flow -- the settings page owns that.
pub fn use_switch_source() -> impl Fn(String) + Clone {
    move |id: String| {
        spawn(async move {
            apply_source_switch(id).await;
        });
    }
}
