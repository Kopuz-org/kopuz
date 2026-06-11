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
    let db = try_consume_context::<db::Db>();
    let gens = try_consume_context::<hooks::db_reactivity::Generations>();

    if track.id.is_server() {
        let item_id = track.id.key().to_string();
        if item_id.trim().is_empty() {
            return;
        }
        let server_id = {
            let cfg = config.peek();
            cfg.active_server_id
                .clone()
                .or_else(|| cfg.server.as_ref().and_then(|s| s.id.clone()))
        };
        spawn(async move {
            if let (Some(db), Some(server_id)) = (&db, &server_id) {
                let new_fav = !db.is_favorite(server_id, &item_id).await.unwrap_or(false);
                if let Err(e) = db.set_favorite(server_id, &item_id, new_fav).await {
                    tracing::warn!(error = %e, "failed to record favorite toggle");
                }
                if let Some(gens) = gens {
                    gens.bump(hooks::db_reactivity::Table::Favorites);
                }
            }
            hooks::use_sync_task::nudge();
        });
    } else if let Some(p) = track.id.local_path() {
        let path = p.to_path_buf();
        spawn(async move {
            if let Some(db) = &db {
                let key = path.to_string_lossy().into_owned();
                let new_fav = !db.is_favorite("local", &key).await.unwrap_or(false);
                if let Err(e) = db.set_favorite("local", &key, new_fav).await {
                    tracing::warn!(error = %e, "failed to record local favorite toggle");
                }
                if let Some(gens) = gens {
                    gens.bump(hooks::db_reactivity::Table::Favorites);
                }
            }
        });
    }
}
