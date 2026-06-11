use dioxus::prelude::*;
use reader::FavoritesStore;

pub fn fmt_time(secs: u64) -> String {
    if secs == u64::MAX {
        return "--:--".to_string();
    }
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

pub fn get_favorite(
    current_track: Option<&reader::models::Track>,
    favorites_store: &Signal<FavoritesStore>,
) -> bool {
    if let Some(track) = current_track {
        if track.id.is_server() {
            let item_id = track.id.key();
            !item_id.trim().is_empty() && favorites_store.read().is_jellyfin_favorite(&item_id)
        } else if let Some(p) = track.id.local_path() {
            favorites_store.read().is_local_favorite(p)
        } else {
            false
        }
    } else {
        false
    }
}

/// Toggle a favorite, optimistically and offline-capable: the in-memory store
/// updates instantly, the DB records the change as a pending row (`dirty`), and
/// the background reconciler pushes it to the server when one is reachable. No
/// auth required at toggle time — anonymous likes queue up and flush on sign-in.
pub fn toggle_favorite(
    current_track: Option<reader::models::Track>,
    mut favorites_store: Signal<FavoritesStore>,
    config: Signal<config::AppConfig>,
) {
    let Some(track) = current_track else { return };
    let db = try_consume_context::<db::Db>();

    // Ordering matters: the pending-row write must COMMIT before the signal
    // mutation arms the favorites save effect. The save effect's
    // replace_favorites_clean deletes clean rows absent from memory — if it ran
    // first, an unlike's clean row would be deleted before set_favorite could
    // tombstone it (`UPDATE … WHERE dirty = 0` matches nothing), and the next
    // sync pull would silently resurrect the favorite. The DB write is a local
    // 1-row upsert, so the UI update lands a few ms later at most.
    if track.id.is_server() {
        let item_id = track.id.key().to_string();
        if item_id.trim().is_empty() {
            return;
        }
        let currently_fav = favorites_store.read().is_jellyfin_favorite(&item_id);
        let new_fav = !currently_fav;

        let server_id = {
            let cfg = config.peek();
            cfg.active_server_id
                .clone()
                .or_else(|| cfg.server.as_ref().and_then(|s| s.id.clone()))
        };
        spawn(async move {
            if let (Some(db), Some(server_id)) = (&db, &server_id) {
                if let Err(e) = db.set_favorite(server_id, &item_id, new_fav).await {
                    tracing::warn!(error = %e, "failed to record favorite toggle");
                }
            }
            favorites_store
                .write()
                .set_jellyfin(item_id.clone(), new_fav);
            hooks::use_sync_task::nudge();
        });
    } else if let Some(p) = track.id.local_path() {
        let path = p.to_path_buf();
        let now_fav = !favorites_store.read().is_local_favorite(&path);
        spawn(async move {
            if let Some(db) = &db {
                let key = path.to_string_lossy().into_owned();
                if let Err(e) = db.set_favorite("local", &key, now_fav).await {
                    tracing::warn!(error = %e, "failed to record local favorite toggle");
                }
            }
            favorites_store.write().toggle_local(path.clone());
        });
    }
}
