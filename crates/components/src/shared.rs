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

pub fn toggle_favorite(
    current_track: Option<reader::models::Track>,
    mut favorites_store: Signal<FavoritesStore>,
    config: Signal<config::AppConfig>,
) {
    if let Some(track) = current_track {
        if track.id.is_server() {
            let item_id = track.id.key().to_string();
            if !item_id.trim().is_empty() {
                let currently_fav = favorites_store.read().is_jellyfin_favorite(&item_id);
                let new_fav = !currently_fav;
                favorites_store
                    .write()
                    .set_jellyfin(item_id.clone(), new_fav);
                spawn(async move {
                    let conn = ::server::server_ops::ServerConn::resolve(&config.peek());
                    match conn {
                        Some(conn) => {
                            let result = ::server::server_ops::set_tracks_favorite(
                                &conn,
                                std::slice::from_ref(&item_id),
                                new_fav,
                            )
                            .await;
                            if let Err(e) = result {
                                tracing::warn!(error = %e, "failed to sync favorite to server");
                                favorites_store.write().set_jellyfin(item_id, !new_fav);
                            }
                        }
                        None => {
                            tracing::warn!("no server credentials, reverting favorite change");
                            favorites_store.write().set_jellyfin(item_id, !new_fav);
                        }
                    }
                });
            }
        } else if let Some(p) = track.id.local_path() {
            favorites_store.write().toggle_local(p.to_path_buf());
        }
    }
}
