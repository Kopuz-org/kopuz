//! The one place a source switch happens, shared by the sidebar source switcher
//! and the Settings "Switch" button so they behave identically. A switch keeps
//! `config.active_source` and `config.server` (the active server's connection
//! snapshot, which the source resolver reads for the URL + creds) consistent —
//! both set in a single `config.write()` so the active `MediaSource` rebuilds
//! exactly once, with the new server, and never on a stale connection.

use config::{AppConfig, MusicServer, MusicService, Source};
use db::ReadDb;
use dioxus::prelude::*;
use server::source::{ActiveSource, AuthOutcome};

/// Live connection status of the active source, for the switcher's indicator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnStatus {
    /// Verifying auth / reaching the server (the loading state).
    Connecting,
    /// Verified and reachable.
    Online,
    /// Credentials are missing or rejected; the user needs to sign in again.
    Expired,
    /// The remote source could not be reached (network/server).
    Unreachable,
}

/// Connection status of the active source: Local is always Online (no auth); a
/// server runs `validate()` on each switch — `Connecting` until it resolves to
/// `Online` (valid), `Expired` (auth missing/rejected), or `Unreachable`
/// (network/server).
pub fn use_connection_status() -> Memo<ConnStatus> {
    let active_source = use_context::<Signal<ActiveSource>>();
    let config = use_context::<Signal<AppConfig>>();
    let mut status = use_signal(|| ConnStatus::Connecting);
    use_effect(move || {
        // Subscribe to the active source (rebuilds on switch); `peek` the config
        // so a volume/theme change doesn't trigger a re-validation.
        let src = active_source.read().clone();
        if matches!(config.peek().active_source, Source::Local) {
            status.set(ConnStatus::Online);
            return;
        }
        status.set(ConnStatus::Connecting);
        spawn(async move {
            let outcome = utils::offload(async move { src.validate().await }).await;
            status.set(status_for(outcome));
        });
    });
    use_memo(move || *status.read())
}

/// Map a credential-validation outcome to the switcher's connection status.
/// `Expired` (missing/rejected creds) and `Unreachable` (network/server) are
/// kept distinct so the UI can prompt sign-in only when re-auth would help.
fn status_for(outcome: AuthOutcome) -> ConnStatus {
    match outcome {
        AuthOutcome::Valid => ConnStatus::Online,
        AuthOutcome::Expired => ConnStatus::Expired,
        AuthOutcome::Unreachable => ConnStatus::Unreachable,
    }
}

/// Apply a source switch. For a server it loads the stored creds from the DB (so
/// the connection is the new server's, not a leftover one) and writes
/// `active_source` and `server` together; for Local it clears the server snapshot.
/// Returns whether the source is usable without a sign-in (stored creds, or
/// anonymous YT), so the caller can launch a sign-in flow otherwise.
pub async fn apply_source_switch(
    mut config: Signal<AppConfig>,
    db: ReadDb,
    source: Source,
) -> bool {
    match source {
        Source::Local => {
            config.write().clear_active_server();
            tracing::info!(target: "kopuz::source", source = "local", "source switched");
            true
        }
        Source::Server(id) => {
            let Some(saved) = config.peek().find_saved_server(&id).cloned() else {
                return false;
            };
            let is_anon = saved.service == MusicService::YtMusic && saved.yt_anonymous;
            // Creds live with the server in the DB — reuse the stored token instead
            // of re-prompting sign-in on every switch.
            let stored = db.load_server(&saved.id).await.ok().flatten();
            let stored_token = stored.as_ref().and_then(|s| s.access_token.clone());
            let stored_user = stored.as_ref().and_then(|s| s.user_id.clone());
            let has_creds = stored_token.as_deref().is_some_and(|t| !t.is_empty());
            let active = MusicServer {
                name: saved.name,
                url: saved.url,
                service: saved.service,
                // Anonymous YT keeps an empty (non-None) token so the backend
                // treats it as anon rather than "needs sign-in".
                access_token: if is_anon {
                    Some(String::new())
                } else {
                    stored_token
                },
                user_id: stored_user,
                id: Some(saved.id.clone()),
                yt_browser: saved.yt_browser,
                yt_anonymous: is_anon,
            };
            {
                let mut cfg = config.write();
                cfg.set_active_server_snapshot(active);
            }
            tracing::info!(target: "kopuz::source", server = %id, "source switched");
            has_creds || is_anon
        }
    }
}

/// A fire-and-forget source switcher for the sidebar: switches (loading creds)
/// without launching a sign-in flow — the Settings page owns that.
pub fn use_switch_source() -> impl Fn(Source) + Clone {
    let config = use_context::<Signal<AppConfig>>();
    let db = use_context::<ReadDb>();
    move |source: Source| {
        let db = db.clone();
        spawn(async move {
            apply_source_switch(config, db, source).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_auth_outcomes_to_connection_statuses() {
        assert_eq!(status_for(AuthOutcome::Valid), ConnStatus::Online);
        assert_eq!(status_for(AuthOutcome::Expired), ConnStatus::Expired);
        assert_eq!(
            status_for(AuthOutcome::Unreachable),
            ConnStatus::Unreachable
        );
    }
}
