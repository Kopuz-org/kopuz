use config::{AppConfig, MusicService};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use reader::Library;

fn format_duration(seconds: u64) -> String {
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}

#[component]
pub fn JellyfinLogs(library: Signal<Library>, config: Signal<AppConfig>) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    let sorted_tracks = use_memo(move || {
        let lib = library.read();
        let conf = config.read();

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
    });

    let conf = config.read();

    rsx! {
        div { class: "p-8 h-full overflow-y-auto w-full",
            div { class: "max-w-[1600px] mx-auto",
                div { class: "mb-8 flex items-end justify-between",
                    div {
                        h1 { class: "text-3xl font-bold text-white mb-2", "Listening Logs" }
                        p { class: "text-slate-400 text-sm", "Your most played tracks." }
                    }
                    div {
                        div { class: "w-12 h-12 rounded-full flex items-center justify-center bg-white/5 border border-white/10 text-slate-400",
                            i { class: "fa-solid fa-chart-simple" }
                        }
                    }
                }

                div { class: "flex items-center px-4 py-3 mb-2 text-xs font-semibold tracking-wider text-slate-400 uppercase border-b border-white/10",
                    div { class: "w-12 shrink-0 text-center", "#" }
                    div { class: "flex-1 min-w-0 pl-14 pr-4", "Title" }
                    div { class: "w-48 lg:w-64 shrink-0 hidden md:block pr-4", "Album" }
                    div { class: "w-24 shrink-0 hidden lg:block pr-4", "Genre" }
                    div { class: "w-24 shrink-0 text-right", "Time" }
                    div { class: "w-24 shrink-0 text-right", "Plays" }
                }

                div { class: "flex flex-col pb-32 space-y-1",
                    for (idx, track) in sorted_tracks.read().iter().enumerate() {
                        {
                            let track_id = track.path.to_string_lossy().to_string();
                            let plays = conf.listen_counts.get(&track_id).copied().unwrap_or(0);

                            let genre = library.read().jellyfin_albums.iter()
                                .find(|a| a.id == track.album_id)
                                .map(|a| a.genre.clone())
                                .unwrap_or_default();

                            let cover_url = {
                                if let Some(server) = &conf.server {
                                    let path_str = track.path.to_string_lossy();
                                    utils::jellyfin_image::track_cover_url_with_album_fallback(
                                        &path_str,
                                        &track.album_id,
                                        &server.url,
                                        server.access_token.as_deref(),
                                        80,
                                        80,
                                    )
                                } else {
                                    None
                                }
                            };

                            rsx! {
                                div {
                                    key: "{track_id}",
                                    class: "flex items-center px-4 py-2 hover:bg-white/5 rounded-xl cursor-pointer transition-colors group",
                                    onclick: move |_| {
                                        ctrl.queue.set(sorted_tracks.read().clone());
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
                                                    title: "Server"
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
                    if sorted_tracks.read().is_empty() {
                        div { class: "flex flex-col items-center justify-center py-24 text-slate-500",
                            i { class: "fa-solid fa-headphones text-4xl mb-4 opacity-50" }
                            p { "No tracks found in your music library." }
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
