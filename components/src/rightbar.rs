use crate::reorder_buttons::ReorderButtons;
use config::AppConfig;
use dioxus::core::use_hook_with_cleanup;
use dioxus::document::eval;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use reader::Library;
use serde_json::Value;

fn js_scroll_to_top() {
    let _scroll_to_top = eval(
        r#"
            const el = document.getElementById('rightbar-content');
            if (el) { el.scrollTo({top: 0, left: 0, behavior: 'auto'}); }
        "#,
    );
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tabs {
    Back,
    Next,
    Lyrics,
}

#[component]
pub fn LyricsPanel(
    lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>>,
    current_song_progress: Signal<u64>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    use_hook_with_cleanup(
        move || {
            let _update_func = eval(
                r#"
                    let currEl;

                    window.rightbar_updateLyrics = (nextIndex) => {
                        let nextEl = document.getElementById(`rightbar-lyrics-${nextIndex}`)
                        if (currEl != nextEl) {
                            if (currEl) {
                                currEl.className = 'text-white/40 transition-all duration-300 hover:text-white/60 cursor-pointer';
                            }
                            if (nextEl) {
                                nextEl.className = 'text-white text-lg font-bold transition-all duration-300';
                                nextEl.scrollIntoView({ behavior: 'smooth', block: 'center' });
                            }
                            currEl = nextEl;
                        }
                    }"#,
            );
        },
        move |_| {
            let _cleanup = eval("window.rightbar_updateLyrics = undefined;");
        },
    );

    use_resource(move || {
        let lyrics = lyrics.read().clone();

        async move {
            if let Some(Some(utils::lyrics::Lyrics::Synced(lines))) = lyrics {
                let mut sleep_duration_ms: u64;

                loop {
                    let current_time = ctrl.displayed_progress_secs_f64();
                    if let Some(next_index) =
                        lines.iter().rposition(|l| l.start_time <= current_time)
                    {
                        let _ = eval(&format!("window.rightbar_updateLyrics({next_index})"));

                        sleep_duration_ms = lines
                            .get(next_index.saturating_add(1))
                            .map(|line| {
                                ((line.start_time - current_time) * 1000.0)
                                    .max(16.0)
                                    .min(50.0) as u64
                            })
                            .unwrap_or(50);
                    } else {
                        let _ = eval("window.rightbar_updateLyrics(-1)");
                        sleep_duration_ms = 50;
                    }

                    utils::sleep(std::time::Duration::from_millis(sleep_duration_ms)).await;
                }
            }
        }
    });

    rsx! {
        div {
            class: "text-white/70 text-center py-4 px-4 leading-relaxed font-medium text-sm flex flex-col gap-4",
            match &*lyrics.read() {
                Some(Some(utils::lyrics::Lyrics::Synced(lines))) => {
                    rsx! {
                        for (i, line) in lines.iter().enumerate() {
                            div {
                                key: "{i}",
                                id: "rightbar-lyrics-{i}",
                                class:  "text-white/40 transition-all duration-300 hover:text-white/60 cursor-pointer",
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
    }
}

#[component]
pub fn QueuePanel(
    items: Memo<Vec<(usize, reader::Track)>>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    current_queue_index: Signal<usize>,
    active_tab: Tabs,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();

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
        js_scroll_to_top();
    };

    let mut move_queue_item = move |from: usize, to: usize| {
        ctrl.move_queue_item(from, to);
    };

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

    let items = items.read();
    let current_idx = *current_queue_index.read();
    let (back_items, up_next_items) = (
        items.get(..current_idx).unwrap_or_default(),
        items.get(current_idx + 1..).unwrap_or_default(),
    );

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
        if active_tab == Tabs::Back {
            if back_items.is_empty() {
                div { class: "text-white/30 text-center py-10 text-sm", "{i18n::t(\"no_previous_songs\")}" }
            } else {
            for (list_pos, (queue_idx, track)) in back_items.iter().enumerate().rev() {
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

        } else if active_tab == Tabs::Next {
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
                    let can_move_down = track_idx + 1 < items.len();
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

    let mut active_tab = use_signal(|| Tabs::Next);
    let ctrl = use_context::<PlayerController>();

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

        if let Some(cached) =
            utils::lyrics::cached_lyrics(&artist, &title, &album, duration, &track_path)
        {
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

    // reset scroll position on tab change
    use_effect(move || {
        let _tab = active_tab.read();
        js_scroll_to_top();
    });

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

    let active_tab_val = *active_tab.read();

    let items = use_memo(move || {
        let q = queue.read();
        let is_shuffle = *ctrl.shuffle.read();

        if is_shuffle {
            ctrl.shuffle_order
                .read()
                .iter()
                .filter_map(|&qi| q.get(qi).cloned().map(|t| (qi, t)))
                .collect::<Vec<_>>()
        } else {
            (0..q.len())
                .filter_map(|qi| q.get(qi).cloned().map(|t| (qi, t)))
                .collect::<Vec<_>>()
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
                        class: if active_tab_val == Tabs::Back {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(Tabs::Back),
                        "{back_text}"
                    }
                    button {
                        class: if active_tab_val == Tabs::Next {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(Tabs::Next),
                        "{up_next_text}"
                    }
                    button {
                        class: if active_tab_val == Tabs::Lyrics {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-2 py-1 text-[10px] font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(Tabs::Lyrics),
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
                id: "rightbar-content",
                class: "flex-1 overflow-y-auto px-2 py-2 space-y-1 relative",

                if active_tab_val == Tabs::Lyrics {
                    LyricsPanel{
                        lyrics: lyrics,
                        current_song_progress: current_song_progress
                    }
                } else {
                    QueuePanel{
                        items: items,
                        library: library,
                        config: config,
                        current_queue_index: current_queue_index,
                        active_tab: active_tab_val
                   }
                }
            }
        }
    }
}
