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
        let path_str = track.path.to_string_lossy();
        if path_str.starts_with("jellyfin:")
            || path_str.starts_with("subsonic:")
            || path_str.starts_with("custom:")
            || path_str.starts_with("ytmusic:")
            || path_str.starts_with("soundcloud:")
        {
            let parts: Vec<&str> = path_str.split(':').collect();
            if parts.len() >= 2 && !parts[1].trim().is_empty() {
                favorites_store.read().is_jellyfin_favorite(parts[1])
            } else {
                false
            }
        } else {
            favorites_store.read().is_local_favorite(&track.path)
        }
    } else {
        false
    }
}

pub fn toggle_favorite(
    current_track: Option<reader::models::Track>,
    mut favorites_store: Signal<FavoritesStore>,
    config: Signal<config::AppConfig>,
    mut playback_error: Signal<Option<String>>,
) {
    if let Some(track) = current_track {
        let path_str = track.path.to_string_lossy().to_string();
        let is_soundcloud = path_str.starts_with("soundcloud:");
        let is_server_item = path_str.starts_with("jellyfin:")
            || path_str.starts_with("subsonic:")
            || path_str.starts_with("custom:")
            || path_str.starts_with("ytmusic:")
            || is_soundcloud;
        if is_server_item {
            let parts: Vec<String> = path_str.split(':').map(|s| s.to_string()).collect();
            if parts.len() >= 2 && !parts[1].trim().is_empty() {
                let item_id = parts[1].clone();
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
                                if is_soundcloud {
                                    playback_error.set(Some(e));
                                }
                            }
                        }
                        None => {
                            tracing::warn!("no server credentials, reverting favorite change");
                            favorites_store.write().set_jellyfin(item_id, !new_fav);
                        }
                    }
                });
            }
        } else {
            favorites_store.write().toggle_local(track.path.clone());
        }
    }
}
