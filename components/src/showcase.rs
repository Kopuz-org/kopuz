use crate::reorder_buttons::ReorderButtons;
use crate::track_row::TrackRow;
use config::{AppConfig, MusicService, MusicSource};
use dioxus::prelude::*;
use reader::{Library, Track};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Props, Clone, PartialEq)]
pub struct ShowcaseProps {
    pub name: String,
    pub description: String,
    pub cover_url: Option<utils::CoverUrl>,
    pub tracks: Vec<Track>,
    pub library: Signal<Library>,
    pub on_play: EventHandler<usize>,
    pub on_add_to_playlist: Option<EventHandler<usize>>,
    pub on_delete_track: Option<EventHandler<usize>>,
    pub on_remove_from_playlist: Option<EventHandler<usize>>,
    pub on_download_track: Option<EventHandler<usize>>,
    pub active_track: Option<std::path::PathBuf>,
    pub on_click_menu: Option<EventHandler<usize>>,
    pub on_close_menu: Option<EventHandler<()>>,
    pub actions: Option<Element>,
    pub on_download_all: Option<EventHandler<()>>,
    pub on_delete_all: Option<EventHandler<()>>,
    #[props(default = false)]
    pub is_downloading_all: bool,
    #[props(default = false)]
    pub is_selection_mode: bool,
    #[props(default = HashSet::new())]
    pub selected_tracks: HashSet<PathBuf>,
    pub on_select: Option<EventHandler<(usize, bool)>>,
    pub on_select_all: Option<EventHandler<bool>>,
    #[props(default = false)]
    pub all_selected: bool,
    pub on_long_press: Option<EventHandler<usize>>,
    pub on_cover_click: Option<EventHandler<()>>,
    #[props(default = false)]
    pub is_reorderable: bool,
    #[props(default)]
    pub on_move_up: EventHandler<usize>,
    #[props(default)]
    pub on_move_down: EventHandler<usize>,
}

#[component]
pub fn Showcase(props: ShowcaseProps) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let total_seconds: u64 = props.tracks.iter().map(|t| t.duration).sum();
    let duration_min = total_seconds / 60;

    let lib = props.library.read();
    let is_server_source = config.read().active_source == MusicSource::Server;

    // Read offline_tracks once for the whole render pass so we can cheaply check per-track
    let offline_tracks = config.read().offline_tracks.clone();

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
                             class: "w-14 h-14 rounded-full bg-indigo-500 hover:bg-indigo-400 text-black flex items-center justify-center transition-transform hover:scale-105",
                             onclick: move |_| props.on_play.call(0),
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
                      div { class: "grid grid-cols-[auto_1fr_1fr_auto_auto] gap-4 px-2 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2 uppercase tracking-wider",
                           div { class: "flex items-center w-24 shrink-0",
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
                               } else {
                                   "#"
                               }
                           }
                           div { "{i18n::t(\"title\")}" }
                           div { "{i18n::t(\"album\")}" }
                      }

                     for (idx, track) in props.tracks.iter().enumerate() {
                         {
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

                             // Determine if this track is already downloaded for offline playback.
                             // Server tracks have paths like "jellyfin:{item_id}" or "jellyfin:{id}:{tag}"
                             let item_id: Option<String> = {
                                 let s = track.path.to_string_lossy();
                                 if s.starts_with("jellyfin:") {
                                     s.split(':').nth(1).map(|id| id.to_string())
                                 } else {
                                     None
                                 }
                             };
                             let is_downloaded = item_id.as_ref()
                                 .map_or(false, |id| {
                                     if let Some(path_str) = offline_tracks.get(id) {
                                         std::path::Path::new(path_str).exists()
                                     } else {
                                         false
                                     }
                                 });
                             let is_downloading = false; // download_queue context not available in Showcase

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
                                             on_play: move |_| props.on_play.call(idx)
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
