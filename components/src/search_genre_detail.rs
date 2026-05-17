use crate::showcase::{self, SortField};
use crate::track_row::TrackRow;
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use player::player;
use reader::Library;
use reader::models::Track;

#[component]
pub fn SearchGenreDetail(
    genre: String,
    genre_tracks: Vec<(Track, Option<utils::CoverUrl>)>,
    genres: Vec<(String, Option<utils::CoverUrl>)>,
    on_back: EventHandler<()>,
    library: Signal<Library>,
    playlist_store: Signal<reader::PlaylistStore>,
    player: Signal<player::Player>,
    mut is_playing: Signal<bool>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    mut queue: Signal<Vec<Track>>,
    mut current_queue_index: Signal<usize>,
    mut active_menu_track: Signal<Option<std::path::PathBuf>>,
    mut show_playlist_modal: Signal<bool>,
    mut selected_track_for_playlist: Signal<Option<std::path::PathBuf>>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();
    let offline_tracks = config.read().offline_tracks.clone();
    let is_modern = config.read().ui_style == UiStyle::Modern;
    let sort_state = use_signal(|| None);
    let sorted_genre_tracks = showcase::sorted_track_pairs(&genre_tracks, *sort_state.read());
    let genre_tracks_list: Vec<Track> =
        sorted_genre_tracks.iter().map(|(t, _)| t.clone()).collect();

    rsx! {
        div {
            class: "space-y-6",
            button {
                class: "mb-4 flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                 onclick: move |_| on_back.call(()),
                 i { class: "fa-solid fa-arrow-left" }
                 "{i18n::t(\"back_to_browse\")}"
            }

            if is_modern {
                div { class: "flex items-end gap-6 mb-8",
                    div {
                        class: "w-44 h-44 rounded-2xl overflow-hidden shrink-0 shadow-2xl bg-white/5",
                        style: "box-shadow: 0 20px 60px rgba(0,0,0,0.6);",
                        if let Some((_, Some(url))) = genres.iter().find(|(g, _)| g == &genre) {
                            img { src: "{url.as_ref()}", class: "w-full h-full object-cover" }
                        } else {
                            div { class: "w-full h-full flex items-center justify-center",
                                i { class: "fa-solid fa-music text-4xl", style: "color: rgba(255,255,255,0.15);" }
                            }
                        }
                    }
                    div { class: "flex flex-col gap-1 pb-1 min-w-0",
                        p {
                            class: "text-xs font-bold tracking-widest uppercase mb-1",
                            style: "color: rgba(255,255,255,0.35);",
                            "{i18n::t(\"genre\")}"
                        }
                        h1 { class: "text-4xl font-bold text-white truncate mb-1", "{genre}" }
                        p {
                            class: "text-sm mb-3",
                            style: "color: rgba(255,255,255,0.45);",
                            {
                                if genre_tracks.len() == 1 {
                                    i18n::t("track_count_singular").to_string()
                                } else {
                                    i18n::t_with("track_count", &[("count", genre_tracks.len().to_string())])
                                }
                            }
                        }
                        div { class: "flex items-center gap-2 flex-wrap",
                            if !genre_tracks.is_empty() {
                                button {
                                    class: "inline-flex items-center justify-center gap-2 h-9 px-5 rounded-full text-sm font-semibold text-white transition-opacity hover:opacity-90 active:scale-95",
                                    style: "background: var(--color-indigo-500);",
                                    onclick: {
                                        let tracks_play = genre_tracks_list.clone();
                                        move |_| {
                                            let is_shuffle = *ctrl.shuffle.peek();
                                            if is_shuffle {
                                                ctrl.play_queue_shuffled(tracks_play.clone());
                                            } else {
                                                ctrl.play_queue_linear(tracks_play.clone());
                                            }
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
                                    onclick: {
                                        let tracks_shuffle = genre_tracks_list.clone();
                                        move |_| {
                                            ctrl.toggle_shuffle();
                                            ctrl.play_queue_shuffled(tracks_shuffle.clone());
                                        }
                                    },
                                    i { class: "fa-solid fa-shuffle text-xs" }
                                    "{i18n::t(\"shuffle\")}"
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "flex items-end gap-6 mb-8",
                     if let Some((_, Some(url))) = genres.iter().find(|(g, _)| g == &genre) {
                         img { src: "{url.as_ref()}", class: "w-48 h-48 rounded-lg object-cover" }
                     } else {
                         div { class: "w-48 h-48 rounded-lg bg-gradient-to-br flex items-center justify-center",
                             i { class: "fa-solid fa-music text-6xl text-white/20" }
                         }
                     }

                     div {
                         h2 { class: "text-sm font-bold text-white/60 uppercase tracking-widest mb-2", "{i18n::t(\"genre\")}" }
                         h1 { class: "text-5xl font-bold text-white mb-4", "{genre}" }
                         p { class: "text-slate-400",
                             {
                                 if genre_tracks.len() == 1 {
                                     i18n::t("track_count_singular").to_string()
                                 } else {
                                     i18n::t_with("track_count", &[("count", genre_tracks.len().to_string())])
                                 }
                             }
                         }
                     }
                }
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
                div { "#" }
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
            div { class: if is_modern { "pb-20" } else { "space-y-1 pb-20" },
                 for (idx, (track, cover_url)) in sorted_genre_tracks.iter().enumerate() {
                     {
                         let track = track.clone();
                         let track_key = track.path.display().to_string();
                         let track_menu = track.clone();
                         let track_add = track.clone();
                         let track_queue = track.clone();
                         let track_delete = track.clone();
                         let queue_source = genre_tracks_list.clone();
                         let is_menu_open = active_menu_track.read().as_ref() == Some(&track.path);
                         let item_id: Option<String> = {
                             let s = track.path.to_string_lossy();
                             if s.starts_with("jellyfin:") {
                                 s.split(':').nth(1).map(|id| id.to_string())
                             } else { None }
                         };
                         let is_downloaded = item_id
                             .as_ref()
                             .map_or(false, |id| {
                                 if let Some(path_str) = offline_tracks.get(id) {
                                     std::path::Path::new(path_str).exists()
                                 } else {
                                     false
                                 }
                             });

                         rsx! {
                             TrackRow {
                                 key: "{track_key}",
                                 track: track.clone(),
                                 cover_url: cover_url.clone(),
                                 row_num: Some(idx + 1),
                                 is_menu_open: is_menu_open,
                                 is_downloaded: is_downloaded,
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
                                         let lib_path = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
                                             .map(|d| d.config_dir().join("library.json"))
                                             .unwrap_or_else(|| std::path::PathBuf::from("./config/library.json"));
                                         let _ = library.read().save(&lib_path);
                                     }
                                 },
                                 on_play: move |_| {
                                     queue.set(queue_source.clone());
                                     ctrl.play_track(idx);
                                 }
                             }
                         }
                     }
                 }
            }
        }
    }
}
