use dioxus::prelude::*;

/// Make `source` the active source (the sidebar source switcher's action).
pub fn set_active_source(mut config: Signal<config::AppConfig>, source: config::Source) {
    let label = source.as_str().to_string();
    {
        let mut cfg = config.write();
        cfg.active_source = source;
        cfg.source_explicitly_set = true;
    }
    tracing::info!(target: "kopuz::source", source = %label, "source switched");
}

pub fn fmt_time(secs: u64) -> String {
    if secs == u64::MAX {
        return "--:--".to_string();
    }
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

/// Toggle a favorite, offline-capable: the DB records the change as a pending
/// row (`dirty`) and the background reconciler pushes it to the server when one
/// is reachable. No auth required at toggle time — anonymous likes queue up and
/// flush on sign-in.
pub fn toggle_favorite(current_track: Option<reader::models::Track>) {
    let Some(track) = current_track else { return };
    let ref_ = track.id.key().to_string();
    if ref_.trim().is_empty() {
        return;
    }
    let Some(source_sig) = try_consume_context::<Signal<::server::source::ActiveSource>>() else {
        return;
    };
    let gens = try_consume_context::<hooks::db_reactivity::Generations>();
    let source = source_sig.peek().clone();
    spawn(async move {
        let new_fav = !source.is_favorite(&ref_).await;
        if new_fav {
            // Cache the track so the favorites view (which resolves refs → the
            // `tracks` table) can display it immediately, instead of only after
            // the next sync upserts it. Harmless for a track already cached.
            let _ = source.upsert_tracks(std::slice::from_ref(&track)).await;
        }
        if let Err(e) = source.set_favorite(&ref_, new_fav).await {
            tracing::warn!(error = %e, "failed to record favorite toggle");
        }
        if let Some(gens) = gens {
            gens.bump(hooks::db_reactivity::Table::Favorites);
            gens.bump(hooks::db_reactivity::Table::Tracks);
        }
        // A like on a syncing source is a pending DB row until the reconciler
        // flushes it — nudge it. (Local has no remote to push to.)
        if source.capabilities().sync {
            hooks::use_sync_task::nudge();
        }
    });
}
