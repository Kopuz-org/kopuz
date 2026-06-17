use dioxus::prelude::*;

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
pub fn toggle_favorite(
    current_track: Option<reader::models::Track>,
    config: Signal<config::AppConfig>,
) {
    let Some(track) = current_track else { return };
    let ref_ = track.id.key().to_string();
    if ref_.trim().is_empty() {
        return;
    }
    let Some(db) = try_consume_context::<db::Db>() else {
        return;
    };
    let gens = try_consume_context::<hooks::db_reactivity::Generations>();
    let source = server::source::for_track(db, &config.peek(), &track);
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
