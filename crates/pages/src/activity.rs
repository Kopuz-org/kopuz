use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_db_queries::{use_active_source, use_albums, use_tracks_window};
use hooks::use_player_controller::PlayerController;
use hooks::{Page, TrackFilter, TrackSort};
use kopuz_route::Route;
use reader::Track;
use std::collections::HashMap;
use utils::CoverUrl;

fn format_duration(seconds: u64) -> String {
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}

/// Source-agnostic "listening logs" (most-played). The data path
/// (`use_tracks_window` with a `PlayCount` sort) is already source-scoped; the
/// only per-source bits are the cover (local file vs remote URL), the track
/// origin icon, and the subtitle.
#[component]
pub fn Activity(config: Signal<AppConfig>) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    let source = use_active_source();
    let albums_res = use_albums(source);
    let filter = use_memo(move || TrackFilter {
        source: source(),
        sort: TrackSort::PlayCount,
        search: String::new(),
    });

    // album_id → genre (covers resolve via the source seam off the track itself).
    let album_map = use_memo(move || {
        albums_res
            .read()
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|a| (a.id.clone(), a.genre.clone()))
            .collect::<HashMap<String, String>>()
    });

    let is_vaxry = config.read().ui_style == UiStyle::Vaxry;
    let mut scroll_positions = use_context::<Signal<std::collections::HashMap<Route, f64>>>();
    let saved_scroll = scroll_positions
        .peek()
        .get(&Route::Activity)
        .copied()
        .unwrap_or(0.0);

    let scroll_stat = use_signal(move || saved_scroll);
    let container_height = use_signal(|| 0.0_f64);
    const ITEM_HEIGHT: f64 = 60.0;

    let mut total_rows = use_signal(|| 0_usize);
    let page = use_memo(move || {
        let info = components::virtual_scroll::use_virtual_scroll(
            *scroll_stat.read(),
            *container_height.read(),
            total_rows(),
            ITEM_HEIGHT,
        );
        Page {
            offset: info.start_index as u32,
            limit: info.items_to_render as u32,
        }
    });
    let window = use_tracks_window(filter, page);
    use_effect(move || {
        let total = window.total.read().unwrap_or(0) as usize;
        if *total_rows.peek() != total {
            total_rows.set(total);
        }
    });

    let track_data_len = total_rows();
    let scroll_info = components::virtual_scroll::use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        track_data_len,
        ITEM_HEIGHT,
    );

    let visible_tracks: Vec<(usize, Track, u64, String, Option<CoverUrl>)> = {
        let conf = config.read();
        let albums = album_map.read();
        let window_rows = window.rows.read().clone().unwrap_or_default();
        let row_offset = window_rows.offset as usize;
        window_rows
            .rows
            .into_iter()
            .enumerate()
            .map(|(i, track)| {
                let plays = conf
                    .listen_counts
                    .get(&track.id.uid())
                    .copied()
                    .unwrap_or(0);
                let genre = albums.get(&track.album_id).cloned().unwrap_or_default();
                let cover_url = ::server::cover::track(&conf, &track, 64);
                (row_offset + i, track, plays, genre, cover_url)
            })
            .collect()
    };

    let subtitle = i18n::t("most_played_tracks");

    rsx! {
        div { class: if is_vaxry { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },
            div { class: "max-w-[1600px] mx-auto w-full shrink-0",
                div { class: "mb-8 flex items-end justify-between",
                    div {
                        if is_vaxry {
                            p {
                                class: "text-[10px] font-bold mb-0.5",
                                style: "color: rgba(255,255,255,0.35);",
                                "{i18n::t(\"library\")}"
                            }
                        }
                        h1 { class: if is_vaxry { "text-2xl font-bold text-white mb-1" } else { "text-3xl font-bold text-white mb-2" },
                            "{i18n::t(\"listening_logs\")}"
                        }
                        p { class: "text-slate-400 text-sm", "{subtitle}" }
                    }
                    if !is_vaxry {
                        div {
                            div { class: "w-12 h-12 rounded-full flex items-center justify-center bg-white/5 border border-white/10 text-slate-400",
                                i { class: "fa-solid fa-chart-simple" }
                            }
                        }
                    }
                }

                div { class: "flex items-center px-4 py-3 mb-2 text-xs font-semibold text-slate-400 border-b border-white/10",
                    div { class: "w-12 shrink-0 text-center", "#" }
                    div { class: "flex-1 min-w-0 pr-4", "{i18n::t(\"title\")}" }
                    div { class: "w-48 lg:w-64 shrink-0 hidden md:block pr-4", "{i18n::t(\"album\")}" }
                    div { class: "w-24 shrink-0 hidden lg:block pr-4", "{i18n::t(\"genre\")}" }
                    div { class: "w-24 shrink-0 text-right", "{i18n::t(\"time\")}" }
                    div { class: "w-24 shrink-0 text-right", "{i18n::t(\"plays\")}" }
                }
            }

            div { class: "max-w-[1600px] mx-auto w-full flex-1 min-h-0 flex flex-col",
                components::virtual_scroll::VirtualScrollView {
                    id: "activity-scroll".to_string(),
                    class: "flex-1 min-h-0 overflow-y-auto pb-32".to_string(),
                    scroll_stat,
                    container_height,
                    item_height: ITEM_HEIGHT,
                    saved_scroll,
                    top_pad: scroll_info.top_pad,
                    bottom_pad: scroll_info.bottom_pad,
                    onscroll: move |scroll| {
                        scroll_positions.write().insert(Route::Activity, scroll);
                    },
                    if track_data_len == 0 {
                        if window.total.read().is_none() {
                            div { class: "flex items-center justify-center py-12",
                                i { class: "fa-solid fa-spinner fa-spin text-3xl text-white/20" }
                            }
                        } else {
                            div { class: "flex flex-col items-center justify-center py-24 text-slate-500",
                                i { class: "fa-solid fa-headphones text-4xl mb-4 opacity-50" }
                                p { "{i18n::t(\"no_tracks_in_library\")}" }
                            }
                        }
                    } else {
                        for (idx, track, plays, genre, cover_url) in visible_tracks {
                            {
                                let track_id = track.id.uid();
                                rsx! {
                                    div { key: "{track_id}", style: "height: {ITEM_HEIGHT}px;",
                                        div {
                                            class: "flex items-center h-full px-4 hover:bg-white/5 rounded-xl cursor-pointer transition-colors group",
                                            onclick: move |_| {
                                                let f = filter.peek().clone();
                                                let read_db = consume_context::<hooks::ReadDb>();
                                                spawn(async move {
                                                    let all = read_db
                                                        .tracks_page(&f, Page { offset: 0, limit: u32::MAX })
                                                        .await
                                                        .unwrap_or_default();
                                                    ctrl.queue.set(all);
                                                    ctrl.play_track(idx);
                                                });
                                            },
                                            div { class: "w-12 shrink-0 flex items-center justify-center tabular-nums text-white/50 font-medium group-hover:text-white transition-colors relative",
                                                span { class: "group-hover:opacity-0 transition-opacity", "{idx + 1}" }
                                                i { class: "fa-solid fa-play absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 opacity-0 group-hover:opacity-100 transition-opacity" }
                                            }

                                            div { class: "flex-1 min-w-0 pr-4 flex items-center",
                                                div {
                                                    class: if is_vaxry { "w-8 h-8 bg-white/5 rounded-md flex items-center justify-center mr-4 shrink-0 text-slate-500 group-hover:text-slate-300 transition-colors overflow-hidden" } else { "w-10 h-10 bg-white/5 rounded-md flex items-center justify-center mr-4 shrink-0 text-slate-500 group-hover:text-slate-300 transition-colors overflow-hidden" },
                                                    if let Some(url) = cover_url {
                                                        img { src: "{url.as_ref()}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
                                                    } else {
                                                        i { class: "fa-solid fa-music text-xs" }
                                                    }
                                                }
                                                div { class: "flex-1 min-w-0",
                                                    div {
                                                        class: if is_vaxry { "text-white font-medium truncate text-sm" } else { "text-white font-medium truncate text-[15px] mb-0.5" },
                                                        "{track.title}"
                                                    }
                                                    div {
                                                        class: if is_vaxry { "text-slate-400 text-xs truncate group-hover:text-slate-300 transition-colors" } else { "text-slate-400 text-sm truncate group-hover:text-slate-300 transition-colors" },
                                                        "{track.artist}"
                                                    }
                                                }
                                            }

                                            div { class: "w-48 lg:w-64 shrink-0 hidden md:block text-slate-400 text-sm truncate pr-4 group-hover:text-slate-300 transition-colors",
                                                "{track.album}"
                                            }

                                            div { class: "w-24 shrink-0 hidden lg:block text-slate-400 text-sm truncate pr-4 group-hover:text-slate-300 transition-colors",
                                                if genre.is_empty() {
                                                    "-"
                                                } else {
                                                    "{genre}"
                                                }
                                            }

                                            div { class: "w-24 shrink-0 text-right text-slate-400 text-sm tabular-nums group-hover:text-slate-300 transition-colors",
                                                "{format_duration(track.duration)}"
                                            }

                                            div { class: "w-24 shrink-0 text-right text-slate-400 text-sm tabular-nums group-hover:text-slate-300 transition-colors flex items-center justify-end gap-2",
                                                if plays > 0 {
                                                    i { class: "fa-solid fa-fire text-orange-500/80 text-[10px]" }
                                                }
                                                span { class: if plays > 0 { "text-white font-medium" } else { "" }, "{plays}" }
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
