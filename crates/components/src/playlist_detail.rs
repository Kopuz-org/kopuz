use config::MusicService;
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};
#[cfg(not(target_arch = "wasm32"))]
use rfd::AsyncFileDialog;
use std::path::PathBuf;

#[component]
pub fn PlaylistDetail(
    playlist_id: String,
    mut playlist_store: Signal<PlaylistStore>,
    mut library: Signal<Library>,
    config: Signal<config::AppConfig>,
    on_close: EventHandler<()>,
    on_download_all: Option<EventHandler<()>>,
    on_delete_all: Option<EventHandler<()>>,
    on_download_track: Option<EventHandler<usize>>,
    #[props(default = false)] is_downloading_all: bool,
) -> Element {
    let store = playlist_store.read();
    let mut tracks = use_signal(Vec::<reader::models::Track>::new);
    let mut has_loaded_jellyfin_tracks = use_signal(|| false);

    let (playlist_name, local_tracks_paths, is_jellyfin, playlist_custom_cover, playlist_image_tag) =
        if let Some(p) = store.playlists.iter().find(|p| p.id == playlist_id) {
            (
                p.name.clone(),
                p.tracks.clone(),
                false,
                p.cover_path.clone(),
                None::<String>,
            )
        } else if let Some(p) = store
            .jellyfin_playlists
            .iter()
            .find(|p| p.id == playlist_id)
        {
            (
                p.name.clone(),
                vec![],
                true,
                p.cover_path.clone(),
                p.image_tag.clone(),
            )
        } else {
            return rsx! { div { "{i18n::t(\"playlist_not_found\")}" } };
        };

    let lib = library.read();

    if !is_jellyfin {
        let local_tracks: Vec<_> = local_tracks_paths
            .iter()
            .filter_map(|path| lib.tracks.iter().find(|t| t.path == *path).cloned())
            .collect();
        let local_tracks_for_effect = local_tracks.clone();
        use_effect(move || {
            tracks.set(local_tracks_for_effect.clone());
        });
    } else {
        let pid = playlist_id.clone();
        use_effect(move || {
            if !*has_loaded_jellyfin_tracks.read() {
                let pid_clone = pid.clone();
                spawn(async move {
                    let conf = config.peek();
                    if let Some(server) = &conf.server {
                        if let (Some(token), Some(user_id)) =
                            (&server.access_token, &server.user_id)
                        {
                            match server.service {
                                MusicService::Jellyfin => {
                                    let remote = server::jellyfin::JellyfinClient::new(
                                        &server.url,
                                        Some(token),
                                        &conf.device_id,
                                        Some(user_id),
                                    );
                                    if let Ok(items) = remote.get_playlist_items(&pid_clone).await {
                                        let mut new_tracks = Vec::new();
                                        for item in items {
                                            let duration_secs =
                                                item.run_time_ticks.unwrap_or(0) / 10_000_000;
                                            let mut path_str = format!("jellyfin:{}", item.id);
                                            if let Some(tags) = &item.image_tags {
                                                if let Some(tag) = tags.get("Primary") {
                                                    path_str.push_str(&format!(":{}", tag));
                                                }
                                            }
                                            let bitrate_kbps = item.bitrate.unwrap_or(0) / 1000;
                                            let bitrate_u16 = bitrate_kbps.min(u16::MAX as u32) as u16;
                                            let artist_str = item
                                                .album_artist
                                                .clone()
                                                .or_else(|| {
                                                    item.artists.as_ref().map(|a| a.join(", "))
                                                })
                                                .unwrap_or_default();
                                            new_tracks.push(reader::models::Track {
                                                path: PathBuf::from(path_str),
                                                album_id: item
                                                    .album_id
                                                    .map(|id| format!("jellyfin:{}", id))
                                                    .unwrap_or_default(),
                                                title: item.name,
                                                artist: artist_str,
                                                album: item.album.unwrap_or_default(),
                                                duration: duration_secs,
                                                khz: item.sample_rate.unwrap_or(0),
                                                bitrate: bitrate_u16,
                                                track_number: item.index_number,
                                                disc_number: item.parent_index_number,
                                                musicbrainz_release_id: None,
                                                playlist_item_id: item.playlist_item_id,
                                                artists: item.artists.unwrap_or_default(),
                                            });
                                        }
                                        tracks.set(new_tracks);
                                        has_loaded_jellyfin_tracks.set(true);
                                    }
                                }
                                MusicService::Subsonic | MusicService::Custom => {
                                    let remote = server::subsonic::SubsonicClient::new(
                                        &server.url,
                                        user_id,
                                        token,
                                    );
                                    if let Ok(items) = remote.get_playlist_entries(&pid_clone).await
                                    {
                                        let mut new_tracks = Vec::new();
                                        for item in items {
                                            let cover_tag = item
                                                .cover_art
                                                .as_ref()
                                                .and_then(|id| {
                                                    remote.cover_art_url(id, Some(512)).ok()
                                                })
                                                .map(|url| {
                                                    let mut hex =
                                                        String::with_capacity(url.len() * 2);
                                                    for b in url.as_bytes() {
                                                        hex.push_str(&format!("{:02x}", b));
                                                    }
                                                    format!("urlhex_{}", hex)
                                                });
                                            let path = if let Some(tag) = &cover_tag {
                                                PathBuf::from(format!(
                                                    "jellyfin:{}:{}",
                                                    item.id, tag
                                                ))
                                            } else {
                                                PathBuf::from(format!("jellyfin:{}", item.id))
                                            };
                                            let album_id = item
                                                .album_id
                                                .as_ref()
                                                .map(|id| {
                                                    if let Some(tag) = &cover_tag {
                                                        format!("jellyfin:{}:{}", id, tag)
                                                    } else {
                                                        format!("jellyfin:{}:none", id)
                                                    }
                                                })
                                                .unwrap_or_else(|| {
                                                    format!("jellyfin:{}:none", item.id)
                                                });
                                            new_tracks.push(reader::models::Track {
                                                path,
                                                album_id,
                                                title: item.title,
                                                artist: item.artist.clone().unwrap_or_default(),
                                                album: item.album.unwrap_or_default(),
                                                duration: item.duration.unwrap_or(0),
                                                khz: item.sampling_rate.unwrap_or(0),
                                                bitrate: item.bit_rate.unwrap_or(0).min(u16::MAX as u32) as u16,
                                                track_number: item.track,
                                                disc_number: item.disc_number,
                                                musicbrainz_release_id: None,
                                                playlist_item_id: None,
                                                artists: vec![item.artist.unwrap_or_default()],
                                            });
                                        }
                                        tracks.set(new_tracks);
                                        has_loaded_jellyfin_tracks.set(true);
                                    }
                                }
                            }
                        }
                    }
                });
            }
        });
    }

    let tracks_val = tracks.read().clone();

    let playlist_cover = if !is_jellyfin {
        playlist_custom_cover
            .as_ref()
            .and_then(|p| utils::format_artwork_url(Some(p)))
            .or_else(|| {
                tracks_val.first().and_then(|t| {
                    lib.albums
                        .iter()
                        .find(|a| a.id == t.album_id)
                        .and_then(|a| utils::format_artwork_url(a.cover_path.as_ref()))
                })
            })
    } else if let Some(server) = &config.read().server {
        if let Some(path) = &playlist_custom_cover {
            utils::format_artwork_url(Some(path))
        } else if let Some(tag) = &playlist_image_tag {
            Some(std::sync::Arc::from(
                utils::jellyfin_image::jellyfin_image_url(
                    &server.url,
                    &playlist_id,
                    Some(tag.as_str()),
                    server.access_token.as_deref(),
                    512,
                    90,
                )
                .as_str(),
            ))
        } else {
            tracks_val.first().and_then(|t| {
                let path_str = t.path.to_string_lossy();
                utils::jellyfin_image::jellyfin_image_url_from_path(
                    &path_str,
                    &server.url,
                    server.access_token.as_deref(),
                    512,
                    90,
                )
                .map(|s| std::sync::Arc::from(s.as_str()))
            })
        }
    } else {
        None
    };

    let pid_for_remove = playlist_id.clone();
    let pid_for_move_up = playlist_id.clone();
    let pid_for_move_down = playlist_id.clone();
    let pid_for_cover = playlist_id.clone();

    rsx! {
        crate::track_list_view::TrackListView {
            name: playlist_name.clone(),
            description: if is_jellyfin { i18n::t("server_playlist").to_string() } else { String::new() },
            cover_url: playlist_cover,
            back_label: i18n::t("back_to_playlists").to_string(),
            tracks: tracks_val,
            library,
            playlist_store,
            on_close,
            on_cover_click: move |_| {
                let _ = &pid_for_cover;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let pid = pid_for_cover.clone();
                    spawn(async move {
                        let file = AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await;
                        if let Some(file) = file {
                            let path = file.path().to_path_buf();
                            if is_jellyfin {
                                let conf = config.peek();
                                if let Some(server) = &conf.server {
                                    if let (Some(token), Some(user_id)) =
                                        (&server.access_token, &server.user_id)
                                    {
                                        if server.service == MusicService::Jellyfin {
                                            if let Ok(bytes) = std::fs::read(&path) {
                                                let ext = path
                                                    .extension()
                                                    .and_then(|e| e.to_str())
                                                    .unwrap_or("")
                                                    .to_lowercase();
                                                let ct =
                                                    if ext == "png" { "image/png" } else { "image/jpeg" };
                                                let remote = server::jellyfin::JellyfinClient::new(
                                                    &server.url,
                                                    Some(token),
                                                    &conf.device_id,
                                                    Some(user_id),
                                                );
                                                let _ =
                                                    remote.set_playlist_image(&pid, bytes, ct).await;
                                            }
                                        }
                                    }
                                }
                                let mut store = playlist_store.write();
                                if let Some(p) =
                                    store.jellyfin_playlists.iter_mut().find(|p| p.id == pid)
                                {
                                    p.cover_path = Some(path);
                                }
                            } else {
                                let mut store = playlist_store.write();
                                if let Some(p) = store.playlists.iter_mut().find(|p| p.id == pid) {
                                    p.cover_path = Some(path);
                                }
                            }
                        }
                    });
                }
            },
            on_delete_track: move |idx: usize| {
                if !is_jellyfin {
                    if let Some(t) = tracks.read().get(idx).cloned() {
                        #[cfg(not(target_arch = "wasm32"))]
                        if std::fs::remove_file(&t.path).is_ok() {
                            library.write().remove_track(&t.path);
                            let lib_path = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
                                .map(|d| d.config_dir().join("library.json"))
                                .unwrap_or_else(|| PathBuf::from("./config/library.json"));
                            let _ = library.read().save(&lib_path);
                        }
                    }
                }
            },
            on_selection_delete: move |paths: Vec<PathBuf>| {
                if !is_jellyfin {
                    #[cfg(not(target_arch = "wasm32"))]
                    for path in &paths {
                        if std::fs::remove_file(path).is_ok() {
                            library.write().remove_track(path);
                        }
                    }
                }
            },
            on_remove_from_playlist: move |idx: usize| {
                if let Some(t) = tracks.read().get(idx).cloned() {
                    if !is_jellyfin {
                        let mut store = playlist_store.write();
                        if let Some(playlist) =
                            store.playlists.iter_mut().find(|p| p.id == pid_for_remove)
                        {
                            playlist.tracks.retain(|p| p != &t.path);
                        }
                    } else {
                        let pid_clone = pid_for_remove.clone();
                        let entry_id_opt = t.playlist_item_id.clone();
                        let remove_idx = idx;
                        spawn(async move {
                            let conf = config.peek();
                            if let Some(server) = &conf.server {
                                if let (Some(token), Some(user_id)) =
                                    (&server.access_token, &server.user_id)
                                {
                                    let removed = match server.service {
                                        MusicService::Jellyfin => {
                                            if let Some(entry_id) = entry_id_opt {
                                                let remote = server::jellyfin::JellyfinClient::new(
                                                    &server.url,
                                                    Some(token),
                                                    &conf.device_id,
                                                    Some(user_id),
                                                );
                                                remote
                                                    .remove_from_playlist(&pid_clone, &entry_id)
                                                    .await
                                                    .is_ok()
                                            } else {
                                                false
                                            }
                                        }
                                        MusicService::Subsonic | MusicService::Custom => {
                                            let remote = server::subsonic::SubsonicClient::new(
                                                &server.url,
                                                user_id,
                                                token,
                                            );
                                            remote
                                                .remove_from_playlist(&pid_clone, remove_idx)
                                                .await
                                                .is_ok()
                                        }
                                    };
                                    if removed {
                                        let mut tw = tracks.write();
                                        if remove_idx < tw.len() {
                                            tw.remove(remove_idx);
                                        }
                                    }
                                }
                            }
                        });
                    }
                }
            },
            is_reorderable: true,
            on_move_up: move |idx: usize| {
                if idx == 0 { return; }
                tracks.write().swap(idx - 1, idx);
                if !is_jellyfin {
                    let mut store = playlist_store.write();
                    if let Some(pl) =
                        store.playlists.iter_mut().find(|p| p.id == pid_for_move_up)
                    {
                        pl.tracks.swap(idx - 1, idx);
                    }
                } else {
                    let track_list = tracks.read().clone();
                    let pid = pid_for_move_up.clone();
                    spawn(async move {
                        let conf = config.peek();
                        if let Some(server) = &conf.server {
                            if let (Some(token), Some(user_id)) =
                                (&server.access_token, &server.user_id)
                            {
                                let moved_item =
                                    track_list.get(idx - 1).and_then(|t| t.playlist_item_id.clone());
                                match server.service {
                                    MusicService::Jellyfin => {
                                        if let Some(item_id) = moved_item {
                                            let remote = server::jellyfin::JellyfinClient::new(
                                                &server.url,
                                                Some(token),
                                                &conf.device_id,
                                                Some(user_id),
                                            );
                                            let _ = remote
                                                .move_playlist_item(&pid, &item_id, idx - 1)
                                                .await;
                                        }
                                    }
                                    MusicService::Subsonic | MusicService::Custom => {
                                        let remote = server::subsonic::SubsonicClient::new(
                                            &server.url,
                                            user_id,
                                            token,
                                        );
                                        let ids: Vec<String> = track_list
                                            .iter()
                                            .filter_map(|t| {
                                                let s = t.path.to_string_lossy();
                                                let parts: Vec<&str> = s.split(':').collect();
                                                if parts.len() >= 2 {
                                                    Some(parts[1].to_string())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();
                                        let id_refs: Vec<&str> =
                                            ids.iter().map(|s| s.as_str()).collect();
                                        let _ = remote
                                            .reorder_playlist(&pid, &id_refs, id_refs.len())
                                            .await;
                                    }
                                }
                            }
                        }
                    });
                }
            },
            on_move_down: move |idx: usize| {
                let len = tracks.read().len();
                if idx + 1 >= len { return; }
                tracks.write().swap(idx, idx + 1);
                if !is_jellyfin {
                    let mut store = playlist_store.write();
                    if let Some(pl) =
                        store.playlists.iter_mut().find(|p| p.id == pid_for_move_down)
                    {
                        pl.tracks.swap(idx, idx + 1);
                    }
                } else {
                    let track_list = tracks.read().clone();
                    let pid = pid_for_move_down.clone();
                    spawn(async move {
                        let conf = config.peek();
                        if let Some(server) = &conf.server {
                            if let (Some(token), Some(user_id)) =
                                (&server.access_token, &server.user_id)
                            {
                                let moved_item =
                                    track_list.get(idx + 1).and_then(|t| t.playlist_item_id.clone());
                                match server.service {
                                    MusicService::Jellyfin => {
                                        if let Some(item_id) = moved_item {
                                            let remote = server::jellyfin::JellyfinClient::new(
                                                &server.url,
                                                Some(token),
                                                &conf.device_id,
                                                Some(user_id),
                                            );
                                            let _ = remote
                                                .move_playlist_item(&pid, &item_id, idx + 1)
                                                .await;
                                        }
                                    }
                                    MusicService::Subsonic | MusicService::Custom => {
                                        let remote = server::subsonic::SubsonicClient::new(
                                            &server.url,
                                            user_id,
                                            token,
                                        );
                                        let ids: Vec<String> = track_list
                                            .iter()
                                            .filter_map(|t| {
                                                let s = t.path.to_string_lossy();
                                                let parts: Vec<&str> = s.split(':').collect();
                                                if parts.len() >= 2 {
                                                    Some(parts[1].to_string())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();
                                        let id_refs: Vec<&str> =
                                            ids.iter().map(|s| s.as_str()).collect();
                                        let _ = remote
                                            .reorder_playlist(&pid, &id_refs, id_refs.len())
                                            .await;
                                    }
                                }
                            }
                        }
                    });
                }
            },
            on_download_all: if is_jellyfin { on_download_all } else { None },
            on_download_track: if is_jellyfin { on_download_track } else { None },
            on_delete_all: if is_jellyfin { on_delete_all } else { None },
            is_downloading_all,
            show_delete_in_selection: !is_jellyfin,
        }
    }
}
