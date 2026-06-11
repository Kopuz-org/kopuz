use crate::header::Header;
use crate::showcase::{self};
use crate::track_row::TrackRow;
use crate::virtual_scroll::{VirtualScrollView, use_virtual_scroll};
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
    let gens = hooks::db_reactivity::use_generations();
    let offline_tracks = config.read().offline_tracks.clone();
    let is_modern = config.read().ui_style == UiStyle::Modern;
    let sort_state = use_signal(|| None);
    let sorted_genre_tracks = showcase::sorted_track_pairs(&genre_tracks, *sort_state.read());
    let genre_tracks_list: Vec<Track> =
        sorted_genre_tracks.iter().map(|(t, _)| t.clone()).collect();
    let currently_playing_path = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|track| track.id.uid_path())
    };
    let current_song_title = ctrl.current_song_title.read().clone();
    let current_song_artist = ctrl.current_song_artist.read().clone();
    let current_song_album = ctrl.current_song_album.read().clone();
    let current_song_duration = *ctrl.current_song_duration.read();

    let scroll_stat = use_signal(|| 0.0_f64);
    let container_height = use_signal(|| 0.0_f64);
    const ITEM_HEIGHT: f64 = 60.0;

    let scroll_info = use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        sorted_genre_tracks.len(),
        ITEM_HEIGHT,
    );

    rsx! {
            div {
                class: "flex-1 min-h-0 flex flex-col w-full max-w-[1600px] mx-auto select-none",
                div { class: "shrink-0 mb-6",
                if !cfg!(target_os = "android") {
                    button {
                        class: "mb-4 flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                         onclick: move |_| on_back.call(()),
                         i { class: "fa-solid fa-arrow-left" }
                         "{i18n::t(\"back_to_browse\")}"
                    }
                }

                if is_modern {
                    div { class: "flex items-end gap-6 mb-8 shrink-0",
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
                    div { class: "flex items-end gap-6 mb-8 shrink-0",
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
                }
                div { class: "shrink-0 mb-4",
                    Header{
                        is_modern: is_modern,
                        is_album: false,
                        sort_state: sort_state
                    }
                }
                div { class: "flex-1 min-h-0 w-full flex flex-col overflow-hidden",
                     VirtualScrollView {
                         id: "genre-tracks-scroll".to_string(),
                         class: "flex-1 min-h-0 overflow-y-auto pb-20".to_string(),
                         scroll_stat,
                         container_height,
                         item_height: ITEM_HEIGHT,
                         saved_scroll: 0.0,
                         top_pad: scroll_info.top_pad,
                         bottom_pad: scroll_info.bottom_pad,
                         for (idx, (track, cover_url)) in sorted_genre_tracks.iter().enumerate().skip(scroll_info.start_index).take(scroll_info.items_to_render) {
                         {
                             let track = track.clone();
                             let track_key = track.id.uid();
                             let track_menu = track.clone();
                             let track_add = track.clone();
                             let track_queue = track.clone();
                             let track_delete = track.clone();
                             let queue_source = genre_tracks_list.clone();
                             let matches_current_path = currently_playing_path.as_ref() == Some(&track.id.uid_path());
                             let matches_current_metadata = currently_playing_path.is_none()
                                 && !current_song_title.is_empty()
                                 && track.title == current_song_title
                                 && track.artist == current_song_artist
                                 && track.album == current_song_album
                                 && track.duration == current_song_duration;
                             let is_currently_playing: bool = matches_current_path || matches_current_metadata;
                             let is_menu_open = active_menu_track.read().as_ref() == Some(&track.id.uid_path());
                             let item_id: Option<String> = {
                                 let s = track.id.uid();
                                 if s.starts_with("jellyfin:") {
                                     s.split(':').nth(1).map(|id| id.to_string())
                                 } else { None }
                             };
                             let is_downloaded = item_id
                                 .as_ref()
                                 .is_some_and(|id| {
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
                                     is_album: false,
                                     is_downloaded: is_downloaded,
                                     is_currently_playing,
                                     on_click_menu: move |_| {
                                         if active_menu_track.read().as_ref() == Some(&track_menu.id.uid_path()) {
                                             active_menu_track.set(None);
                                         } else {
                                             active_menu_track.set(Some(track_menu.id.uid_path()));
                                         }
                                     },
                                     on_add_to_playlist: move |_| {
                                         selected_track_for_playlist.set(Some(track_add.id.uid_path()));
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
                                         if let Some(del_path) = track_delete.id.local_path()
                                             && std::fs::remove_file(del_path).is_ok()
                                         {
                                             let db = consume_context::<db::Db>();
                                             let key = track_delete.id.key().into_owned();
                                             spawn(async move {
                                                 if db
                                                     .delete_tracks(&db::Source::Local, &[key])
                                                     .await
                                                     .is_ok()
                                                 {
                                                     gens.bump(hooks::db_reactivity::Table::Tracks);
                                                 }
                                             });
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
}
