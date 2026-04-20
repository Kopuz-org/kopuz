use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::stat_card::StatCard;
use components::track_row::TrackRow;
use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use reader::Library;
use std::collections::HashSet;
use std::path::PathBuf;

const ITEM_HEIGHT: f64 = 64.0; // 60px content + 4px margin (mb-1)
#[component]
pub fn JellyfinLibrary(
    mut library: Signal<Library>,
    mut config: Signal<AppConfig>,
    playlist_store: Signal<reader::PlaylistStore>,
    mut queue: Signal<Vec<reader::models::Track>>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut is_loading = use_signal(|| false);
    let mut has_fetched = use_signal(|| false);
    let mut fetch_generation = use_signal(|| 0usize);
    let mut sort_order = use_signal(|| config.peek().sort_order.clone());
    let mut scroll_stat = use_signal(|| 0.0);
    use_effect(move || {
        let curr = sort_order.read().clone();
        if config.peek().sort_order != curr {
            config.write().sort_order = curr;
        }
    });

    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());

    let mut fetch_jellyfin = move || {
        has_fetched.set(true);
        is_loading.set(true);
        fetch_generation.with_mut(|g| *g += 1);
        let current_gen = *fetch_generation.peek();
        {
            let mut lib_write = library.write();
            lib_write.jellyfin_tracks.clear();
            lib_write.jellyfin_albums.clear();
        }
        spawn(async move {
            if *fetch_generation.read() == current_gen {
                let _ =
                    crate::server::subsonic_sync::sync_server_library(library, config, true).await;
                if *fetch_generation.read() == current_gen {
                    is_loading.set(false);
                }
            }
        });
    };

    use_effect(move || {
        if !*has_fetched.read() {
            if library.read().jellyfin_tracks.is_empty() {
                fetch_jellyfin();
            } else {
                has_fetched.set(true);
            }
        }
    });

    let displayed_tracks = use_memo(move || {
        let mut tracks = library.read().jellyfin_tracks.clone();
        match *sort_order.read() {
            config::SortOrder::Title => tracks.sort_by_cached_key(|a| {
                (
                    a.title.to_lowercase(),
                    a.artist.to_lowercase(),
                    a.album.to_lowercase(),
                    a.disc_number,
                    a.track_number,
                )
            }),
            config::SortOrder::Artist => tracks.sort_by_cached_key(|a| {
                (
                    a.artist.to_lowercase(),
                    a.album.to_lowercase(),
                    a.disc_number,
                    a.track_number,
                    a.title.to_lowercase(),
                )
            }),
            config::SortOrder::Album => tracks.sort_by_cached_key(|a| {
                (
                    a.album.to_lowercase(),
                    a.disc_number,
                    a.track_number,
                    a.title.to_lowercase(),
                )
            }),
        }
        let conf = config.read();
        tracks
            .into_iter()
            .map(|t| {
                let cover_url = if let Some(server) = &conf.server {
                    let path_str = t.path.to_string_lossy();
                    utils::jellyfin_image::track_cover_url_with_album_fallback(
                        &path_str,
                        &t.album_id,
                        &server.url,
                        server.access_token.as_deref(),
                        80,
                        80,
                    )
                } else {
                    None
                };
                (t, cover_url)
            })
            .collect::<Vec<_>>()
    });

    let queue_tracks = use_memo(move || {
        displayed_tracks()
            .iter()
            .map(|(t, _)| t.clone())
            .collect::<Vec<_>>()
    });

    let is_empty = displayed_tracks().is_empty();
    let queue_source = std::sync::Arc::new(queue_tracks());
    let mut container_height = use_signal(|| 800.0);
    let scroll_top = *scroll_stat.read();
    let row_height = ITEM_HEIGHT;
    let window_size = (*container_height.read() / row_height).ceil() as usize;
    let buffer_size = 10;
    let total_tracks = displayed_tracks().len();

    let start_index = {
        let calc = (scroll_top - (buffer_size as f64) * row_height) / row_height;
        calc.floor().max(0.0) as usize
    };

    let end_index = {
        let last_index = start_index + 2 * buffer_size + window_size;
        let last_index_inclusive = last_index.saturating_sub(1);
        if total_tracks == 0 { 0 } else { last_index_inclusive.min(total_tracks - 1) }
    };

    let items_to_render = if total_tracks == 0 { 0 } else { (end_index + 1).saturating_sub(start_index) };
    
    let top_pad = (start_index as f64) * row_height;
    
    let bottom_pad = {
        let total_height = (total_tracks as f64) * row_height;
        let rendered_height = (items_to_render as f64) * row_height;
        (total_height - rendered_height - top_pad).max(0.0)
    };

    let tracks_nodes =
        displayed_tracks()
            .into_iter()
            .enumerate()
            .skip(start_index)
            .take(items_to_render)
            .map(|(idx, (track, cover_url))| {
                let track_menu = track.clone();
                let track_add = track.clone();
                let track_path = track.path.clone();
                let track_select = track.path.clone();
                let queue_arc = std::sync::Arc::clone(&queue_source);
                let track_key = format!("{}-{}", track.path.display(), idx);
                let is_menu_open = active_menu_track.read().as_ref() == Some(&track.path);
                let is_selected = selected_tracks.read().contains(&track_path);
                rsx! {
                    div {
                        key: "{track_key}",
                        class: "mb-1",
                        style: "height: {ITEM_HEIGHT}px;",
                    TrackRow {
                        track: track.clone(),
                        cover_url: cover_url.clone(),
                        is_menu_open,
                        is_selection_mode: is_selection_mode(),
                        is_selected,
                        on_long_press: move |_| {
                            is_selection_mode.set(true);
                            selected_tracks.write().insert(track_path.clone());
                        },
                        on_select: move |selected| {
                            if selected {
                                selected_tracks.write().insert(track_select.clone());
                            } else {
                                selected_tracks.write().remove(&track_select);
                                if selected_tracks.read().is_empty() {
                                    is_selection_mode.set(false);
                                }
                            }
                        },
                        on_click_menu: move |_| {
                            if active_menu_track.read().as_ref() == Some(&track_menu.path) {
                                active_menu_track.set(None);
                            } else {
                                active_menu_track.set(Some(track_menu.path.clone()));
                            }
                        },
                        on_add_to_playlist: move |_| {
                            selected_track_for_playlist.set(Some(track_add.path.clone()));
                            show_playlist_modal.set(true);
                            active_menu_track.set(None);
                        },
                        on_close_menu: move |_| active_menu_track.set(None),
                        on_delete: move |_| active_menu_track.set(None),
                        hide_delete: true,
                        on_play: move |_| {
                            queue.set((*queue_arc).clone());
                            ctrl.play_track(idx);
                        },
                    }
                }
                }
            });

    rsx! {
        div {
            class: "p-8 relative h-full flex flex-col overflow-hidden",

            if *show_playlist_modal.read() {
                PlaylistModal {
                    playlist_store,
                    is_jellyfin: true,
                    on_close: move |_| {
                        show_playlist_modal.set(false);
                        if is_selection_mode() {
                            is_selection_mode.set(false);
                            selected_tracks.write().clear();
                        }
                    },
                    on_add_to_playlist: move |playlist_id: String| {
                        let mut selected_paths = Vec::new();
                        if is_selection_mode() {
                            selected_paths = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            selected_paths.push(path);
                        }

                        if !selected_paths.is_empty() {
                            let pid = playlist_id.clone();
                            spawn(async move {
                                let conf = config.peek();
                                if let Some(server) = &conf.server {
                                    if let (Some(token), Some(user_id)) =
                                        (&server.access_token, &server.user_id)
                                    {
                                        match server.service {
                                            MusicService::Jellyfin => {
                                                let remote = JellyfinClient::new(
                                                    &server.url,
                                                    Some(token),
                                                    &conf.device_id,
                                                    Some(user_id),
                                                );
                                                for path in selected_paths {
                                                    let parts: Vec<&str> = path
                                                        .to_str()
                                                        .unwrap_or_default()
                                                        .split(':')
                                                        .collect();
                                                    if parts.len() >= 2 {
                                                        let item_id = parts[1];
                                                        let _ = remote.add_to_playlist(&pid, item_id).await;
                                                    }
                                                }
                                            }
                                            MusicService::Subsonic | MusicService::Custom => {
                                                let remote =
                                                    SubsonicClient::new(&server.url, user_id, token);
                                                for path in selected_paths {
                                                    let parts: Vec<&str> = path
                                                        .to_str()
                                                        .unwrap_or_default()
                                                        .split(':')
                                                        .collect();
                                                    if parts.len() >= 2 {
                                                        let item_id = parts[1];
                                                        let _ = remote.add_to_playlist(&pid, item_id).await;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_create_playlist: move |name: String| {
                        let mut selected_paths = Vec::new();
                        if is_selection_mode() {
                            selected_paths = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            selected_paths.push(path);
                        }

                        if !selected_paths.is_empty() {
                            let playlist_name = name.clone();
                            spawn(async move {
                                let conf = config.peek();
                                if let Some(server) = &conf.server {
                                    if let (Some(token), Some(user_id)) =
                                        (&server.access_token, &server.user_id)
                                    {
                                        let item_ids: Vec<String> = selected_paths
                                            .iter()
                                            .filter_map(|p| {
                                                let parts: Vec<&str> = p.to_str()?.split(':').collect();
                                                if parts.len() >= 2 {
                                                    Some(parts[1].to_string())
                                                } else {
                                                    None
                                                }
                                            })
                                            .collect();

                                        if !item_ids.is_empty() {
                                            let item_id_refs: Vec<&str> = item_ids.iter().map(|s| s.as_str()).collect();
                                            match server.service {
                                                MusicService::Jellyfin => {
                                                    let remote = JellyfinClient::new(
                                                        &server.url,
                                                        Some(token),
                                                        &conf.device_id,
                                                        Some(user_id),
                                                    );
                                                    let _ = remote
                                                        .create_playlist(&playlist_name, &item_id_refs)
                                                        .await;
                                                }
                                                MusicService::Subsonic | MusicService::Custom => {
                                                    let remote =
                                                        SubsonicClient::new(&server.url, user_id, token);
                                                    let _ = remote
                                                        .create_playlist(&playlist_name, &item_id_refs)
                                                        .await;
                                                }
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                }
            }

            if is_selection_mode() {
                SelectionBar {
                    count: selected_tracks.read().len(),
                    show_delete: false,
                    on_add_to_playlist: move |_| {
                        show_playlist_modal.set(true);
                    },
                    on_delete: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_cancel: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    }
                }
            }

            div {
                class: "flex items-center justify-between mb-6",
                h1 { class: "text-3xl font-bold text-white", "{rust_i18n::t!(\"your_library\")}" }
                button {
                    class: "text-white/60 hover:text-white transition-colors p-2 rounded-full hover:bg-white/10",
                    title: rust_i18n::t!("refresh_music_library").to_string(),
                    onclick: move |_| fetch_jellyfin(),
                    i { class: "fa-solid fa-rotate" }
                }
            }

            div {
                class: "grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-12",
                {
                    let lib = library.read();
                    let (artist_count, album_count) = {
                        let mut artists = HashSet::new();
                        let mut album_titles = HashSet::new();
                        for album in &lib.jellyfin_albums {
                            artists.insert(&album.artist);
                            album_titles.insert(album.title.to_lowercase());
                        }
                        for track in &lib.jellyfin_tracks { artists.insert(&track.artist); }
                        (artists.len(), album_titles.len())
                    };
                    rsx! {
                        StatCard { label: rust_i18n::t!("tracks").to_string(),    value: "{lib.jellyfin_tracks.len()}",  icon: "fa-music" }
                        StatCard { label: rust_i18n::t!("albums").to_string(),    value: "{album_count}",                icon: "fa-compact-disc" }
                        StatCard { label: rust_i18n::t!("artists").to_string(),   value: "{artist_count}",               icon: "fa-user" }
                        StatCard { label: rust_i18n::t!("playlists").to_string(), value: "{playlist_store.read().jellyfin_playlists.len()}", icon: "fa-list" }
                    }
                }
            }

            div {
                class: "flex items-center justify-between mb-4",
                h2 { class: "text-xl font-semibold text-white/80", "{rust_i18n::t!(\"tracks\")}" }
                div {
                    class: "flex space-x-1 bg-white/5 border border-white/5 p-1 rounded-lg",
                    button {
                        class: if *sort_order.read() == config::SortOrder::Title {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Title),
                        "Title"
                    }
                    button {
                        class: if *sort_order.read() == config::SortOrder::Artist {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Artist),
                        "Artist"
                    }
                    button {
                        class: if *sort_order.read() == config::SortOrder::Album {
                            "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all"
                        } else {
                            "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all"
                        },
                        onclick: move |_| sort_order.set(config::SortOrder::Album),
                        "Album"
                    }
                }
            }

            div {
                class: "flex-1 overflow-y-auto pb-20",
                onmounted: move |event| {
                    spawn(async move {
                        if let Ok(window) = event.get_client_rect().await {
                            container_height.set(window.height());
                        }
                    });
                },
                onscroll: move |event| {
                    let scroll_y = event.scroll_top();
                    let height = event.client_height() as f64;
                    scroll_stat.set(scroll_y);
                    container_height.set(height);
                },
                if is_empty {
                    if *is_loading.read() {
                        div { class: "flex items-center justify-center py-12",
                            i { class: "fa-solid fa-spinner fa-spin text-3xl text-white/20" }
                        }
                    } else {
                        p { class: "text-slate-500 italic", "{rust_i18n::t!(\"no_tracks_found\")}" }
                    }
                } else {
                    div { style: "height: {top_pad}px; flex-shrink: 0;" }
                    {tracks_nodes}
                    div { style: "height: {bottom_pad}px; flex-shrink: 0;" }
                    if *is_loading.read() {
                        div { class: "flex items-center justify-center py-4",
                            i { class: "fa-solid fa-spinner fa-spin text-xl text-white/20" }
                        }
                    }
                }
            }
        }
    }
}

pub use JellyfinLibrary as ServerLibrary;
