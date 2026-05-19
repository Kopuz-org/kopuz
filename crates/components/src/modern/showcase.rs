use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

use crate::showcase::{self, ShowcaseProps, SortField};
use crate::track_row::TrackRow;
use crate::NavigationController;
use std::collections::HashSet;

#[component]
pub fn ShowcaseModern(props: ShowcaseProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();
    let nav_ctrl = use_context::<NavigationController>();

    let total_seconds: u64 = props.tracks.iter().map(|t| t.duration).sum();
    let duration_min = total_seconds / 60;

    let offline_tracks = config.read().offline_tracks.clone();
    let fmt_dur = |s: u64| format!("{}:{:02}", s / 60, s % 60);
    let sort_state = use_signal(|| None);
    let indexed_tracks: Vec<_> = props
        .tracks
        .iter()
        .cloned()
        .enumerate()
        .map(|(idx, track)| (track, idx))
        .collect();
    let sorted_track_pairs = showcase::sorted_track_pairs(&indexed_tracks, *sort_state.read());
    let sorted_tracks: Vec<_> = sorted_track_pairs
        .iter()
        .map(|(track, _)| track.clone())
        .collect();
    let tracks_for_shuffle = sorted_tracks.clone();

    let has_multiple_discs = sorted_tracks
        .iter()
        .filter_map(|t| t.disc_number)
        .collect::<HashSet<_>>()
        .len()
        > 1;
    let mut last_disc = None;
    let mut last_disc_size = 0;

    let currently_playing_path = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|track| track.path.clone())
    };

    let current_song_title = ctrl.current_song_title.read().clone();
    let current_song_artist = ctrl.current_song_artist.read().clone();
    let current_song_album = ctrl.current_song_album.read().clone();
    let current_song_duration = *ctrl.current_song_duration.read();
    let tracks_for_play_all = sorted_tracks.clone();

    let columns = if props.is_album {
        "40px minmax(200px, 1fr) minmax(100px,400px) 64px 40px".to_string()
    } else {
        "40px minmax(200px, 1fr) minmax(100px,200px) minmax(100px,200px) 64px 40px".to_string()
    };

    rsx! {
        div { class: "w-full max-w-[1600px] mx-auto select-none pb-8",

            div { class: "flex items-end gap-6 mb-8 px-6 pt-6",
                div {
                    class: "w-44 h-44 rounded-2xl overflow-hidden shrink-0 shadow-2xl bg-white/5",
                    style: "box-shadow: 0 20px 60px rgba(0,0,0,0.6);",
                    if let Some(url) = &props.cover_url {
                        img {
                            src: "{url.as_ref()}",
                            class: "w-full h-full object-cover cursor-pointer",
                            onclick: move |_| {
                                if let Some(ref h) = props.on_cover_click { h.call(()); }
                            }
                        }
                    } else {
                        div { class: "w-full h-full flex items-center justify-center",
                            i { class: "fa-solid fa-music text-4xl", style: "color: var(--color-white); opacity: 0.15;" }
                        }
                    }
                }

                div { class: "flex flex-col gap-1 pb-1 min-w-0",
                    if !props.description.is_empty() {
                        p {
                            class: "text-xs font-bold tracking-widest uppercase mb-1",
                            style: "color: var(--color-white); opacity: 0.35;",
                            "{props.description}"
                        }
                    }
                    h1 {
                        class: "text-4xl font-bold text-white truncate mb-1",
                        "{props.name}"
                    }
                    p {
                        class: "text-sm mb-3",
                        style: "color: var(--color-white); opacity: 0.45;",
                        {
                            let count = props.tracks.len();
                            let song_text = i18n::t_with("showcase_song_count", &[("count", count.to_string())]);
                            rsx! { "{song_text} · {duration_min} {i18n::t(\"min\")}" }
                        }
                    }

                    div { class: "flex items-center gap-2 flex-wrap",
                        if !props.tracks.is_empty() {
                            button {
                                class: "inline-flex items-center justify-center gap-2 h-9 px-5 rounded-full text-sm font-semibold text-white transition-opacity hover:opacity-90 active:scale-95",
                                style: "background: var(--color-indigo-500);",
                                onclick: move |_| {
                                    if *ctrl.shuffle.peek() {
                                        ctrl.play_queue_shuffled(tracks_for_play_all.clone());
                                    } else {
                                        ctrl.play_queue_linear(tracks_for_play_all.clone());
                                    }
                                },
                                i { class: "fa-solid fa-play text-xs" }
                                "{i18n::t(\"play\")}"
                            }
                            button {
                                class: "inline-flex items-center justify-center gap-2 h-9 px-5 rounded-full text-sm font-semibold text-white transition-opacity hover:opacity-90 active:scale-95",
                                style: if *ctrl.shuffle.read() {
                                    "background: var(--color-indigo-500);"
                                } else {
                                    "background: color-mix(in oklab, var(--color-indigo-500) 25%, transparent); border: 1px solid color-mix(in oklab, var(--color-indigo-500) 40%, transparent);"
                                },
                                onclick: move |_| {
                                    ctrl.toggle_shuffle();
                                    ctrl.play_queue_shuffled(tracks_for_shuffle.clone());
                                },
                                i { class: "fa-solid fa-shuffle text-xs" }
                                "{i18n::t(\"shuffle\")}"
                            }
                            if props.on_download_all.is_some() || props.on_delete_all.is_some() {
                                button {
                                    class: "inline-flex items-center justify-center h-9 w-9 rounded-full text-sm font-medium transition-colors border border-white/12 hover:bg-white/10",
                                    style: "color: var(--color-white); opacity: 0.6;",
                                    disabled: props.is_downloading_all,
                                    onclick: move |_| {
                                        if props.on_delete_all.is_some() {
                                            if let Some(ref h) = props.on_delete_all { h.call(()); }
                                        } else if let Some(ref h) = props.on_download_all { h.call(()); }
                                    },
                                    if props.is_downloading_all {
                                        i { class: "fa-solid fa-spinner fa-spin text-xs" }
                                    } else {
                                        i { class: "fa-solid fa-download text-xs" }
                                    }
                                }
                            }
                        }
                        if let Some(actions) = props.actions {
                            {actions}
                        }
                    }
                }
            }

            if props.tracks.is_empty() {
                div { class: "flex flex-col items-center justify-center py-16 gap-3",
                    i { class: "fa-regular fa-folder-open text-4xl", style: "color: var(--color-white); opacity: 0.15;" }
                    p { class: "text-sm", style: "color: var(--color-white); opacity: 0.3;", "{i18n::t(\"no_songs_here\")}" }
                }
            } else {
                div {
                    class: "grid px-3 py-2 text-[10px] font-bold text-slate-500 border-white/5 uppercase tracking-widest border-b mb-1",
                    style: "grid-template-columns: {columns};",
                    div {
                        class: "flex items-center h-4 shrink-0",
                        if props.is_selection_mode {
                            if let Some(handler) = props.on_select_all {
                                div { class: "flex items-center w-6 h-6 shrink-0",
                                      button {
                                          class: if props.all_selected {
                                              "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                                          } else {
                                              "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                                          },
                                          aria_label: if props.all_selected { "Deselect all tracks" } else { "Select all tracks" },
                                          onclick: move |_| handler.call(!props.all_selected),
                                          if props.all_selected {
                                              i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                                          }
                                      }
                                }
                            }
                        } else {
                            "#"
                        }
                    }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Title),
                        "{i18n::t(\"title\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Title)} text-[9px]" }
                    }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Artist),
                        "{i18n::t(\"artist\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Artist)} text-[9px]" }
                    }
                    if !props.is_album {
                        button {
                            class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                            onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Album),
                            "{i18n::t(\"album\")}"
                            i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Album)} text-[9px]" }
                        }
                    }
                    button {
                        class: "flex items-center justify-end gap-1 uppercase tracking-widest text-right hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Duration),
                        i { class: "fa-regular fa-clock" }
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Duration)} text-[9px]" }
                    }
                    div {}
                }

                for (display_idx, (track, idx)) in sorted_track_pairs.iter().enumerate() {
                    {
                        let idx = *idx;
                        let matches_current_path = currently_playing_path.as_ref() == Some(&track.path);
                        let matches_current_metadata = currently_playing_path.is_none()
                            && !current_song_title.is_empty()
                            && track.title == current_song_title
                            && track.artist == current_song_artist
                            && track.album == current_song_album
                            && track.duration == current_song_duration;
                        let is_currently_playing: bool = matches_current_path || matches_current_metadata;
                        let is_selected = props.is_selection_mode && props.selected_tracks.contains(&track.path);
                        let selection_shadow = if is_selected {
                            "inset 0 0 0 9999px color: var(--color-white); opacity: 0.07;"
                        } else {
                            "none"
                        };
                        let track_dur = fmt_dur(track.duration);
                        let artist = track.artist.clone();
                        let album = track.album.clone();
                        let album_id = track.album_id.clone();
                        let row_num = display_idx + 1;


                        let path_str = track.path.to_string_lossy();
                        let item_id_str: String = path_str.split(':').nth(1).unwrap_or(&path_str).to_string();
                        let is_downloaded = if let Some(path_str) = offline_tracks.get(&item_id_str) {
                            std::path::Path::new(path_str).exists()
                        } else {
                            false
                        };
                        let is_downloading = false;
                        let play_queue = sorted_tracks.clone();
                        let play_queue_button = sorted_tracks.clone();

                        let cover_url: Option<utils::CoverUrl> = {
                            let path_str = track.path.to_string_lossy();
                            if path_str.starts_with("jellyfin:") {
                                let conf = config.read();
                                conf.server.as_ref().and_then(|s| {
                                    utils::jellyfin_image::track_cover_url_with_album_fallback(
                                        &path_str,
                                        &track.album_id,
                                        &s.url,
                                        s.access_token.as_deref(),
                                        64,
                                        90,
                                    ).map(|u| std::sync::Arc::from(u.as_str()))
                                })
                            } else {
                                let lib = props.library.read();
                                lib.albums
                                    .iter()
                                    .find(|a| a.id == track.album_id)
                                    .and_then(|a| utils::format_artwork_url(a.cover_path.as_ref()))
                            }
                        };

                        let mut is_new_disc = false;
                        if track.disc_number != last_disc && sort_state.peek().is_none() && props.is_album {
                            last_disc = track.disc_number;
                            is_new_disc = true;
                            last_disc_size = display_idx;
                        }

                        rsx! {
                            div {
                                class: "flex items-center group",
                                if has_multiple_discs && props.is_album && is_new_disc && sort_state.peek().is_none() {
                                    div {
                                        class: "flex-1 min-w-0",
                                        div {
                                            class: "grid items-center p-2 rounded-lg hover:bg-white/5 group transition-colors relative select-none",
                                            style: format!("grid-template-columns: {columns};"),
                                            i { class: "fa-solid fa-compact-disc" }
                                            p { "Disc {track.disc_number.unwrap_or(1)}" }
                                        }
                                    }
                                }
                            }
                            div {
                                key: "{track.path.display()}",
                                class: "flex items-center group",
                                div { class: "flex-1 min-w-0",
                                    TrackRow {
                                        track: track.clone(),
                                        cover_url: cover_url,
                                        is_menu_open: props.active_track.as_ref() == Some(&track.path),
                                        is_album: props.is_album,
                                        is_selection_mode: props.is_selection_mode,
                                        is_selected: is_selected,
                                        is_downloaded: is_downloaded,
                                        is_downloading: is_downloading,
                                        is_currently_playing,
                                        row_num: Some(display_idx + 1 - last_disc_size),
                                        on_select: move |selected| {
                                            if let Some(handler) = &props.on_select {
                                                handler.call((idx, selected));
                                            }
                                        },
                                        on_long_press: move |_| {
                                            if let Some(handler) = &props.on_long_press {
                                                handler.call(idx);
                                            }
                                        },
                                        on_click_menu: move |_| {
                                            if let Some(handler) = &props.on_click_menu {
                                                handler.call(idx);
                                            }
                                        },
                                        on_add_to_playlist: move |_| {
                                            if let Some(handler) = &props.on_add_to_playlist {
                                                handler.call(idx);
                                            }
                                        },
                                        on_queue: move |_| {
                                            if let Some(handler) = &props.on_queue {
                                                handler.call(idx);
                                            }
                                        },
                                        on_close_menu: move |_| {
                                            if let Some(handler) = &props.on_close_menu {
                                                handler.call(());
                                            }
                                        },
                                        on_delete: move |_| {
                                            if let Some(handler) = &props.on_delete_track {
                                                handler.call(idx);
                                            }
                                        },
                                        on_remove_from_playlist: move |_| {
                                            if let Some(handler) = &props.on_remove_from_playlist {
                                                handler.call(idx);
                                            }
                                        },
                                        on_download: move |_| {
                                            if let Some(handler) = &props.on_download_track {
                                                handler.call(idx);
                                            }
                                        },
                                        on_play: move |_| {
                                            ctrl.queue.set(play_queue.clone());
                                            ctrl.play_track(display_idx);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
