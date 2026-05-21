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
                let cover_url = cover_url_base
                    .as_ref()
                    .map(|(url, token)| {
                        utils::jellyfin_image::track_cover_url_with_album_fallback(
                            &track_id,
                            &track.album_id,
                            url,
                            token.as_deref(),
                            80,
                            80,
                        )
                    })
                    .flatten();
                (track, plays, genre, cover_url)
            })
            .collect::<Vec<(Track, u64, String, Option<String>)>>()
    });

    let is_modern = config.read().ui_style == UiStyle::Modern;
    let mut scroll_positions = use_context::<Signal<std::collections::HashMap<Route, f64>>>();
    let saved_scroll = scroll_positions
        .peek()
        .get(&Route::Activity)
        .copied()
        .unwrap_or(0.0);

    rsx! {
        div {
            id: "activity-scroll",
            class: if is_modern { "px-6 pt-6 pb-24 h-full overflow-y-auto w-full" } else { "p-8 h-full overflow-y-auto w-full" },
            onmounted: move |_| {
                if saved_scroll > 0.0 {
                    let _ = dioxus::document::eval(&format!(
                        "let el = document.getElementById('activity-scroll'); if (el) el.scrollTop = {saved_scroll};"
                    ));
                }
            },
            onscroll: move |evt| {
                scroll_positions.write().insert(Route::Activity, evt.scroll_top());
            },
            div { class: "max-w-[1600px] mx-auto",
                div { class: "mb-8 flex items-end justify-between",
                    div {
                        if is_modern {
                            p {
                                class: "text-[10px] font-bold tracking-widest uppercase mb-0.5",
                                style: "color: rgba(255,255,255,0.35);",
                                "{i18n::t(\"library\")}"
                            }
                        }
                        h1 {
                            class: if is_modern { "text-2xl font-bold text-white mb-1" } else { "text-3xl font-bold text-white mb-2" },
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
                    div { class: "flex-1 min-w-0 pl-14 pr-4", "{i18n::t(\"title\")}" }
                    div { class: "w-48 lg:w-64 shrink-0 hidden md:block pr-4", "{i18n::t(\"album\")}" }
                    div { class: "w-24 shrink-0 hidden lg:block pr-4", "{i18n::t(\"genre\")}" }
                    div { class: "w-24 shrink-0 text-right", "{i18n::t(\"time\")}" }
                    div { class: "w-24 shrink-0 text-right", "{i18n::t(\"plays\")}" }
                }

                div { class: "flex flex-col pb-32 space-y-1",
                    for (idx, (track, plays, genre, cover_url)) in track_data.read().iter().enumerate() {
                        {
                            let track_id = track.path.to_string_lossy().to_string();
                            let plays = *plays;
                            let genre = genre.clone();
                            let cover_url = cover_url.clone();

                            rsx! {
                                div {
                                    key: "{track_id}",
                                    class: "flex items-center px-4 py-2 hover:bg-white/5 rounded-xl cursor-pointer transition-colors group",
                                    onclick: move |_| {
                                        ctrl.queue.set(track_data.read().iter().map(|(t, _, _, _)| t.clone()).collect());
                                        ctrl.play_track(idx);
                                    },
                                    div { class: "w-12 shrink-0 flex items-center justify-center tabular-nums text-slate-500 font-medium group-hover:text-white transition-colors relative",
                                        span { class: "group-hover:opacity-0 transition-opacity", "{idx + 1}" }
                                        i { class: "fa-solid fa-play absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 opacity-0 group-hover:opacity-100 transition-opacity" }
                                    }

                                    div { class: "flex-1 min-w-0 pr-4 flex items-center",
                                        div { class: "w-10 h-10 bg-white/5 rounded-md flex items-center justify-center mr-4 shrink-0 text-slate-500 group-hover:text-slate-300 transition-colors overflow-hidden",
                                            if let Some(url) = cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover" }
                                            } else {
                                                i { class: "fa-solid fa-music text-xs" }
                                            }
                                        }
                                        div { class: "flex-1 min-w-0",
                                            div { class: "text-white font-medium truncate text-[15px] mb-0.5 flex items-center gap-2",
                                                "{track.title}"
                                                i {
                                                    class: "fa-solid fa-database text-[10px] text-slate-500",
                                                    title: i18n::t("server").to_string()
                                                }
                                            }
                                            div { class: "text-slate-400 text-sm truncate group-hover:text-slate-300 transition-colors", "{track.artist}" }
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
                    if track_data.read().is_empty() {
                        div { class: "flex flex-col items-center justify-center py-24 text-slate-500",
                            i { class: "fa-solid fa-headphones text-4xl mb-4 opacity-50" }
                            p { "{i18n::t(\"no_tracks_in_library\")}" }
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
