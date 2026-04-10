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
    pub cover_url: Option<String>,
    pub tracks: Vec<Track>,
    pub library: Signal<Library>,
    pub on_play: EventHandler<usize>,
    pub on_add_to_playlist: Option<EventHandler<usize>>,
    pub on_delete_track: Option<EventHandler<usize>>,
    pub on_remove_from_playlist: Option<EventHandler<usize>>,
    pub active_track: Option<std::path::PathBuf>,
    pub on_click_menu: Option<EventHandler<usize>>,
    pub on_close_menu: Option<EventHandler<()>>,
    pub actions: Option<Element>,
    #[props(default = false)]
    pub is_selection_mode: bool,
    #[props(default = HashSet::new())]
    pub selected_tracks: HashSet<PathBuf>,
    pub on_select: Option<EventHandler<(usize, bool)>>,
    pub on_long_press: Option<EventHandler<usize>>,
}

#[component]
pub fn Showcase(props: ShowcaseProps) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let total_seconds: u64 = props.tracks.iter().map(|t| t.duration).sum();
    let duration_min = total_seconds / 60;

    let lib = props.library.read();
    let is_server_source = config.read().active_source == MusicSource::Server;

    rsx! {
         div {
             class: "select-none",
             div {
                 class: "flex flex-col md:flex-row items-end gap-8 mb-12",
                 div { class: "w-64 h-64 rounded-xl bg-stone-800 overflow-hidden relative flex-shrink-0",
                     if let Some(url) = &props.cover_url {
                         img { src: "{url}", class: "w-full h-full object-cover" }
                     } else {
                         div { class: "w-full h-full flex flex-col items-center justify-center text-white/20",
                             i { class: "fa-solid fa-music text-6xl mb-4" }
                         }
                     }
                 }
                 div { class: "flex-1",
                     if !props.description.is_empty() {
                         h5 { class: "text-sm font-bold tracking-widest text-white/60 uppercase mb-2", "{props.description}" }
                     }
                     h1 { class: "text-5xl md:text-7xl font-bold text-white mb-6", "{props.name}" }
                     div { class: "flex items-center gap-6 text-slate-400",
                         p { "{props.tracks.len()} songs" }
                         span { "•" }
                         p { "{duration_min} min" }
                     }
                 }

                div { class: "flex items-center gap-4",
                     if !props.tracks.is_empty() {
                         button {
                             class: "w-14 h-14 rounded-full bg-indigo-500 hover:bg-indigo-400 text-black flex items-center justify-center transition-transform hover:scale-105",
                             onclick: move |_| props.on_play.call(0),
                             i { class: "fa-solid fa-play text-xl ml-1" }
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
                         p { class: "text-lg", "No songs here." }
                     }
                 } else {
                     div { class: "grid grid-cols-[auto_1fr_1fr_auto_auto] gap-4 px-4 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2 uppercase tracking-wider",
                          div { class: "w-8 text-center", "#" }
                          div { "Title" }
                          div { "Album" }
                     }

                     for (idx, track) in props.tracks.iter().enumerate() {
                         {
                             let cover_url = if is_server_source {
                                 if let Some(server) = &config.read().server {
                                     let path_str = track.path.to_string_lossy();
                                     match server.service {
                                         MusicService::Jellyfin => {
                                             utils::jellyfin_image::jellyfin_image_url_from_path(
                                                 &path_str,
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
                                     }
                                 } else { None }
                             } else {
                                 lib.albums.iter()
                                    .find(|a| a.id == track.album_id)
                                    .and_then(|a| utils::format_artwork_url(a.cover_path.as_ref()))
                             };

                             let is_selected = props.selected_tracks.contains(&track.path);

                             rsx! {
                                 TrackRow {
                                     key: "{track.path.display()}",
                                     track: track.clone(),
                                     cover_url: cover_url,
                                     is_menu_open: props.active_track.as_ref() == Some(&track.path),
                                     is_selection_mode: props.is_selection_mode,
                                     is_selected: is_selected,
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
                                     on_play: move |_| props.on_play.call(idx)
                                 }
                             }
                         }
                     }
                 }
             }
         }
    }
}
