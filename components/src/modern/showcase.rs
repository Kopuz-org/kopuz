use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

use crate::showcase::{self, ShowcaseProps, SortField};
use crate::NavigationController;

#[component]
pub fn ShowcaseModern(props: ShowcaseProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();
    let nav_ctrl = use_context::<NavigationController>();

    let total_seconds: u64 = props.tracks.iter().map(|t| t.duration).sum();
    let duration_min = total_seconds / 60;

    let fmt_dur = |s: u64| format!("{}:{:02}", s / 60, s % 60);
    let sort_state = use_signal(|| None);
    let indexed_tracks: Vec<_> = props
        .tracks
        .iter()
        .cloned()
        .enumerate()
        .map(|(idx, track)| (track, idx))
        .collect();
    let sorted_track_pairs = showcase::sorted_track_pairs(&indexed_tracks, *sort_state.read());
    let sorted_tracks: Vec<_> = sorted_track_pairs
        .iter()
        .map(|(track, _)| track.clone())
        .collect();
    let tracks_for_shuffle = sorted_tracks.clone();

    let currently_playing_path = {
        let queue = ctrl.queue.read();
        let idx = *ctrl.current_queue_index.read();
        let queue_idx = if *ctrl.shuffle.read() {
            ctrl.shuffle_order.read().get(idx).copied().unwrap_or(idx)
        } else {
            idx
        };
        queue.get(queue_idx).map(|track| track.path.clone())
    };
    let current_song_title = ctrl.current_song_title.read().clone();
    let current_song_artist = ctrl.current_song_artist.read().clone();
    let current_song_album = ctrl.current_song_album.read().clone();
    let current_song_duration = *ctrl.current_song_duration.read();
    let tracks_for_play_all = sorted_tracks.clone();

    rsx! {
        div { class: "w-full max-w-[1600px] mx-auto select-none pb-8",

            div { class: "flex items-end gap-6 mb-8 px-6 pt-6",
                div {
                    class: "w-44 h-44 rounded-2xl overflow-hidden shrink-0 shadow-2xl bg-white/5",
                    style: "box-shadow: 0 20px 60px rgba(0,0,0,0.6);",
                    if let Some(url) = &props.cover_url {
                        img {
                            src: "{url.as_ref()}",
                            class: "w-full h-full object-cover cursor-pointer",
                            onclick: move |_| {
                                if let Some(ref h) = props.on_cover_click { h.call(()); }
                            }
                        }
                    } else {
                        div { class: "w-full h-full flex items-center justify-center",
                            i { class: "fa-solid fa-music text-4xl", style: "color: var(--color-white); opacity: 0.15;" }
                        }
                    }
                }

                div { class: "flex flex-col gap-1 pb-1 min-w-0",
                    if !props.description.is_empty() {
                        p {
                            class: "text-xs font-bold tracking-widest uppercase mb-1",
                            style: "color: var(--color-white); opacity: 0.35;",
                            "{props.description}"
                        }
                    }
                    h1 {
                        class: "text-4xl font-bold text-white truncate mb-1",
                        "{props.name}"
                    }
                    p {
                        class: "text-sm mb-3",
                        style: "color: var(--color-white); opacity: 0.45;",
                        {
                            let count = props.tracks.len();
                            let song_text = i18n::t_with("showcase_song_count", &[("count", count.to_string())]);
                            rsx! { "{song_text} · {duration_min} {i18n::t(\"min\")}" }
                        }
                    }

                    div { class: "flex items-center gap-2 flex-wrap",
                        if !props.tracks.is_empty() {
                            button {
                                class: "inline-flex items-center justify-center gap-2 h-9 px-5 rounded-full text-sm font-semibold text-white transition-opacity hover:opacity-90 active:scale-95",
                                style: "background: var(--color-indigo-500);",
                                onclick: move |_| ctrl.play_queue_linear(tracks_for_play_all.clone()),
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
                                onclick: move |_| {
                                    ctrl.toggle_shuffle();
                                    ctrl.play_queue_shuffled(tracks_for_shuffle.clone());
                                },
                                i { class: "fa-solid fa-shuffle text-xs" }
                                "{i18n::t(\"shuffle\")}"
                            }
                            if props.on_download_all.is_some() || props.on_delete_all.is_some() {
                                button {
                                    class: "inline-flex items-center justify-center h-9 w-9 rounded-full text-sm font-medium transition-colors border border-white/12 hover:bg-white/10",
                                    style: "color: var(--color-white); opacity: 0.6;",
                                    disabled: props.is_downloading_all,
                                    onclick: move |_| {
                                        if props.on_delete_all.is_some() {
                                            if let Some(ref h) = props.on_delete_all { h.call(()); }
                                        } else if let Some(ref h) = props.on_download_all { h.call(()); }
                                    },
                                    if props.is_downloading_all {
                                        i { class: "fa-solid fa-spinner fa-spin text-xs" }
                                    } else {
                                        i { class: "fa-solid fa-download text-xs" }
                                    }
                                }
                            }
                        }
                        if let Some(actions) = props.actions {
                            {actions}
                        }
                    }
                }
            }

            if props.tracks.is_empty() {
                div { class: "flex flex-col items-center justify-center py-16 gap-3",
                    i { class: "fa-regular fa-folder-open text-4xl", style: "color: var(--color-white); opacity: 0.15;" }
                    p { class: "text-sm", style: "color: var(--color-white); opacity: 0.3;", "{i18n::t(\"no_songs_here\")}" }
                }
            } else {
                div {
                    class: "grid px-3 py-2 text-[10px] font-bold text-slate-500 border-white/5 uppercase tracking-widest border-b mb-1",
                    style: "grid-template-columns: 40px 1fr 180px 180px 56px 40px;;",
                    div { class: "flex items-center", "#" }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Title),
                        "{i18n::t(\"title\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Title)} text-[9px]" }
                    }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Artist),
                        "{i18n::t(\"artist\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Artist)} text-[9px]" }
                    }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-widest text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Album),
                        "{i18n::t(\"album\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Album)} text-[9px]" }
                    }
                    button {
                        class: "flex items-center justify-end gap-1 uppercase tracking-widest text-right hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Duration),
                        i { class: "fa-regular fa-clock" }
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Duration)} text-[9px]" }
                    }
                    div {}
                }

                for (display_idx, (track, idx)) in sorted_track_pairs.iter().enumerate() {
                    {
                        let idx = *idx;
                        let matches_current_path = currently_playing_path.as_ref() == Some(&track.path);
                        let matches_current_metadata = !current_song_title.is_empty()
                            && track.title == current_song_title
                            && track.artist == current_song_artist
                            && track.album == current_song_album
                            && track.duration == current_song_duration;
                        let is_playing: bool = matches_current_path || matches_current_metadata;
                        let is_selected = props.is_selection_mode && props.selected_tracks.contains(&track.path);
                        let selection_shadow = if is_selected {
                            "inset 0 0 0 9999px color: var(--color-white); opacity: 0.07;"
                        } else {
                            "none"
                        };
                        let track_dur = fmt_dur(track.duration);
                        let artist = track.artist.clone();
                        let album = track.album.clone();
                        let album_id = track.album_id.clone();
                        let row_num = display_idx + 1;

                        let play_queue = sorted_tracks.clone();
                        let play_queue_button = sorted_tracks.clone();

                        let cover_url: Option<utils::CoverUrl> = {
                            let path_str = track.path.to_string_lossy();
                            if path_str.starts_with("jellyfin:") {
                                let conf = config.read();
                                conf.server.as_ref().and_then(|s| {
                                    utils::jellyfin_image::track_cover_url_with_album_fallback(
                                        &path_str,
                                        &track.album_id,
                                        &s.url,
                                        s.access_token.as_deref(),
                                        64,
                                        90,
                                    ).map(|u| std::sync::Arc::from(u.as_str()))
                                })
                            } else {
                                let lib = props.library.read();
                                lib.albums
                                    .iter()
                                    .find(|a| a.id == track.album_id)
                                    .and_then(|a| utils::format_artwork_url(a.cover_path.as_ref()))
                            }
                        };

                        rsx! {
                            div {
                                key: "{track.path.display()}",
                                class: "grid px-2 py-1.5 rounded-lg mx-1 group cursor-default transition-colors hover:bg-white/5",
                                style: if is_playing {
                                    format!("grid-template-columns: 40px 1fr 180px 180px 56px 40px; background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent); box-shadow: {selection_shadow};")
                                } else {
                                    format!("grid-template-columns: 40px 1fr 180px 180px 56px 40px; box-shadow: {selection_shadow};")
                                },
                                ondoubleclick: move |_| {
                                    ctrl.queue.set(play_queue.clone());
                                    ctrl.play_track(display_idx);
                                },

                                div { class: "flex items-center",
                                    if is_playing {
                                        i {
                                            class: "fa-solid fa-volume-high text-xs",
                                            style: "color: var(--color-indigo-500);"
                                        }
                                    } else {
                                        span {
                                            class: "text-xs group-hover:hidden",
                                            style: "color: var(--color-white); opacity: 0.25;",
                                            "{row_num}"
                                        }
                                        button {
                                            class: "hidden group-hover:flex items-center justify-center",
                                            onclick: move |_| {
                                                ctrl.queue.set(play_queue_button.clone());
                                                ctrl.play_track(display_idx);
                                            },
                                            i { class: "fa-solid fa-play text-xs", style: "color: var(--color-white); opacity: 0.8;" }
                                        }
                                    }
                                }

                                div { class: "flex items-center min-w-0 pr-4 gap-3",
                                    div { class: "w-8 h-8 rounded bg-white/5 overflow-hidden shrink-0 flex items-center justify-center",
                                        if let Some(ref url) = cover_url {
                                            img {
                                                src: "{url.as_ref()}",
                                                class: "w-full h-full object-cover",
                                                loading: "lazy",
                                                decoding: "async",
                                            }
                                        } else {
                                            i { class: "fa-solid fa-music", style: "color: var(--color-white); opacity: 0.2; font-size: 10px;" }
                                        }
                                    }
                                    span {
                                        class: "text-sm font-medium truncate",
                                        style: if is_playing {
                                            "color: var(--color-indigo-500); font-weight: 600;"
                                        } else {
                                            "color: var(--color-white); opacity: 0.9;"
                                        },
                                        ondoubleclick: move |evt| evt.stop_propagation(),
                                        "{track.title}"
                                    }
                                }

                                div { class: "flex items-center min-w-0 pr-4",
                                    span {
                                        class: "text-sm truncate cursor-pointer hover:underline",
                                        style: "color: var(--color-white); opacity: 0.45;",
                                        onclick: {
                                            let artist = artist.clone();
                                            move |evt: MouseEvent| {
                                                evt.stop_propagation();
                                                nav_ctrl.navigate_to_artist(artist.clone());
                                            }
                                        },
                                        ondoubleclick: move |evt| evt.stop_propagation(),
                                        "{artist}"
                                    }
                                }

                                div { class: "flex items-center min-w-0 pr-4",
                                    span {
                                        class: "text-sm truncate cursor-pointer hover:underline",
                                        style: "color: var(--color-white); opacity: 0.35;",
                                        onclick: {
                                            let album_id = album_id.clone();
                                            move |evt: MouseEvent| {
                                                evt.stop_propagation();
                                                nav_ctrl.navigate_to_album(album_id.clone());
                                            }
                                        },
                                        ondoubleclick: move |evt| evt.stop_propagation(),
                                        "{album}"
                                    }
                                }

                                div { class: "flex items-center justify-end",
                                    span {
                                        class: "text-xs font-mono",
                                        style: "color: var(--color-white); opacity: 0.3;",
                                        "{track_dur}"
                                    }
                                }

                                div { class: "flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity",
                                    if let Some(ref _handler) = props.on_click_menu {
                                        button {
                                            class: "w-6 h-6 flex items-center justify-center rounded transition-colors hover:bg-white/10",
                                            style: "color: var(--color-white) opacity: 0.5;",
                                            onclick: move |_| {
                                                if let Some(ref h) = props.on_click_menu { h.call(idx); }
                                            },
                                            i { class: "fa-solid fa-ellipsis text-xs" }
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
