use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};
use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::showcase::{self, SortField};
use components::track_row::TrackRow;
use config::{AppConfig, MusicService, UiStyle};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use reader::{FavoritesStore, Library, PlaylistStore};
use std::collections::HashSet;
use std::path::PathBuf;
use tracing::Instrument;

#[component]
pub fn JellyfinFavorites(
    favorites_store: Signal<FavoritesStore>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    playlist_store: Signal<PlaylistStore>,
    mut queue: Signal<Vec<reader::models::Track>>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    // YT sync state:
    // - `is_syncing`: true while a fetch is in flight
    // - `synced_so_far`: count of tracks streamed into the library so far
    // - `refresh_nonce`: bumped by the manual refresh button to force a
    //   re-sync even when the library already has data on disk
    let mut is_syncing = use_signal(|| false);
    let mut synced_so_far: Signal<usize> = use_signal(|| 0);
    let mut refresh_nonce: Signal<u64> = use_signal(|| 0);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());
    let sort_state = use_signal(|| None);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);
    let download_queue = use_context::<Signal<DownloadQueue>>();

    use_effect(move || {
        let nonce = *refresh_nonce.read();

        let token = match config.peek().server.as_ref().and_then(|s| s.access_token.clone()) {
            Some(t) => t,
            None => return,
        };
        let service = config.peek().server.as_ref().map(|s| s.service);
        let is_ytmusic = service == Some(MusicService::YtMusic);

        if is_ytmusic && nonce == 0 && library.peek().last_yt_sync_at.is_some() {
            return;
        }

        is_syncing.set(true);
        synced_so_far.set(0);
        spawn(async move {
            let device_id = config.peek().device_id.clone();
            let server_snapshot = config.peek().server.clone();
            let Some(server) = server_snapshot else {
                is_syncing.set(false);
                return;
            };
            let user_id = server.user_id.clone().unwrap_or_default();
            let url = server.url.clone();

            let ids: Vec<String> = match server.service {
                MusicService::Jellyfin => {
                    let remote =
                        JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                    remote
                        .get_favorite_items()
                        .await
                        .map(|items| items.into_iter().map(|i| i.id).collect())
                        .unwrap_or_default()
                }
                MusicService::Subsonic | MusicService::Custom => {
                    let remote = SubsonicClient::new(&url, &user_id, &token);
                    remote.get_starred_song_ids().await.unwrap_or_default()
                }
                MusicService::YtMusic => {
                    let yt =
                        ::server::ytmusic::YouTubeMusicClient::with_cookies(token);

                    {
                        let mut lib = library.write();
                        lib.jellyfin_tracks.clear();
                        lib.jellyfin_albums.clear();
                    }

                    let mut accumulated: Vec<reader::models::Track> = Vec::new();
                    let result = yt
                        .stream_liked_songs(|page| {
                            accumulated.extend(page.iter().cloned());
                            let albums = synthesize_albums(&accumulated);
                            {
                                let mut lib = library.write();
                                lib.jellyfin_tracks.extend(page);
                                lib.jellyfin_albums = albums;
                            }
                            synced_so_far.set(accumulated.len());
                        })
                        .await;

                    match result {
                        Ok(()) => {
                            let ids: Vec<String> = accumulated
                                .iter()
                                .filter_map(|t| {
                                    t.path
                                        .to_string_lossy()
                                        .split(':')
                                        .nth(1)
                                        .map(|s| s.to_string())
                                })
                                .collect();
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            library.write().last_yt_sync_at = Some(now);
                            let liked_cover = accumulated
                                .first()
                                .and_then(|t| {
                                    t.path
                                        .to_string_lossy()
                                        .split(':')
                                        .nth(2)
                                        .filter(|s| s.starts_with("urlhex_"))
                                        .map(|s| s.to_string())
                                });
                            let liked_entry = reader::models::JellyfinPlaylist {
                                id: "LM".to_string(),
                                name: "Liked Songs".to_string(),
                                tracks: ids.clone(),
                                image_tag: liked_cover,
                                cover_path: None,
                            };
                            {
                                let mut ps = playlist_store.write();
                                if let Some(existing) = ps
                                    .jellyfin_playlists
                                    .iter_mut()
                                    .find(|p| p.id == "LM")
                                {
                                    *existing = liked_entry;
                                } else {
                                    ps.jellyfin_playlists.insert(0, liked_entry);
                                }
                            }
                            ids
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "YT favorites sync failed");
                            Vec::new()
                        }
                    }
                }
            };

            favorites_store.write().jellyfin_favorites = ids;
            is_syncing.set(false);
        }.instrument(tracing::info_span!("favorites.sync")));
    });

    let displayed_tracks: Vec<(reader::models::Track, Option<utils::CoverUrl>)> = {
        let store = favorites_store.read();
        let lib = library.read();
        let server = config.read();
        let server_ref = server.server.as_ref().cloned();

        let fav_set: std::collections::HashSet<&str> = store
            .jellyfin_favorites
            .iter()
            .map(|s| s.as_str())
            .collect();
        lib.jellyfin_tracks
            .iter()
            .filter(|t| {
                let path_str = t.path.to_string_lossy();
                path_str
                    .split(':')
                    .nth(1)
                    .map(|id| fav_set.contains(id))
                    .unwrap_or(false)
            })
            .map(|t| {
                let cover_url = if let Some(ref srv) = server_ref {
                    let path_str = t.path.to_string_lossy();
                    utils::map_cover_url(
                        utils::jellyfin_image::track_cover_url_with_album_fallback(
                            &path_str,
                            &t.album_id,
                            &srv.url,
                            srv.access_token.as_deref(),
                            80,
                            80,
                        ),
                    )
                } else {
                    None
                };
                (t.clone(), cover_url)
            })
            .collect()
    };

    let sorted_displayed_tracks =
        showcase::sorted_track_pairs(&displayed_tracks, *sort_state.read());

    let queue_tracks: Vec<reader::models::Track> = sorted_displayed_tracks
        .iter()
        .map(|(t, _)| t.clone())
        .collect();

    let currently_playing_path = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|track| track.path.clone())
    };

    let displayed_tracks_for_selection = sorted_displayed_tracks.clone();
    let is_empty = displayed_tracks.is_empty();
    let is_modern = config.read().ui_style == UiStyle::Modern;

    let tracks_nodes = sorted_displayed_tracks
        .iter()
        .cloned()
        .enumerate()
        .map(|(idx, (track, cover_url))| {
            let track_menu = track.clone();
            let track_path = track.path.clone();
            let track_select = track.path.clone();
            let track_add = track.clone();
            let track_queue = track.clone();
            let queue_source = queue_tracks.clone();
            let track_key = format!("{}-{}", track.path.display(), idx);
            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.path);
            let is_selected = selected_tracks.read().contains(&track_path);
            let matches_current_path = currently_playing_path.as_ref() == Some(&track.path);

            let path_str = track.path.to_string_lossy().to_string();
            let item_id: String = path_str.split(':').nth(1).unwrap_or("").to_string();
            let is_downloaded = if let Some(path_str) = config.read().offline_tracks.get(&item_id) {
                std::path::Path::new(path_str).exists()
            } else {
                false
            };
            let is_downloading = download_queue.read().items.iter().any(|i| i.id == item_id && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading));
            let item_id_dl = item_id.clone();
            let track_title = track.title.clone();
            let track_artist = track.artist.clone();

            rsx! {
                TrackRow {
                    key: "{track_key}",
                    track: track.clone(),
                    cover_url: cover_url.clone(),
                    row_num: Some(idx + 1),
                    is_menu_open,
                    is_album: false,
                    is_currently_playing: matches_current_path,
                    is_selection_mode: is_selection_mode(),
                    is_selected,
                    is_downloaded,
                    is_downloading,
                    on_long_press: move |_| {
                        is_selection_mode.set(true);
                        selected_tracks.write().insert(track_path.clone());
                    },
                    on_select: move |selected| {
                        if selected {
                            is_selection_mode.set(true);
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
                    on_queue: move |_| {
                        ctrl.add_to_queue(vec![track_queue.clone()]);
                        active_menu_track.set(None);
                    },
                    on_close_menu: move |_| active_menu_track.set(None),
                    on_delete: move |_| active_menu_track.set(None),
                    hide_delete: true,
                    on_download: move |_| {
                        if !is_downloaded {
                            active_menu_track.set(None);
                            queue_downloads(
                                vec![(item_id_dl.clone(), track_title.clone(), track_artist.clone())],
                                config,
                                download_queue,
                            );
                        }
                    },
                    on_play: move |_| {
                        queue.set(queue_source.clone());
                        ctrl.play_track(idx);
                    },
                }
            }
        });

    rsx! {
        div {
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
                                let conn = {
                                    let conf = config.peek();
                                    let Some(server) = conf.server.as_ref() else { return; };
                                    let Some(token) = server.access_token.as_ref() else { return; };
                                    ::server::server_ops::ServerConn {
                                        service: server.service,
                                        url: server.url.clone(),
                                        token: token.clone(),
                                        user_id: server.user_id.clone().unwrap_or_default(),
                                        device_id: conf.device_id.clone(),
                                    }
                                };
                                let item_ids: Vec<String> = selected_paths
                                    .iter()
                                    .filter_map(|p| {
                                        ::server::server_ops::parse_item_id(p.to_str()?)
                                            .map(str::to_string)
                                    })
                                    .collect();
                                let _ = ::server::server_ops::add_tracks_to_playlist(
                                    &conn, &pid, &item_ids,
                                )
                                .await;
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
                                let conn = {
                                    let conf = config.peek();
                                    let Some(server) = conf.server.as_ref() else { return; };
                                    let Some(token) = server.access_token.as_ref() else { return; };
                                    ::server::server_ops::ServerConn {
                                        service: server.service,
                                        url: server.url.clone(),
                                        token: token.clone(),
                                        user_id: server.user_id.clone().unwrap_or_default(),
                                        device_id: conf.device_id.clone(),
                                    }
                                };
                                let item_ids: Vec<String> = selected_paths
                                    .iter()
                                    .filter_map(|p| {
                                        ::server::server_ops::parse_item_id(p.to_str()?)
                                            .map(str::to_string)
                                    })
                                    .collect();
                                if !item_ids.is_empty() {
                                    let _ = ::server::server_ops::create_server_playlist(
                                        &conn,
                                        &playlist_name,
                                        &item_ids,
                                    )
                                    .await;
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
                    on_add_to_queue: move |_| {
                        let selected = selected_tracks.read().clone();
                        if selected.is_empty() {
                            return;
                        }
                        let tracks: Vec<_> = displayed_tracks_for_selection
                            .iter()
                            .filter(|(t, _)| selected.contains(&t.path))
                            .map(|(track, _)| track.clone())
                            .collect();
                        if !tracks.is_empty() {
                            ctrl.add_to_queue(tracks);
                        }
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
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

            // Generic "Syncing with server" spinner for non-YT
            // services. YT has its own status row below with track
            // counter + refresh button — don't double-render.
            if *is_syncing.read()
                && config
                    .read()
                    .server
                    .as_ref()
                    .map(|s| s.service != MusicService::YtMusic)
                    .unwrap_or(true)
            {
                div {
                    class: "flex items-center gap-2 text-slate-400 text-sm mb-4",
                    i { class: "fa-solid fa-circle-notch fa-spin" }
                    span { "{i18n::t(\"syncing_with_server\")}" }
                }
            }

            // Sync status row — visible whenever we're syncing or when
            // YT Music is the active server (so the user always has a
            // refresh button to force a re-fetch). Counter ticks up as
            // pages stream in. Stays out of the way for non-YT services
            // because it's a YT-specific affordance.
            {
                let is_ytmusic = config
                    .read()
                    .server
                    .as_ref()
                    .map(|s| s.service == MusicService::YtMusic)
                    .unwrap_or(false);
                let synced = *synced_so_far.read();
                let syncing = *is_syncing.read();
                let total = displayed_tracks.len();
                if is_ytmusic {
                    rsx! {
                        div {
                            class: "flex items-center justify-between gap-3 mb-3 px-2 text-xs text-slate-400",
                            div {
                                class: "flex items-center gap-2",
                                if syncing {
                                    i { class: "fa-solid fa-arrows-rotate fa-spin text-indigo-300" }
                                    span {
                                        "{i18n::t_with(\"yt_syncing_progress\", &[(\"count\", synced.to_string())])}"
                                    }
                                } else if total > 0 {
                                    i { class: "fa-solid fa-check text-emerald-400" }
                                    span {
                                        "{i18n::t_with(\"yt_synced_total\", &[(\"count\", total.to_string())])}"
                                    }
                                }
                            }
                            button {
                                class: "px-3 py-1 rounded bg-white/5 hover:bg-white/10 text-white/80 transition-colors disabled:opacity-50",
                                disabled: syncing,
                                onclick: move |_| {
                                    let next = *refresh_nonce.peek() + 1;
                                    refresh_nonce.set(next);
                                },
                                i { class: "fa-solid fa-arrows-rotate mr-1" }
                                "{i18n::t(\"refresh\")}"
                            }
                        }
                    }
                } else {
                    rsx! {}
                }
            }

            if is_empty && !*is_syncing.read() {
                {
                    let yt_anon = config
                        .read()
                        .server
                        .as_ref()
                        .map(|s| {
                            s.service == config::MusicService::YtMusic && s.yt_anonymous
                        })
                        .unwrap_or(false);
                    rsx! {
                        div {
                            class: "flex flex-col items-center justify-center h-64 text-slate-500 text-center px-6",
                            if yt_anon {
                                i { class: "fa-solid fa-right-to-bracket text-4xl mb-4 opacity-50" }
                                p { class: "text-base", "{i18n::t(\"yt_anon_favorites\")}" }
                            } else {
                                i { class: "fa-regular fa-heart text-4xl mb-4 opacity-30" }
                                p { class: "text-base", "{i18n::t(\"no_favorites\")}" }
                                p { class: "text-sm mt-1 opacity-70",
                                    "{i18n::t(\"heart_track_to_add_server\")}"
                                }
                            }
                        }
                    }
                }
            } else if !is_empty {
                div {
                    class: "flex items-center gap-3 mb-4 px-2 text-sm font-medium text-slate-500 uppercase tracking-wider",
                    button {
                        class: if displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.path)) {
                            "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                        } else {
                            "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                        },
                        aria_label: i18n::t("select_all_tracks"),
                        onclick: move |_| {
                            let all_selected = !displayed_tracks.is_empty() && displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.path));
                            if all_selected {
                                selected_tracks.write().clear();
                                is_selection_mode.set(false);
                            } else {
                                selected_tracks.set(displayed_tracks.iter().map(|(track, _)| track.path.clone()).collect());
                                is_selection_mode.set(true);
                            }
                        },
                        if displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.path)) {
                            i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                        }
                    }
                    span { "{i18n::t(\"select_all\")}" }
                }
                div {
                    class: if is_modern {
                        "grid px-3 py-2 text-[10px] font-bold uppercase tracking-widest border-b mb-1"
                    } else {
                        "grid gap-6 px-2 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2 uppercase tracking-wider"
                    },
                    style: if is_modern {
                        "grid-template-columns: 40px 1fr 180px 180px 56px 40px; color: rgba(255,255,255,0.25); border-color: rgba(255,255,255,0.06);"
                    } else {
                        "grid-template-columns: 40px minmax(0, 1fr) 200px 200px 64px 40px; align-items: center;"
                    },
                    div {}
                    button {
                        class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Title),
                        "{i18n::t(\"title\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Title)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Artist),
                        "{i18n::t(\"artist\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Artist)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Album),
                        "{i18n::t(\"album\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Album)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center justify-end gap-1 uppercase tracking-wider text-right hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Duration),
                        i { class: "fa-regular fa-clock" }
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Duration)} text-[10px]" }
                    }
                    div {}
                }
                div {
                    class: if is_modern { "" } else { "space-y-1" },
                    {tracks_nodes}
                }
            }
        }
    }
}

