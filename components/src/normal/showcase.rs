use crate::reorder_buttons::ReorderButtons;
use crate::showcase::{self, ShowcaseProps, SortField};
use crate::track_row::TrackRow;
use config::{AppConfig, MusicService, MusicSource};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

#[component]
pub fn ShowcaseNormal(props: ShowcaseProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();
    let total_seconds: u64 = props.tracks.iter().map(|t| t.duration).sum();
    let duration_min = total_seconds / 60;

    let lib = props.library.read();
    let is_server_source = config.read().active_source == MusicSource::Server;

    let offline_tracks = config.read().offline_tracks.clone();
    let mut sort_state = use_signal(|| None);
    let sorted_indices = showcase::sorted_track_indices(&props.tracks, *sort_state.read());
    let sorted_tracks: Vec<_> = sorted_indices
        .iter()
        .map(|&idx| props.tracks[idx].clone())
        .collect();

    let currently_playing_idx: Option<usize> = {
        let queue = ctrl.queue.read();
        let idx = *ctrl.current_queue_index.read();
        if queue.len() == sorted_tracks.len()
            && queue
                .iter()
                .zip(sorted_tracks.iter())
                .all(|(q, t)| q.path == t.path)
        {
            Some(idx)
        } else {
            None
        }
    };
    let tracks_for_play_all = sorted_tracks.clone();

    let all_downloaded = !props.tracks.is_empty()
        && props.tracks.iter().all(|t| {
            let p = t.path.to_string_lossy();
            let id = p.split(':').nth(1).unwrap_or(&p);
            if let Some(path_str) = offline_tracks.get(id) {
                std::path::Path::new(path_str).exists()
            } else {
                false
            }
        });

    rsx! {
         div {
             class: "select-none",
             div {
                 class: "flex flex-col md:flex-row items-end gap-8 mb-12",
                 div { class: "w-64 h-64 rounded-xl bg-stone-800 overflow-hidden relative flex-shrink-0",
                     if let Some(url) = &props.cover_url {
                         img { src: "{url.as_ref()}", class: "w-full h-full object-cover" }
                     } else {
                         div { class: "w-full h-full flex flex-col items-center justify-center text-white/20",
                             i { class: "fa-solid fa-music text-6xl mb-4" }
                         }
                     }
                     if props.on_cover_click.is_some() {
                         div {
                             class: "absolute inset-0 bg-black/50 opacity-0 hover:opacity-100 transition-opacity flex items-center justify-center cursor-pointer rounded-xl",
                             onclick: move |_| {
                                 if let Some(ref h) = props.on_cover_click {
                                     h.call(());
                                 }
                             },
                             i { class: "fa-solid fa-camera text-white text-3xl" }
                         }
                     }
                 }
                 div { class: "flex-1",
                     if !props.description.is_empty() {
                         h5 { class: "text-sm font-bold tracking-widest text-white/60 uppercase mb-2", "{props.description}" }
                     }
                     h1 { class: "text-5xl md:text-7xl font-bold text-white mb-6", "{props.name}" }
                     div { class: "flex items-center gap-6 text-slate-400",
                         {
                            let count = props.tracks.len();
                            let song_text = i18n::t_with("showcase_song_count", &[("count", count.to_string())]);
                            rsx! {
                                p { "{song_text}" }
                            }
                         }
                         span { "•" }
                         p { "{duration_min} {i18n::t(\"min\")}" }
                     }
                 }

                div { class: "flex items-center gap-4",
                     if !props.tracks.is_empty() {
                        button {
                            class: format!("w-14 h-14 rounded-full flex items-center justify-center {}", if *ctrl.shuffle.read() { "text-white" } else { "text-slate-400 hover:text-white" }),
                            title: if *ctrl.shuffle.read() {
                                i18n::t("shuffle_on").to_string()
                            } else {
                                i18n::t("shuffle_off").to_string()
                            },
                            onclick: move |_| ctrl.toggle_shuffle(),
                            i { class: "fa-solid fa-shuffle text-xl ml-1" }
                        }
                        button {
                             class: "w-14 h-14 rounded-full bg-indigo-500 hover:bg-indigo-400 text-black flex items-center justify-center transition-transform hover:scale-105",
                             onclick: move |_| ctrl.play_queue_linear(tracks_for_play_all.clone()),
                             i { class: "fa-solid fa-play text-xl ml-1" }
                         }
                         if props.on_download_all.is_some() || props.on_delete_all.is_some() {
                             button {
                                 class: "w-12 h-12 rounded-full border border-white/20 hover:border-white/40 text-white/70 hover:text-white flex items-center justify-center transition-colors",
                                 title: if all_downloaded { "Remove downloads" } else { "Download all for offline playback" },
                                 disabled: props.is_downloading_all,
                                 onclick: move |_| {
                                     if all_downloaded {
                                         if let Some(ref h) = props.on_delete_all { h.call(()); }
                                     } else {
                                         if let Some(ref h) = props.on_download_all { h.call(()); }
                                     }
                                 },
                                 if props.is_downloading_all {
                                     i { class: "fa-solid fa-spinner fa-spin" }
                                 } else if all_downloaded {
                                     i { class: "fa-solid fa-trash" }
                                 } else {
                                     i { class: "fa-solid fa-download" }
                                 }
                             }
                         }
                     }
                     if let Some(actions) = props.actions {
                         {actions}
                     }
                 }
             }

             div { class: "space-y-1",
                 if props.tracks.is_empty() {
                     div { class: "py-12 flex flex-col items-center justify-center text-slate-600",
                         i { class: "fa-regular fa-folder-open text-4xl mb-4" }
                         p { class: "text-lg", "{i18n::t(\"no_songs_here\")}" }
                     }
                 } else {
                      div {
                           class: "grid gap-6 px-2 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2 uppercase tracking-wider",
                           style: "grid-template-columns: 40px minmax(0, 1fr) 200px 200px 64px 40px; align-items: center;",
                           div { class: "flex items-center w-10 shrink-0",
                               if props.is_selection_mode {
                                   if let Some(handler) = props.on_select_all {
                                       div { class: "mr-4 flex items-center justify-center w-6 h-6 shrink-0",
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
                               class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                               onclick: move |_| {
                                                                  let next = showcase::next_sort_state(*sort_state.peek(), SortField::Title);
                                                                  sort_state.set(next);
                                                              },
                               "{i18n::t(\"title\")}"
                               i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Title)} text-[10px]" }
                           }
                           button {
                               class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                               onclick: move |_| {
                                                                  let next = showcase::next_sort_state(*sort_state.peek(), SortField::Artist);
                                                                  sort_state.set(next);
                                                              },
                               "{i18n::t(\"artist\")}"
                               i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Artist)} text-[10px]" }
                           }
                           button {
                               class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                               onclick: move |_| {
                                                                  let next = showcase::next_sort_state(*sort_state.peek(), SortField::Album);
                                                                  sort_state.set(next);
                                                              },
                               "{i18n::t(\"album\")}"
                               i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Album)} text-[10px]" }
                           }
                           button {
                               class: "flex items-center justify-end gap-1 uppercase tracking-wider text-right hover:text-white transition-colors",
                               onclick: move |_| {
                                                                  let next = showcase::next_sort_state(*sort_state.peek(), SortField::Duration);
                                                                  sort_state.set(next);
                                                              },
                               i { class: "fa-regular fa-clock" }
                               i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Duration)} text-[10px]" }
                           }
                           div {}
                      }

                     for (display_idx, idx) in sorted_indices.iter().copied().enumerate() {
                         {
                             let track = &props.tracks[idx];
                             let cover_url = if is_server_source {
                                 if let Some(server) = &config.read().server {
                                     let path_str = track.path.to_string_lossy();
                                     let url = match server.service {
                                         MusicService::Jellyfin => {
                                             utils::jellyfin_image::track_cover_url_with_album_fallback(
                                                 &path_str,
                                                 &track.album_id,
                                                 &server.url,
                                                 server.access_token.as_deref(),
                                                 80,
                                                 80,
                                             )
                                         }
                                         MusicService::Subsonic | MusicService::Custom => {
                                             utils::subsonic_image::subsonic_image_url_from_path(
                                                 &path_str,
                                                 &server.url,
                                                 server.access_token.as_deref(),
                                                 80,
                                                 80,
                                             )
                                         }
                                     };
                                     utils::map_cover_url(url)
                                 } else { None }
                             } else {
                                 lib.albums.iter()
                                    .find(|a| a.id == track.album_id)
                                    .and_then(|a| utils::format_artwork_url(a.cover_path.as_ref()))
                             };

                             let is_selected = props.selected_tracks.contains(&track.path);
                             let track_count = props.tracks.len();
                             let can_move_up = props.is_reorderable && idx > 0;
                             let can_move_down = props.is_reorderable && idx + 1 < track_count;

                             let path_str = track.path.to_string_lossy();
                             let item_id_str: String = path_str.split(':').nth(1).unwrap_or(&path_str).to_string();
                             let is_downloaded = if let Some(path_str) = offline_tracks.get(&item_id_str) {
                                 std::path::Path::new(path_str).exists()
                             } else {
                                 false
                             };
                             let is_downloading = false;
                             let play_queue = sorted_tracks.clone();

                             rsx! {
                                 div {
                                     key: "{track.path.display()}",
                                     class: "flex items-center group",
                                     div { class: "flex-1 min-w-0",
                                         TrackRow {
                                             track: track.clone(),
                                             cover_url: cover_url,
                                             is_menu_open: props.active_track.as_ref() == Some(&track.path),
                                             is_selection_mode: props.is_selection_mode,
                                             is_selected: is_selected,
                                             is_downloaded: is_downloaded,
                                             is_downloading: is_downloading,
                                             is_currently_playing: currently_playing_idx == Some(display_idx),
                                             row_num: Some(display_idx + 1),
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
                                     if props.is_reorderable && !props.is_selection_mode {
                                         ReorderButtons {
                                             can_move_up,
                                             can_move_down,
                                             on_move_up: move |_| props.on_move_up.call(idx),
                                             on_move_down: move |_| props.on_move_down.call(idx),
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
