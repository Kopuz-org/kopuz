//! The one place a source switch happens, shared by the sidebar source switcher
//! and the Settings "Switch" button so they behave identically. A switch keeps
//! `config.active_source` and `config.server` (the active server's connection
//! snapshot, which the source resolver reads for the URL + creds) consistent —
//! both set in a single `config.write()` so the active `MediaSource` rebuilds
//! exactly once, with the new server, and never on a stale connection.

use config::{AppConfig, MusicServer, MusicService, Source};
use db::ReadDb;
use dioxus::prelude::*;

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
            let mut cfg = config.write();
            cfg.active_source = Source::Local;
            cfg.server = None;
            cfg.source_explicitly_set = true;
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
                cfg.active_source = Source::Server(saved.id);
                cfg.server = Some(active);
                cfg.source_explicitly_set = true;
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
