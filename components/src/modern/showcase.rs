use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

use crate::showcase::ShowcaseProps;

#[component]
pub fn ShowcaseModern(props: ShowcaseProps) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();

    let total_seconds: u64 = props.tracks.iter().map(|t| t.duration).sum();
    let duration_min = total_seconds / 60;

    let currently_playing_idx: Option<usize> = {
        let queue = ctrl.queue.read();
        let idx = *ctrl.current_queue_index.read();
        if queue.len() == props.tracks.len()
            && queue
                .iter()
                .zip(props.tracks.iter())
                .all(|(q, t)| q.path == t.path)
        {
            Some(idx)
        } else {
            None
        }
    };

    let tracks_for_shuffle = props.tracks.clone();
    let fmt_dur = |s: u64| format!("{}:{:02}", s / 60, s % 60);

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
                            i { class: "fa-solid fa-music text-4xl", style: "color: rgba(255,255,255,0.15);" }
                        }
                    }
                }

                div { class: "flex flex-col gap-1 pb-1 min-w-0",
                    if !props.description.is_empty() {
                        p {
                            class: "text-xs font-bold tracking-widest uppercase mb-1",
                            style: "color: rgba(255,255,255,0.35);",
                            "{props.description}"
                        }
                    }
                    h1 {
                        class: "text-4xl font-bold text-white truncate mb-1",
                        "{props.name}"
                    }
                    p {
                        class: "text-sm mb-3",
                        style: "color: rgba(255,255,255,0.45);",
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
                                onclick: move |_| props.on_play_all.call(()),
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
                                    class: "inline-flex items-center justify-center h-9 w-9 rounded-full text-sm font-medium transition-colors hover:bg-white/10",
                                    style: "color: rgba(255,255,255,0.6); border: 1px solid rgba(255,255,255,0.12);",
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
                    i { class: "fa-regular fa-folder-open text-4xl", style: "color: rgba(255,255,255,0.15);" }
                    p { class: "text-sm", style: "color: rgba(255,255,255,0.3);", "{i18n::t(\"no_songs_here\")}" }
                }
            } else {
                div {
                    class: "grid px-6 py-2 text-[10px] font-bold uppercase tracking-widest border-b",
                    style: "grid-template-columns: 40px 1fr 200px 200px 56px 40px; color: rgba(255,255,255,0.25); border-color: rgba(255,255,255,0.06);",
                    div { class: "flex items-center", "#" }
                    div { "{i18n::t(\"title\")}" }
                    div { "{i18n::t(\"artist\")}" }
                    div { "{i18n::t(\"album\")}" }
                    div { class: "text-right", i { class: "fa-regular fa-clock" } }
                    div {}
                }

                for (idx, track) in props.tracks.iter().enumerate() {
                    {
                        let is_playing = currently_playing_idx == Some(idx);
                        let is_selected = props.selected_tracks.contains(&track.path);
                        let track_dur = fmt_dur(track.duration);
                        let title = track.title.clone();
                        let artist = track.artist.clone();
                        let album = track.album.clone();
                        let row_num = idx + 1;

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
                                class: "grid px-6 py-1.5 rounded-lg mx-2 group cursor-default transition-colors hover:bg-white/5",
                                style: if is_playing {
                                    format!("grid-template-columns: 40px 1fr 200px 200px 56px 40px; background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent);")
                                } else if is_selected {
                                    "grid-template-columns: 40px 1fr 200px 200px 56px 40px; background: rgba(255,255,255,0.07);".to_string()
                                } else {
                                    "grid-template-columns: 40px 1fr 200px 200px 56px 40px;".to_string()
                                },
                                ondoubleclick: move |_| props.on_play.call(idx),

                                div { class: "flex items-center",
                                    if is_playing {
                                        i {
                                            class: "fa-solid fa-volume-high text-xs",
                                            style: "color: var(--color-indigo-500);"
                                        }
                                    } else {
                                        span {
                                            class: "text-xs group-hover:hidden",
                                            style: "color: rgba(255,255,255,0.25);",
                                            "{row_num}"
                                        }
                                        button {
                                            class: "hidden group-hover:flex items-center justify-center",
                                            onclick: move |_| props.on_play.call(idx),
                                            i { class: "fa-solid fa-play text-xs", style: "color: rgba(255,255,255,0.8);" }
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
                                            i { class: "fa-solid fa-music", style: "color: rgba(255,255,255,0.2); font-size: 10px;" }
                                        }
                                    }
                                    span {
                                        class: "text-sm font-medium truncate",
                                        style: if is_playing {
                                            "color: var(--color-indigo-500); font-weight: 600;"
                                        } else {
                                            "color: rgba(255,255,255,0.9);"
                                        },
                                        "{title}"
                                    }
                                }

                                div { class: "flex items-center min-w-0 pr-4",
                                    span {
                                        class: "text-sm truncate",
                                        style: "color: rgba(255,255,255,0.45);",
                                        "{artist}"
                                    }
                                }

                                div { class: "flex items-center min-w-0 pr-4",
                                    span {
                                        class: "text-sm truncate",
                                        style: "color: rgba(255,255,255,0.35);",
                                        "{album}"
                                    }
                                }

                                div { class: "flex items-center justify-end",
                                    span {
                                        class: "text-xs font-mono",
                                        style: "color: rgba(255,255,255,0.3);",
                                        "{track_dur}"
                                    }
                                }

                                div { class: "flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity",
                                    if let Some(ref _handler) = props.on_click_menu {
                                        button {
                                            class: "w-6 h-6 flex items-center justify-center rounded transition-colors hover:bg-white/10",
                                            style: "color: rgba(255,255,255,0.5);",
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
