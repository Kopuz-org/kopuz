use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::showcase::{self, SortField};
use components::track_row::TrackRow;
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use reader::{FavoritesStore, Library, PlaylistStore};
use std::collections::HashSet;
use std::path::PathBuf;

#[component]
pub fn LocalFavorites(
    favorites_store: Signal<FavoritesStore>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    playlist_store: Signal<PlaylistStore>,
    mut queue: Signal<Vec<reader::models::Track>>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());
    let sort_state = use_signal(|| None);

    let displayed_tracks: Vec<(reader::models::Track, Option<utils::CoverUrl>)> = {
        let store = favorites_store.read();
        let lib = library.read();
        let album_covers: std::collections::HashMap<_, _> = lib
            .albums
            .iter()
            .map(|a| {
                (
                    a.id.clone(),
                    a.cover_path
                        .as_ref()
                        .and_then(|cp| utils::format_artwork_url(Some(cp))),
                )
            })
            .collect::<std::collections::HashMap<String, Option<utils::CoverUrl>>>();
        lib.tracks
            .iter()
            .filter(|t| store.is_local_favorite(&t.path))
            .map(|t| {
                let cover_url = album_covers.get(&t.album_id).cloned().flatten();
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
    let queue_tracks_for_selection = queue_tracks.clone();

    let currently_playing_path = {
        let queue = ctrl.queue.read();
        let idx = *ctrl.current_queue_index.read();
        let queue_idx = if *ctrl.shuffle.read() {
            ctrl.shuffle_order.read().get(idx).copied().unwrap_or(idx)
        } else {
            idx
        };
        queue.get(queue_idx).map(|track| track.path.clone())
    };

    let is_empty = displayed_tracks.is_empty();
    let is_modern = config.read().ui_style == UiStyle::Modern;

    let tracks_nodes =
        sorted_displayed_tracks
            .iter()
            .cloned()
            .enumerate()
            .map(|(idx, (track, cover_url))| {
                let track_menu = track.clone();
                let track_path = track.path.clone();
                let track_select = track.path.clone();
                let track_add = track.clone();
                let track_queue = track.clone();
                let track_delete = track.clone();
                let queue_source = queue_tracks.clone();
                let track_key = format!("{}-{}", track.path.display(), idx);
                let is_menu_open = active_menu_track.read().as_ref() == Some(&track.path);
                let is_selected = selected_tracks.read().contains(&track_path);
                let matches_current_path = currently_playing_path.as_ref() == Some(&track.path);

                rsx! {
                    div {
                        key: "{track_key}",
                        style: "content-visibility: auto; contain-intrinsic-size: 0 60px;",
                        TrackRow {
                            track: track.clone(),
                            cover_url: cover_url.clone(),
                            row_num: Some(idx + 1),
                        is_menu_open,
                        is_currently_playing: matches_current_path,
                        is_selection_mode: is_selection_mode(),
                        is_selected,
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
                        on_delete: move |_| {
                            active_menu_track.set(None);
                            if std::fs::remove_file(&track_delete.path).is_ok() {
                                library.write().remove_track(&track_delete.path);
                            }
                        },
                        on_play: move |_| {
                            queue.set(queue_source.clone());
                            ctrl.play_track(idx);
                        },
                    }
                    }
                }
            });

    rsx! {
        div {
            if *show_playlist_modal.read() {
                PlaylistModal {
                    playlist_store,
                    is_jellyfin: false,
                    on_close: move |_| {
                        show_playlist_modal.set(false);
                        if is_selection_mode() {
                            is_selection_mode.set(false);
                            selected_tracks.write().clear();
                        }
                    },
                    on_add_to_playlist: move |playlist_id: String| {
                        let mut store = playlist_store.write();
                        if let Some(playlist) = store.playlists.iter_mut().find(|p| p.id == playlist_id) {
                            if is_selection_mode() {
                                for path in selected_tracks.read().iter() {
                                    if !playlist.tracks.contains(path) {
                                        playlist.tracks.push(path.clone());
                                    }
                                }
                            } else if let Some(path) = selected_track_for_playlist.read().clone() {
                                if !playlist.tracks.contains(&path) {
                                    playlist.tracks.push(path);
                                }
                            }
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_create_playlist: move |name: String| {
                        let mut tracks = Vec::new();
                        if is_selection_mode() {
                            tracks = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            tracks.push(path);
                        }

                        let mut store = playlist_store.write();
                        store.playlists.push(reader::models::Playlist {
                            id: uuid::Uuid::new_v4().to_string(),
                            name,
                            tracks,
                            cover_path: None,
                        });
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
                    on_add_to_queue: move |_| {
                        let selected = selected_tracks.read().clone();
                        if selected.is_empty() {
                            return;
                        }
                        let tracks: Vec<_> = queue_tracks_for_selection
                            .iter()
                            .filter(|t| selected.contains(&t.path))
                            .cloned()
                            .collect();
                        if !tracks.is_empty() {
                            ctrl.add_to_queue(tracks);
                        }
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    },
                    on_add_to_playlist: move |_| {
                        show_playlist_modal.set(true);
                    },
                    on_delete: move |_| {
                        let paths: Vec<_> = selected_tracks.read().iter().cloned().collect();
                        for path in paths {
                            if std::fs::remove_file(&path).is_ok() {
                                library.write().remove_track(&path);
                            }
                        }
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                    },
                    on_cancel: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    }
                }
            }

            if is_empty {
                div {
                    class: "flex flex-col items-center justify-center h-64 text-slate-500",
                    i { class: "fa-regular fa-heart text-4xl mb-4 opacity-30" }
                    p { class: "text-base", "{i18n::t(\"no_favorites\")}" }
                    p { class: "text-sm mt-1 opacity-70",
                        "{i18n::t(\"heart_track_to_add\")}"
                    }
                }
            } else {
                div {
                    class: "flex items-center gap-3 mb-4 px-2 text-sm font-medium text-slate-500 uppercase tracking-wider",
                    button {
                        class: if !is_empty && displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.path)) {
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
                        if !is_empty && displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.path)) {
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
