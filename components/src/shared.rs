use config::MusicService;
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
) {
    if let Some(track) = current_track {
        let path_str = track.path.to_string_lossy().to_string();
        let is_server_item = path_str.starts_with("jellyfin:")
            || path_str.starts_with("subsonic:")
            || path_str.starts_with("custom:");
        if is_server_item {
            let parts: Vec<String> = path_str.split(':').map(|s| s.to_string()).collect();
            if parts.len() >= 2 && !parts[1].trim().is_empty() {
                let item_id = parts[1].clone();
                let currently_fav = favorites_store.read().is_jellyfin_favorite(&item_id);
                let new_fav = !currently_fav;
                favorites_store.write().set_jellyfin(item_id.clone(), new_fav);
                spawn(async move {
                    let server_config = {
                        let conf = config.peek();
                        conf.server.as_ref().and_then(|server| {
                            if let (Some(token), Some(user_id)) =
                                (&server.access_token, &server.user_id)
                            {
                                Some((
                                    server.service,
                                    server.url.clone(),
                                    token.clone(),
                                    user_id.clone(),
                                    conf.device_id.clone(),
                                ))
                            } else {
                                None
                            }
                        })
                    };
                    if let Some((service, url, token, user_id, device_id)) = server_config {
                        let result = match service {
                            MusicService::Jellyfin => {
                                let remote = server::jellyfin::JellyfinClient::new(
                                    &url,
                                    Some(&token),
                                    &device_id,
                                    Some(&user_id),
                                );
                                if new_fav {
                                    remote.mark_favorite(&item_id).await
                                } else {
                                    remote.unmark_favorite(&item_id).await
                                }
                            }
                            MusicService::Subsonic | MusicService::Custom => {
                                let remote =
                                    server::subsonic::SubsonicClient::new(&url, &user_id, &token);
                                if new_fav {
                                    remote.star(&item_id).await
                                } else {
                                    remote.unstar(&item_id).await
                                }
                            }
                        };
                        if let Err(e) = result {
                            eprintln!("Failed to sync favorite to server: {e}");
                            favorites_store.write().set_jellyfin(item_id, !new_fav);
                        }
                    } else {
                        eprintln!("No server credentials, reverting favorite change");
                        favorites_store.write().set_jellyfin(item_id, !new_fav);
                    }
                });
            }
        } else {
            favorites_store.write().toggle_local(track.path.clone());
        }
    }
}
