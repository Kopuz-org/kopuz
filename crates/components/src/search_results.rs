use crate::header::Header;
use crate::showcase::{self};
use crate::track_row::TrackRow;
use crate::virtual_scroll::{VirtualScrollView, use_virtual_scroll};
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use player::player;
use reader::models::{Album, Track};

#[component]
pub fn SearchResults(
    search_query: String,
    tracks: Vec<(Track, Option<utils::CoverUrl>)>,
    albums: Vec<(Album, Option<utils::CoverUrl>)>,
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
    let gens = hooks::db_reactivity::use_generations();
    let offline_tracks = config.read().offline_tracks.clone();
    let is_modern = config.read().ui_style == UiStyle::Modern;
    let sort_state = use_signal(|| None);
    let sorted_tracks = showcase::sorted_track_pairs(&tracks, *sort_state.read());
    let search_queue: Vec<Track> = sorted_tracks.iter().map(|(t, _)| t.clone()).collect();
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
        sorted_tracks.len(),
        ITEM_HEIGHT,
    );

    rsx! {
        div { class: "mt-8 w-full max-w-[1600px] mx-auto select-none flex-1 min-h-0 flex flex-col",
            if !tracks.is_empty() {
                div { class: "shrink-0 mb-4",
                    h2 { class: "text-xl font-semibold text-white/80 mb-4", "{i18n::t(\"tracks\")}" }
                    Header {
                        is_modern: is_modern,
                        is_album: false,
                        sort_state: sort_state
                    }
                }
                div { class: "flex-1 min-h-0 w-full flex flex-col overflow-hidden",
                    VirtualScrollView {
                        id: "search-tracks-scroll".to_string(),
                        class: "flex-1 min-h-0 overflow-y-auto pb-20".to_string(),
                        scroll_stat,
                        container_height,
                        item_height: ITEM_HEIGHT,
                        saved_scroll: 0.0,
                        top_pad: scroll_info.top_pad,
                        bottom_pad: scroll_info.bottom_pad,
                        bottom_content: rsx! {
                            if !albums.is_empty() {
                                div { class: "mt-12",
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
                        },
                        for (idx, (track, cover_url)) in sorted_tracks.iter().enumerate().skip(scroll_info.start_index).take(scroll_info.items_to_render) {
                            {
                                let track = track.clone();
                                let track_key = track.id.uid();
                                let track_menu = track.clone();
                                let track_add = track.clone();
                                let track_queue = track.clone();
                                let track_delete = track.clone();
                                let queue_source = search_queue.clone();
                                let matches_current_path = currently_playing_path.as_ref() == Some(&track.id.uid_path());
                                let matches_current_metadata = currently_playing_path.is_none()
                                    && !current_song_title.is_empty()
                                    && track.title == current_song_title
                                    && track.album == current_song_album
                                    && track.artist == current_song_artist
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
            } else if !albums.is_empty() {
                div { class: "flex-1 overflow-y-auto pb-20",
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
