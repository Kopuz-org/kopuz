use crate::constants::{COLUMNS_MODERN, COLUMNS_NORMAL};
use crate::header::Header;
use crate::showcase::{self, SortField};
use crate::track_row::TrackRow;
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use player::player;
use reader::Library;
use reader::models::{Album, Track};

#[component]
pub fn SearchResults(
    search_query: String,
    tracks: Vec<(Track, Option<utils::CoverUrl>)>,
    albums: Vec<(Album, Option<utils::CoverUrl>)>,
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
    on_select_album: EventHandler<String>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();
    let offline_tracks = config.read().offline_tracks.clone();
    let is_modern = config.read().ui_style == UiStyle::Modern;
    let sort_state = use_signal(|| None);
    let sorted_tracks = showcase::sorted_track_pairs(&tracks, *sort_state.read());
    let search_queue: Vec<Track> = sorted_tracks.iter().map(|(t, _)| t.clone()).collect();
    let currently_playing_path = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|track| track.path.clone())
    };
    let current_song_title = ctrl.current_song_title.read().clone();
    let current_song_artist = ctrl.current_song_artist.read().clone();
    let current_song_album = ctrl.current_song_album.read().clone();
    let current_song_duration = *ctrl.current_song_duration.read();

    rsx! {
        div { class: "mt-8 space-y-8 w-full max-w-[1600px] mx-auto select-none pb-8",
            if !tracks.is_empty() {
                div {
                    h2 { class: "text-xl font-semibold text-white/80 mb-4", "{i18n::t(\"tracks\")}" }
                    Header {
                        is_modern: is_modern,
                        is_album: false,
                        sort_state: sort_state
                    }
                    div { class: if is_modern { "" } else { "space-y-2" },
                        for (idx, (track, cover_url)) in sorted_tracks.iter().enumerate() {
                            {
                                let track = track.clone();
                                let track_key = track.path.display().to_string();
                                let track_menu = track.clone();
                                let track_add = track.clone();
                                let track_queue = track.clone();
                                let track_delete = track.clone();
                                let queue_source = search_queue.clone();
                                let matches_current_path = currently_playing_path.as_ref() == Some(&track.path);
                                let matches_current_metadata = currently_playing_path.is_none()
                                    && !current_song_title.is_empty()
                                    && track.title == current_song_title
                                    && track.album == current_song_album
                                    && track.artist == current_song_artist
                                    && track.duration == current_song_duration;
                                let is_currently_playing: bool = matches_current_path || matches_current_metadata;
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
                                        is_album: false,
                                        is_downloaded: is_downloaded,
                                        is_currently_playing,
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

            if !albums.is_empty() {
                div {
                    h2 { class: "text-xl font-semibold text-white/80 mb-4", "{i18n::t(\"albums\")}" }
                    div { class: "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-4",
                        for (album, cover_url) in &albums {
                            {
                                let album_id = album.id.clone();
                                rsx! {
                                    div {
                                        key: "{album_id}",
                                        class: "p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors cursor-pointer group",
                                        onclick: move |_| on_select_album.call(album_id.clone()),
                                        div {
                                            class: "aspect-square rounded-lg bg-black/40 mb-3 overflow-hidden relative",
                                            if let Some(url) = cover_url {
                                                img {
                                                    src: "{url.as_ref()}",
                                                    class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-300",
                                                    decoding: "async", loading: "lazy",
                                                }
                                            } else {
                                                div { class: "w-full h-full flex items-center justify-center",
                                                    i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                                }
                                            }
                                        }
                                        h3 { class: "text-white font-medium truncate", "{album.title}" }
                                        p { class: "text-sm text-slate-400 truncate", "{album.artist}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if tracks.is_empty() && albums.is_empty() {
                div { class: "text-center py-12 text-slate-500",
                    p { "{i18n::t_with(\"no_results_found\", &[(\"query\", search_query.to_string())])}" }
                }
            }
        }
    }
}
