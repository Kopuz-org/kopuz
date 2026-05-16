use crate::reorder_buttons::ReorderButtons;
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
            utils::sleep(std::time::Duration::from_millis(50)).await;
            exact_progress.set(ctrl.displayed_progress_secs_f64());
        }
    });

    let config = use_context::<Signal<AppConfig>>();

    let mut lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>> = use_signal(|| None);
    let mut fetch_gen: Signal<u32> = use_signal(|| 0);
    let mut last_key: Signal<String> = use_signal(String::new);

    use_effect(move || {
        let title = current_song_title.read().clone();
        let track_path = {
            let q = queue.read();
            let idx = *current_queue_index.read();
            q.get(idx)
                .map(|t| t.path.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let new_key = format!("{}|{}", title, track_path);
        if *last_key.peek() == new_key {
            return;
        }
        last_key.set(new_key);

        let artist = current_song_artist.peek().clone();
        let album = current_song_album.peek().clone();
        let duration = *current_song_duration.peek();
        let (server_url, server_token, server_user_id) = {
            let conf = config.peek();
            if let Some(server) = &conf.server {
                (
                    Some(server.url.clone()),
                    server.access_token.clone(),
                    server.user_id.clone(),
                )
            } else {
                (None, None, None)
            }
        };

        let fetch_id = fetch_gen.peek().wrapping_add(1);
        fetch_gen.set(fetch_id);

        if title.is_empty() {
            lyrics.set(Some(None));
            return;
        }

        if let Some(cached) = utils::lyrics::cached_lyrics(
            &artist,
            &title,
            &album,
            duration,
            &track_path,
        ) {
            let display = cached.or_else(|| {
                Some(utils::lyrics::Lyrics::Plain(
                    i18n::t("lyrics_not_found").to_string(),
                ))
            });
            lyrics.set(Some(display));
            return;
        }

        lyrics.set(None);

        spawn(async move {
            let result = utils::lyrics::fetch_lyrics(
                &artist,
                &title,
                &album,
                duration,
                &track_path,
                server_url.as_deref(),
                server_token.as_deref(),
                server_user_id.as_deref(),
            )
            .await;
            if *fetch_gen.peek() == fetch_id {
                let display = result.or_else(|| {
                    Some(utils::lyrics::Lyrics::Plain(
                        i18n::t("lyrics_not_found").to_string(),
                    ))
                });
                lyrics.set(Some(display));
            }
        });
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

    let get_track_cover = |track: &reader::Track| -> Option<utils::CoverUrl> {
        let lib = library.read();
        let conf = config.read();

        let is_server_track = conf.active_source == config::MusicSource::Server;

        if is_server_track {
            if let Some(server) = &conf.server {
                let path_str = track.path.to_string_lossy();
                let url = match server.service {
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
                return utils::map_cover_url(url);
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
    let mut move_queue_item = move |from: usize, to: usize| {
        ctrl.move_queue_item(from, to);
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

    let back_text = i18n::t("back").to_string().to_uppercase();
    let up_next_text = i18n::t("up_next").to_string();
    let lyrics_text = i18n::t("lyrics").to_string();
    let format_queue_duration = |seconds: u64| {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;
        if hours > 0 {
            format!("{hours}:{minutes:02}:{secs:02}")
        } else {
            format!("{minutes}:{secs:02}")
        }
    };
    let q = queue.read();
    let current_idx = *current_queue_index.read();
    let is_shuffle = *ctrl.shuffle.read();

    let (back_items, up_next_items): (Vec<_>, Vec<_>) = if is_shuffle {
        let order = ctrl.shuffle_order.read();
        let back = order
            .get(..current_idx)
            .unwrap_or_default()
            .iter()
            .filter_map(|&qi| q.get(qi).cloned().map(|t| (qi, t)))
            .collect();
        let next = order
            .get(current_idx + 1..)
            .unwrap_or_default()
            .iter()
            .filter_map(|&qi| q.get(qi).cloned().map(|t| (qi, t)))
            .collect();
        (back, next)
    } else {
        let back = (0..current_idx)
            .filter_map(|qi| q.get(qi).cloned().map(|t| (qi, t)))
            .collect();
        let next = (current_idx + 1..q.len())
            .filter_map(|qi| q.get(qi).cloned().map(|t| (qi, t)))
            .collect();
        (back, next)
    };

    let up_next_count = up_next_items.len();
    let up_next_duration: u64 = up_next_items.iter().map(|(_, t)| t.duration).sum();
    let up_next_summary = format!(
        "{} • {}",
        i18n::t_with(
            "showcase_song_count",
            &[("count", up_next_count.to_string())]
        ),
        format_queue_duration(up_next_duration)
    );

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
                        "{back_text}"
                    }
                    button {
                        class: if *active_tab.read() == 1 {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(1),
                        "{up_next_text}"
                    }
                    button {
                        class: if *active_tab.read() == 2 {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(2),
                        "{lyrics_text}"
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
                            None => rsx! { "{i18n::t(\"loading_lyrics\")}" },
                        }
                    }
                } else if *active_tab.read() == 0 {
                    if back_items.is_empty() {
                        div { class: "text-white/30 text-center py-10 text-sm", "{i18n::t(\"no_previous_songs\")}" }
                    } else {
                    for (list_pos, (queue_idx, track)) in back_items.iter().enumerate() {
                        {
                            let queue_idx = *queue_idx;
                            let track_idx = list_pos;
                            let cover_url = get_track_cover(&track);
                            rsx! {
                                div {
                                    key: "{queue_idx}",
                                    class: "flex items-center gap-3 px-2 py-2 hover:bg-white/5 cursor-pointer rounded-lg transition-colors group",
                                    style: "content-visibility: auto; contain-intrinsic-size: 0 56px;",
                                    ondoubleclick: move |_| play_song_at_index(track_idx),
                                    div {
                                        class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                                        style: "width: 40px; height: 40px;",
                                        if let Some(ref url) = cover_url {
                                            img { src: "{url.as_ref()}", class: "w-full h-full object-cover" }
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

                } else if *active_tab.read() == 1 {
                    if up_next_items.is_empty() {
                        div { class: "text-white/30 text-center py-10 text-sm", "{i18n::t(\"no_more_songs\")}" }
                    } else {
                        div {
                            class: "px-2 pt-1 pb-2 text-[11px] uppercase tracking-[0.18em] text-slate-500",
                            "{up_next_summary}"
                        }
                        for (list_pos, (queue_idx, track)) in up_next_items.iter().enumerate() {
                            {
                                let queue_idx = *queue_idx;
                                let cover_url = get_track_cover(&track);
                                let track_idx = current_idx + 1 + list_pos;
                                let can_move_up = track_idx > current_idx + 1;
                                let can_move_down = track_idx + 1 < q.len();
                                rsx! {
                                    div {
                                        key: "{queue_idx}",
                                        class: "flex items-center gap-3 px-2 py-2 hover:bg-white/5 cursor-pointer rounded-lg transition-colors group",
                                        style: "content-visibility: auto; contain-intrinsic-size: 0 56px;",
                                        ondoubleclick: move |_| play_song_at_index(track_idx),
                                        div {
                                            class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                                            style: "width: 40px; height: 40px;",
                                            if let Some(ref url) = cover_url {
                                                img { src: "{url.as_ref()}", class: "w-full h-full object-cover" }
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
                                        ReorderButtons {
                                            can_move_up,
                                            can_move_down,
                                            class: "flex flex-col pr-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity".to_string(),
                                            on_move_up: move |_| move_queue_item(track_idx, track_idx - 1),
                                            on_move_down: move |_| move_queue_item(track_idx, track_idx + 1),
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
