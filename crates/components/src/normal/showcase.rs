use std::collections::HashSet;

use crate::constants::{COLUMNS_NORMAL, COLUMNS_NORMAL_ALBUM};
use crate::header::Header;
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
    let sorted_tracks_arc = std::sync::Arc::new(sorted_tracks.clone());

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
    let selected_queue_tracks: Vec<_> = sorted_tracks
        .iter()
        .filter(|track| props.selected_tracks.contains(&track.path))
        .cloned()
        .collect();
    let selected_queue_tracks_arc = std::sync::Arc::new(selected_queue_tracks.clone());

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

    let columns = if props.is_album {
        COLUMNS_NORMAL_ALBUM
    } else {
        COLUMNS_NORMAL
    };
    let column_gap = if cfg!(target_os = "android") { "0.5rem" } else { "1.5rem" };

    let scroll_stat = use_signal(|| 0.0_f64);
    let container_height = use_signal(|| 0.0_f64);
    const ITEM_HEIGHT: f64 = 56.0;

    let scroll_info = crate::virtual_scroll::use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        sorted_track_pairs.len(),
        ITEM_HEIGHT,
    );

    rsx! {
         div {
             class: "select-none flex-1 min-h-0 flex flex-col w-full",
             div {
                 class: if cfg!(target_os = "android") { "flex flex-col items-center text-center gap-4 mb-6 shrink-0" } else { "flex flex-col md:flex-row items-end gap-8 mb-12 shrink-0" },
                 div { class: if cfg!(target_os = "android") { "w-44 h-44 rounded-xl bg-stone-800 overflow-hidden relative flex-shrink-0" } else { "w-64 h-64 rounded-xl bg-stone-800 overflow-hidden relative flex-shrink-0" },
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
                     h1 { class: if cfg!(target_os = "android") { "text-3xl font-bold text-white mb-3" } else { "text-5xl md:text-7xl font-bold text-white mb-6" }, "{props.name}" }
                     div { class: if cfg!(target_os = "android") { "flex items-center justify-center gap-4 text-slate-400" } else { "flex items-center gap-6 text-slate-400" },
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
                             onclick: move |_| {
                                let is_shuffle = *ctrl.shuffle.peek();
                                if is_shuffle {
                                    ctrl.play_queue_shuffled(tracks_for_play_all.clone());
                                } else {
                                    ctrl.play_queue_linear(tracks_for_play_all.clone());
                                }
                             },
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

             div { class: "flex-1 min-h-0 flex flex-col w-full",
                 if props.tracks.is_empty() {
                     div { class: "py-12 flex flex-col items-center justify-center text-slate-600",
                         i { class: "fa-regular fa-folder-open text-4xl mb-4" }
                         p { class: "text-lg", "{i18n::t(\"no_songs_here\")}" }
                     }
                 } else {
                     div { class: "shrink-0",
                         Header {
                             is_modern: false,
                             is_album: props.is_album,
                             is_selection_mode: props.is_selection_mode,
                             on_select_all: props.on_select_all,
                             all_selected: props.all_selected,
                             sort_state: sort_state,
                             is_reorderable: props.is_reorderable
                         }
                     }
                     div { class: "flex-1 relative min-h-0 w-full overflow-hidden",
                     crate::virtual_scroll::VirtualScrollView {
                         id: "normal-showcase-scroll".to_string(),
                         class: "absolute inset-0 overflow-y-auto pb-20".to_string(),
                         scroll_stat,
                         container_height,
                         item_height: ITEM_HEIGHT,
                         saved_scroll: 0.0,
                         top_pad: scroll_info.top_pad,
                         bottom_pad: scroll_info.bottom_pad,
                         for (display_idx, (track, idx)) in sorted_track_pairs.iter().enumerate().skip(scroll_info.start_index).take(scroll_info.items_to_render) {
                         {
                             let idx = *idx;
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
                             let matches_current_path = currently_playing_path.as_ref() == Some(&track.path);
                             let matches_current_metadata = currently_playing_path.is_none()
                                 && !current_song_title.is_empty()
                                 && track.title == current_song_title
                                 && track.artist == current_song_artist
                                 && track.album == current_song_album
                                 && track.duration == current_song_duration;
                             let is_currently_playing: bool = matches_current_path || matches_current_metadata;
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
                             let play_queue = std::sync::Arc::clone(&sorted_tracks_arc);

                             let mut is_new_disc = false;
                             if track.disc_number != last_disc && sort_state.peek().is_none() && props.is_album {
                                 last_disc = track.disc_number;
                                 is_new_disc = true;
                                 last_disc_size = display_idx;
                             }

                             rsx! {
                                 // discs
                                 div {
                                     class: "flex items-center group",
                                     if has_multiple_discs && props.is_album && is_new_disc && sort_state.peek().is_none() {
                                         div {
                                             class: "flex-1 min-w-0",
                                             div {
                                                 class: "grid items-center p-2 rounded-lg hover:bg-white/5 group transition-colors relative select-none",
                                                 style: format!("grid-template-columns: {columns}; column-gap: {column_gap};"),
                                                 i { class: "fa-solid fa-compact-disc text-center" }
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
                                             selected_queue_tracks: (*selected_queue_tracks_arc).clone(),
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
                                                 ctrl.queue.set((*play_queue).clone());
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
    }
}
