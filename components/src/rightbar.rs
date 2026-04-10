use config::AppConfig;
use dioxus::document::eval;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use reader::Library;
use serde_json::Value;

#[component]
pub fn Rightbar(
    library: Signal<Library>,
    mut is_rightbar_open: Signal<bool>,
    mut width: Signal<usize>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    queue: Signal<Vec<reader::Track>>,
    mut current_queue_index: Signal<usize>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_album: Signal<String>,
) -> Element {
    if !*is_rightbar_open.read() {
        return rsx! { div {} };
    }

    let mut active_tab = use_signal(|| 1usize);
    let mut ctrl = use_context::<PlayerController>();
    let mut exact_progress = use_signal(|| 0.0_f64);

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            exact_progress.set(ctrl.player.peek().get_position().as_secs_f64());
        }
    });

    let config = use_context::<Signal<AppConfig>>();

    let lyrics = use_resource(move || {
        let title = current_song_title.read().clone();
        let artist = current_song_artist.read().clone();
        let album = current_song_album.read().clone();
        let duration = *current_song_duration.read();

        async move {
            if !title.is_empty() {
                if let Some(l) =
                    utils::lyrics::fetch_lyrics(&artist, &title, &album, duration).await
                {
                    Some(l)
                } else {
                    Some(utils::lyrics::Lyrics::Plain("Lyrics not found".to_string()))
                }
            } else {
                None
            }
        }
    });

    let active_lyric_index = use_memo(move || {
        if *active_tab.read() == 2 {
            if let Some(Some(utils::lyrics::Lyrics::Synced(lines))) = &*lyrics.read() {
                let current_time = *exact_progress.read();
                return lines
                    .iter()
                    .rposition(|l| l.start_time <= current_time)
                    .unwrap_or(0);
            }
        }
        0
    });

    use_effect(move || {
        let _idx = active_lyric_index();
        if *active_tab.read() == 2 {
            let _ = eval(
                r#"
                setTimeout(() => {
                    let el = document.getElementById('rightbar-active-lyric');
                    if (el) {
                        el.scrollIntoView({ behavior: 'smooth', block: 'center' });
                    }
                }, 50);
                "#,
            );
        }
    });

    let get_track_cover = |track: &reader::Track| -> Option<String> {
        let lib = library.read();
        let conf = config.read();

        let is_server_track = conf.active_source == config::MusicSource::Server;

        if is_server_track {
            if let Some(server) = &conf.server {
                let path_str = track.path.to_string_lossy();
                return match server.service {
                    config::MusicService::Jellyfin => {
                        utils::jellyfin_image::jellyfin_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            80,
                            80,
                        )
                    }
                    config::MusicService::Subsonic | config::MusicService::Custom => {
                        utils::subsonic_image::subsonic_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            80,
                            80,
                        )
                    }
                };
            }
            None
        } else {
            lib.albums
                .iter()
                .find(|a| a.id == track.album_id)
                .and_then(|album| utils::format_artwork_url(album.cover_path.as_ref()))
        }
    };

    let mut play_song_at_index = move |index: usize| {
        ctrl.play_track_no_history(index);
    };

    let mut is_resizing = use_signal(|| false);

    use_effect(move || {
        if *is_resizing.read() {
            spawn(async move {
                let mut eval = eval(
                    r#"
                    const handleMouseMove = (e) => {
                        dioxus.send(window.innerWidth - e.clientX);
                    };
                    const handleMouseUp = () => {
                        dioxus.send("stop");
                        window.removeEventListener('mousemove', handleMouseMove);
                        window.removeEventListener('mouseup', handleMouseUp);
                    };
                    window.addEventListener('mousemove', handleMouseMove);
                    window.addEventListener('mouseup', handleMouseUp);
                    "#,
                );

                while let Ok(val) = eval.recv::<Value>().await {
                    if let Some(w) = val.as_f64() {
                        let new_width = w.max(280.0).min(600.0);
                        width.set(new_width as usize);
                    } else if val.as_str() == Some("stop") {
                        is_resizing.set(false);
                        break;
                    }
                }
            });
        }
    });

    rsx! {
        div {
            class: "bg-black/40 border-l border-white/5 flex flex-col h-full flex-shrink-0 z-10 relative",
            style: "width: {width}px; min-width: {width}px;",

            div {
                class: "absolute -left-1 top-0 w-3 h-full cursor-col-resize hover:bg-white/20 transition-colors z-50 group/handle",
                onmousedown: move |evt| {
                    evt.stop_propagation();
                    is_resizing.set(true);
                },
                div { class: "w-[1px] h-full bg-white/0 group-hover/handle:bg-white/10 mx-auto" }
            }

            div {
                class: "flex items-center justify-between px-4 py-4 border-b border-white/10",
                div {
                    class: "flex items-center gap-1",
                    button {
                        class: if *active_tab.read() == 0 {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(0),
                        "BACK"
                    }
                    button {
                        class: if *active_tab.read() == 1 {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(1),
                        "UP NEXT"
                    }
                    button {
                        class: if *active_tab.read() == 2 {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(2),
                        "LYRICS"
                    }
                }
                button {
                    class: "text-white/40 hover:text-white",
                    onclick: move |_| is_rightbar_open.set(false),
                    i { class: "fa-solid fa-xmark text-sm" }
                }
            }

            div {
                class: "flex-1 overflow-y-auto px-2 py-2 space-y-1 relative",

                if *active_tab.read() == 2 {
                    div {
                        class: "text-white/70 text-center py-4 px-4 leading-relaxed font-medium text-sm flex flex-col gap-4",
                        match &*lyrics.read() {
                            Some(Some(utils::lyrics::Lyrics::Synced(lines))) => {
                                let active_idx = active_lyric_index();
                                rsx! {
                                    for (i, line) in lines.iter().enumerate() {
                                        div {
                                            key: "{i}",
                                            id: if i == active_idx { "rightbar-active-lyric" } else { "" },
                                            class: if i == active_idx {
                                                "text-white text-lg font-bold transition-all duration-300"
                                            } else {
                                                "text-white/40 transition-all duration-300 hover:text-white/60 cursor-pointer"
                                            },
                                            onclick: {
                                                let st = line.start_time;
                                                move |_| {
                                                    ctrl.player.write().seek(std::time::Duration::from_secs_f64(st));
                                                    current_song_progress.set(st as u64);
                                                }
                                            },
                                            "{line.text}"
                                        }
                                    }
                                }
                            }
                            Some(Some(utils::lyrics::Lyrics::Plain(text))) => rsx! {
                                div { class: "whitespace-pre-wrap", "{text}" }
                            },
                            Some(None) => rsx! { "" },
                            None => rsx! { "Loading lyrics..." },
                        }
                    }
                } else if *active_tab.read() == 0 {
                    if *current_queue_index.read() == 0 {
                        div { class: "text-white/30 text-center py-10 text-sm", "No previous songs" }
                    }
                    for i in 0..*current_queue_index.read() {
                        {
                            let track = queue.read()[i].clone();
                            let cover_url = get_track_cover(&track);
                            rsx! {
                                div {
                                    key: "{i}",
                                    class: "flex items-center gap-3 px-2 py-2 hover:bg-white/5 cursor-pointer rounded-lg transition-colors group",
                                    ondoubleclick: move |_| play_song_at_index(i),
                                    div {
                                        class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                                        style: "width: 40px; height: 40px;",
                                        if let Some(ref url) = cover_url {
                                            img { src: "{url}", class: "w-full h-full object-cover" }
                                        } else {
                                            div {
                                                class: "w-full h-full flex items-center justify-center",
                                                i { class: "fa-solid fa-music text-white/20", style: "font-size: 12px;" }
                                            }
                                        }
                                    }
                                    div {
                                        class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                                        div { class: "text-sm text-white truncate font-medium", "{track.title}" }
                                        div { class: "text-xs text-white/50 truncate group-hover:text-white/70", "{track.artist}" }
                                    }
                                }
                            }
                        }
                    }
                } else if *active_tab.read() == 1 {
                    if queue.read().len() <= *current_queue_index.read() + 1 {
                        div { class: "text-white/30 text-center py-10 text-sm", "No more songs in queue" }
                    }
                    for i in (*current_queue_index.read() + 1)..queue.read().len() {
                        {
                            let track = queue.read()[i].clone();
                            let cover_url = get_track_cover(&track);
                            rsx! {
                                div {
                                    key: "{i}",
                                    class: "flex items-center gap-3 px-2 py-2 hover:bg-white/5 cursor-pointer rounded-lg transition-colors group",
                                    ondoubleclick: move |_| play_song_at_index(i),
                                    div {
                                        class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                                        style: "width: 40px; height: 40px;",
                                        if let Some(ref url) = cover_url {
                                            img { src: "{url}", class: "w-full h-full object-cover" }
                                        } else {
                                            div {
                                                class: "w-full h-full flex items-center justify-center",
                                                i { class: "fa-solid fa-music text-white/20", style: "font-size: 12px;" }
                                            }
                                        }
                                    }
                                    div {
                                        class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                                        div { class: "text-sm text-white truncate font-medium", "{track.title}" }
                                        div { class: "text-xs text-white/50 truncate group-hover:text-white/70", "{track.artist}" }
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