pub use JellyfinFavorites as ServerFavorites;

/// Build a list of synthetic Album entries out of the user's YT tracks.
/// YT doesn't expose a separate albums endpoint, so we group by
/// Track.album_id (assigned in search.rs::synthesize_album_id) and pick
/// the first track per group as the album's representative for title +
/// artist + cover.
fn synthesize_albums(tracks: &[reader::models::Track]) -> Vec<reader::models::Album> {
    use std::collections::HashMap;
    use std::path::PathBuf;
    let mut by_album: HashMap<String, &reader::models::Track> = HashMap::new();
    for t in tracks {
        if t.album_id.is_empty() {
            continue;
        }
        by_album.entry(t.album_id.clone()).or_insert(t);
    }
    by_album
        .into_iter()
        .map(|(album_id, t)| {
            // Reuse the first track's encoded thumbnail (3rd segment of
            // its path: `ytmusic:VID:urlhex_HEX`) as the album cover.
            // `jellyfin_image_url_from_path` will decode the embedded
            // URL out of this PathBuf the same way it does for tracks.
            let cover_path = t
                .path
                .to_string_lossy()
                .split(':')
                .nth(2)
                .filter(|s| s.starts_with("urlhex_"))
                .map(|tag| PathBuf::from(format!("ytmusic:_:{tag}")));
            reader::models::Album {
                id: album_id,
                title: if t.album.is_empty() {
                    "Singles".to_string()
                } else {
                    t.album.clone()
                },
                artist: t.artist.clone(),
                genre: String::new(),
                year: 0,
                cover_path,
                manual_cover: false,
            }
        })
        .collect()
}
