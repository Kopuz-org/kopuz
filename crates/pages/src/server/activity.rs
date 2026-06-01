use config::{AppConfig, MusicService, UiStyle};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use kopuz_route::Route;
use reader::{Library, Track};
use std::collections::HashMap;

fn format_duration(seconds: u64) -> String {
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}

#[component]
pub fn JellyfinLogs(library: Signal<Library>, config: Signal<AppConfig>) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    let track_data = use_memo(move || {
        let lib = library.read();
        let conf = config.read();

        let album_genre_map: HashMap<String, String> = lib
            .jellyfin_albums
            .iter()
            .map(|a| (a.id.clone(), a.genre.clone()))
            .collect();

        let cover_url_base = conf
            .server
            .as_ref()
            .map(|s| (s.url.clone(), s.access_token.clone()));

        let mut all_tracks = lib.jellyfin_tracks.clone();

        all_tracks.sort_by(|a, b| {
            let a_plays = conf
                .listen_counts
                .get(&a.path.to_string_lossy().to_string())
                .copied()
                .unwrap_or(0);
            let b_plays = conf
                .listen_counts
                .get(&b.path.to_string_lossy().to_string())
                .copied()
                .unwrap_or(0);

            match b_plays.cmp(&a_plays) {
                std::cmp::Ordering::Equal => a.title.cmp(&b.title),
                other => other,
            }
        });

        all_tracks
            .into_iter()
            .map(|track| {
                let track_id = track.path.to_string_lossy().to_string();
                let plays = conf.listen_counts.get(&track_id).copied().unwrap_or(0);
                let genre = album_genre_map
                    .get(&track.album_id)
                    .cloned()
                    .unwrap_or_default();
                let cover_url = if let Some((ref base_url, ref token)) = cover_url_base {
                    utils::map_cover_url(
                        utils::jellyfin_image::track_cover_url_with_album_fallback(
                            &track.path.to_string_lossy(),
                            &track.album_id,
                            base_url,
                            token.as_deref(),
                            64,
                            90,
                        ),
                    )
                } else {
                    None
                };
                (track, plays, genre, cover_url)
            })
            .collect::<Vec<(Track, u64, String, Option<utils::CoverUrl>)>>()
    });

    let queue_tracks = use_memo(move || {
        std::sync::Arc::new(
            track_data
                .read()
                .iter()
                .map(|(t, _, _, _)| t.clone())
                .collect::<Vec<_>>(),
        )
    });

    let is_modern = config.read().ui_style == UiStyle::Modern;
    let mut scroll_positions = use_context::<Signal<std::collections::HashMap<Route, f64>>>();
    let saved_scroll = scroll_positions
        .peek()
        .get(&Route::Activity)
        .copied()
        .unwrap_or(0.0);

    let scroll_stat = use_signal(|| 0.0_f64);
    let container_height = use_signal(|| 0.0_f64);
    const ITEM_HEIGHT: f64 = 60.0;

    let track_data_len = track_data.read().len();
    let scroll_info = components::virtual_scroll::use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        track_data_len,
        ITEM_HEIGHT,
    );

    let visible_tracks: Vec<(usize, Track, u64, String, Option<utils::CoverUrl>)> = track_data
        .read()
        .iter()
        .enumerate()
        .skip(scroll_info.start_index)
        .take(scroll_info.items_to_render)
        .map(|(idx, (track, plays, genre, cover_url))| {
            (idx, track.clone(), *plays, genre.clone(), cover_url.clone())
        })
        .collect();

    rsx! {
        div { class: if is_modern { "px-6 pt-6 pb-24 absolute inset-0 flex flex-col" } else { "p-8 absolute inset-0 flex flex-col" },
            div { class: "max-w-[1600px] mx-auto w-full shrink-0",
                div { class: "mb-8 flex items-end justify-between",
                    div {
                        if is_modern {
                            p {
                                class: "text-[10px] font-bold tracking-widest uppercase mb-0.5",
                                style: "color: rgba(255,255,255,0.35);",
                                "{i18n::t(\"library\")}"
                            }
                        }
                        h1 { class: if is_modern { "text-2xl font-bold text-white mb-1" } else { "text-3xl font-bold text-white mb-2" },
                            "{i18n::t(\"listening_logs\")}"
                        }
                        p { class: "text-slate-400 text-sm", "{i18n::t(\"most_played_tracks\")}" }
                    }
                    if !is_modern {
                        div {
                            div { class: "w-12 h-12 rounded-full flex items-center justify-center bg-white/5 border border-white/10 text-slate-400",
                                i { class: "fa-solid fa-chart-simple" }
                            }
                        }
                    }
                }

                div { class: "flex items-center px-4 py-3 mb-2 text-xs font-semibold tracking-wider text-slate-400 uppercase border-b border-white/10",
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
                        div { class: "flex flex-col items-center justify-center py-24 text-slate-500",
                            i { class: "fa-solid fa-headphones text-4xl mb-4 opacity-50" }
                            p { "{i18n::t(\"no_tracks_in_library\")}" }
                        }
                    } else {
                        for (idx, track, plays, genre, cover_url) in visible_tracks {
                            {
                                let track_id = track.path.to_string_lossy().to_string();
                                let queue = std::sync::Arc::clone(&*queue_tracks.read());
                                rsx! {
                                    div { key: "{track_id}", style: "height: {ITEM_HEIGHT}px;",
                                        div {
                                            class: "flex items-center h-full px-4 hover:bg-white/5 rounded-xl cursor-pointer transition-colors group",
                                            onclick: move |_| {
                                                ctrl.queue.set((*queue).clone());
                                                ctrl.play_track(idx);
                                            },
                                            div { class: "w-12 shrink-0 flex items-center justify-center tabular-nums text-slate-500 font-medium group-hover:text-white transition-colors relative",
                                                span { class: "group-hover:opacity-0 transition-opacity", "{idx + 1}" }
                                                i { class: "fa-solid fa-play absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 opacity-0 group-hover:opacity-100 transition-opacity" }
                                            }

                                            div { class: "flex-1 min-w-0 pr-4 flex items-center",
                                                div {
                                                    class: if is_modern { "w-8 h-8 bg-white/5 rounded-md flex items-center justify-center mr-4 shrink-0 text-slate-500 group-hover:text-slate-300 transition-colors overflow-hidden" } else { "w-10 h-10 bg-white/5 rounded-md flex items-center justify-center mr-4 shrink-0 text-slate-500 group-hover:text-slate-300 transition-colors overflow-hidden" },
                                                    if let Some(url) = cover_url {
                                                        img { src: "{url.as_ref()}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
                                                    } else {
                                                        i { class: "fa-solid fa-music text-xs" }
                                                    }
                                                }
                                                div { class: "flex-1 min-w-0",
                                                    div {
                                                        class: if is_modern { "text-white font-medium truncate text-sm flex items-center gap-2" } else { "text-white font-medium truncate text-[15px] mb-0.5 flex items-center gap-2" },
                                                        "{track.title}"
                                                        i {
                                                            class: "fa-solid fa-database text-[10px] text-slate-500",
                                                            title: i18n::t("server").to_string(),
                                                        }
                                                    }
                                                    div {
                                                        class: if is_modern { "text-slate-400 text-xs truncate group-hover:text-slate-300 transition-colors" } else { "text-slate-400 text-sm truncate group-hover:text-slate-300 transition-colors" },
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

#[component]
pub fn ServerLogs(library: Signal<Library>, config: Signal<AppConfig>) -> Element {
    let service = config
        .read()
        .active_service()
        .unwrap_or(MusicService::Jellyfin);

    match service {
        MusicService::Jellyfin | MusicService::Subsonic | MusicService::Custom => rsx! {
            JellyfinLogs { library, config }
        },
    }
}
