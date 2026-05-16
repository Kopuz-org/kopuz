use crate::reorder_buttons::ReorderButtons;
use crate::titlebar::Titlebar;
use config::AppConfig;
use dioxus::document::eval;
use dioxus::prelude::*;
use hooks::use_player_controller::{LoopMode, PlayerController};
use player::player::Player;
use reader::Library;
use crate::shared::fmt_time;

#[component]
pub fn Fullscreen(
    library: Signal<Library>,
    mut player: Signal<Player>,
    mut is_playing: Signal<bool>,
    mut is_fullscreen: Signal<bool>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    queue: Signal<Vec<reader::Track>>,
    mut current_queue_index: Signal<usize>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_khz: Signal<u32>,
    mut current_song_bitrate: Signal<u16>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_album: Signal<String>,
    mut volume: Signal<f32>,
    mut persisted_volume: Signal<f32>,
    palette: Signal<Option<Vec<utils::color::Color>>>,
) -> Element {
    let mut is_dragging = use_signal(|| false);
    let mut drag_progress = use_signal(|| 0u64);

    let display_progress = if *is_dragging.read() {
        *drag_progress.read()
    } else {
        *current_song_progress.read()
    };
    if !*is_fullscreen.read() {
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

    let progress_percent = if *current_song_duration.read() > 0 {
        (display_progress as f64 / *current_song_duration.read() as f64) * 100.0
    } else {
        0.0
    };

    let volume_percent = *volume.read() * 100.0;

    let mut play_song_at_index = move |index: usize| {
        ctrl.play_track_no_history(index);
    };
    let mut move_queue_item = move |from: usize, to: usize| {
        ctrl.move_queue_item(from, to);
    };

    let mut config = use_context::<Signal<AppConfig>>();

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
                    let el = document.getElementById('active-lyric');
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

        let path_str = track.path.to_string_lossy();
        let provider = if path_str.starts_with("jellyfin:") {
            Some(config::MusicService::Jellyfin)
        } else if path_str.starts_with("subsonic:") {
            Some(config::MusicService::Subsonic)
        } else if path_str.starts_with("custom:") {
            Some(config::MusicService::Custom)
        } else {
            None
        };

        if let Some(provider) = provider {
            if let Some(server) = &conf.server {
                let url = match provider {
                    config::MusicService::Jellyfin => {
                        utils::jellyfin_image::jellyfin_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            96,
                            80,
                        )
                    }
                    config::MusicService::Subsonic | config::MusicService::Custom => {
                        utils::subsonic_image::subsonic_image_url_from_path(
                            &path_str,
                            &server.url,
                            server.access_token.as_deref(),
                            96,
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

    let is_shuffle = *ctrl.shuffle.read();
    let up_next_items: Vec<(usize, reader::Track)> = {
        let q = queue.read();
        let current_idx = *current_queue_index.read();
        if is_shuffle {
            ctrl.shuffle_order
                .read()
                .iter()
                .filter_map(|&qi| q.get(qi).map(|t| (qi, t.clone())))
                .collect()
        } else {
            (current_idx + 1..q.len())
                .filter_map(|qi| q.get(qi).map(|t| (qi, t.clone())))
                .collect()
        }
    };

    let background_style = if config.read().theme == "album-art" {
        utils::color::get_background_style(palette.read().as_deref())
    } else {
        "background-color: var(--color-black); background-image: none;".to_string()
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

    let is_radio = *current_song_duration.read() == u64::MAX;

    rsx! {
            div {
                class: "fixed inset-0 z-50 flex flex-col text-white select-none",
                style: "{background_style}",

                if cfg!(any(target_os = "linux", target_os = "windows")) {
                    div { dir: "ltr", Titlebar {} }
                }

                div {
                    class: "flex flex-1 overflow-hidden",

                div {
                    class: "flex flex-col items-center justify-center p-8 lg:p-12 relative flex-shrink-0",
                    style: "width: 50%; max-width: 600px;",

                    div {
                        class: "rounded-2xl overflow-hidden mb-8 shadow-2xl",
                        style: "width: 100%; max-width: 420px; aspect-ratio: 1/1;",
                        {
                            let cover = current_song_cover_url.read();
                            if cover.is_empty() {
                                rsx! {
                                    div {
                                        class: "w-full h-full flex items-center justify-center bg-black/30",
                                        i { class: "fa-solid fa-music text-5xl text-white/20" }
                                    }
                                }
                            } else {
                                let src = if cover.starts_with("artwork://") {
                                    format!("{}&hq=1", cover)
                                } else {
                                    cover.clone()
                                };
                                rsx! {
                                    img {
                                        src: "{src}",
                                        class: "w-full h-full object-cover"
                                    }
                                }
                            }
                        }
                    }

                    div {
                        class: "flex flex-col items-start w-full mb-2",
                        style: "max-width: 420px;",
                        h1 { class: "text-3xl font-bold text-white mb-2 line-clamp-1", "{current_song_title}" }
                        div {
                            class: "flex items-center gap-2",
                            h2 { class: "text-xl text-white/70 font-medium line-clamp-1", "{current_song_artist}" }
                            span { class: "text-white/30", "•" }
                            h3 { class: "text-lg text-white/50 line-clamp-1", "{current_song_album}" }
                        }
                    }

                    div {
                        class: "flex items-center gap-4 text-xs text-white/50 mb-6 w-full",
                        style: "max-width: 420px;",
                        span { style: "font-size: 10px;", "{current_song_khz() / 1000} kHz / {current_song_bitrate} kbps" }
                    }

                    div {
                        class: "w-full mb-6",
                        style: "max-width: 420px;",
                        div {
                            class: "flex items-center gap-3",
                            span { class: "text-xs text-white/70 font-mono", style: "width: 50px; text-align: left;", "{fmt_time(display_progress)}" }
                            div {
                                class: format!("flex-1 {} relative", if is_radio { "" } else { "cursor-pointer" }),
                                style: "height: 20px;",
                                div {
                                    class: "absolute bg-white/20 rounded-full",
                                    style: "height: 4px; top: 8px; left: 0; right: 0;"
                                }
                                div {
                                    class: "absolute rounded-full pointer-events-none",
                                    style: "height: 4px; top: 8px; left: 0; width: {progress_percent}%; background: linear-gradient(to right, #5a9a9a, #ffffff);"
                                }
                                div {
                                    class: "absolute bg-white rounded-full pointer-events-none",
                                    style: "width: 12px; height: 12px; top: 4px; left: calc({progress_percent}% - 6px);"
                                }
                                input {
                                    r#type: "range",
                                    min: "0",
                                    max: "{*current_song_duration.read()}",
                                    value: "{display_progress}",
                                    class: format!("absolute top-0 left-0 w-full h-full opacity-0 {}", if is_radio { "" } else { "cursor-pointer" }),
                                    disabled: is_radio,
                                    onchange: move |evt| {
                                        if let Ok(val) = evt.value().parse::<f64>().map(|v| v as u64) {
                                            player.write().seek(std::time::Duration::from_secs(val));
                                            current_song_progress.set(val);
                                            drag_progress.set(val);
                                            is_dragging.set(false);
                                        }
                                    },
                                    oninput: move |evt| {
                                        if let Ok(val) = evt.value().parse::<f64>().map(|v| v as u64) {
                                            is_dragging.set(true);
                                            drag_progress.set(val);
                                        }
                                    }
                                }
                            }
                            span { class: "text-xs text-white/70 font-mono", style: "width: 50px; text-align: right;", "{fmt_time(*current_song_duration.read())}" }
                        }
                    }

                    div {
                        class: "flex items-center justify-between w-full mb-8",
                        style: "max-width: 420px;",
                        button {
                            class: format!("{} transition-all active:scale-95 relative flex-shrink-0", if *ctrl.shuffle.read() { "text-white" } else { "text-white/50 hover:text-white" }),
                            onclick: move |_| ctrl.toggle_shuffle(),
                            title: if *ctrl.shuffle.read() { i18n::t("shuffle_on").to_string() } else { i18n::t("shuffle_off").to_string() },
                            i { class: "fa-solid fa-shuffle text-lg" }
                        }
                        div {
                            class: "flex items-center gap-8",
                            button {
                                class: "text-white hover:text-white/80 transition-colors flex-shrink-0",
                                onclick: move |_| {
                                    ctrl.play_prev();
                                },
                                i { class: "fa-solid fa-backward-step text-3xl" }
                            }
                            button {
                                class: "w-20 h-20 bg-white text-black hover:bg-white/90 rounded-full flex items-center justify-center transition-all flex-shrink-0 shadow-lg hover:scale-105 active:scale-95",
                                onclick: move |_| {
                                    ctrl.toggle();
                                },
                                i { class: if *is_playing.read() { "fa-solid fa-pause text-3xl" } else { "fa-solid fa-play text-3xl ml-1" } }
                            }
                            button {
                                class: "text-white hover:text-white/80 transition-colors flex-shrink-0",
                                onclick: move |_| {
                                    ctrl.play_next();
                                },
                                i { class: "fa-solid fa-forward-step text-3xl" }
                            }
                        }
                        button {
                            class: format!("{} transition-all active:scale-95 relative flex-shrink-0",
                                match *ctrl.loop_mode.read() {
                                    LoopMode::None => "text-white/50 hover:text-white",
                                    LoopMode::Queue => "text-white",
                                    LoopMode::Track => "text-white",
                                }
                            ),
                            onclick: move |_| ctrl.toggle_loop(),
                            title: match *ctrl.loop_mode.read() {
                                LoopMode::None => i18n::t("repeat_off").to_string(),
                                LoopMode::Queue => i18n::t("repeat_queue").to_string(),
                                LoopMode::Track => i18n::t("repeat_track").to_string(),
                            },
                            i { class: "fa-solid fa-repeat text-lg" }
                            match *ctrl.loop_mode.read() {
                                 LoopMode::Track => rsx! {
                                     span { class: "absolute -bottom-2.5 left-1/2 -translate-x-1/2 text-[10px] font-bold text-white leading-none", "1" }
                                 },
                                 _ => rsx! {
                                     div {}
                                 }
                            }
                        }
                    }

                    div {
                        class: "flex items-center gap-5 w-full",
                        style: "max-width: 420px;",
                        i { class: "fa-solid fa-volume-low text-white/40" }
                        div {
                            class: "flex-1 cursor-pointer relative",
                            style: "height: 20px;",
                            div {
                                class: "absolute bg-white rounded-full",
                                style: "height: 4px; top: 8px; left: 6px; right: 0;"
                            }
                            div {
                                class: "absolute bg-white/70 rounded-full pointer-events-none",
                                style: "height: 4px; top: 8px; left: 0; width: {volume_percent}%;"
                            }
                            div {
                                class: "absolute bg-white rounded-full pointer-events-none",
                                style: "width: 12px; height: 12px; top: 4px; left: calc({volume_percent}% - 6px);"
                            }
                            input {
                                r#type: "range",
                                min: "0",
                                max: "1",
                                step: "0.01",
                                value: "{*volume.read()}",
                                class: "absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer",
                                onchange: move |evt| {
                                    if let Ok(val) = evt.value().parse::<f32>() {
                                        persisted_volume.set(val);
                                    }
                                },
                                oninput: move |evt| {
                                    if let Ok(val) = evt.value().parse::<f32>() {
                                        player.write().set_volume(val);
                                        volume.set(val);
                                    }
                                }
                            }
                        }
                    }

                    button {
                        class: "absolute top-8 left-8 text-white/30 hover:text-white transition-colors",
                        onclick: move |_| is_fullscreen.set(false),
                        i { class: "fa-solid fa-chevron-down text-2xl" }
                    }
                }

                div {
                    class: "flex-1 flex flex-col h-full min-w-0",

                    div {
                        class: "flex items-center gap-1 px-6 pt-4 pb-2 border-b border-white/10",
                        button {
                            class: if *active_tab.read() == 0 {
                                "px-4 py-2 text-xs font-medium tracking-wider text-white border-b-2 border-white"
                            } else {
                                "px-4 py-2 text-xs font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                            },
                            onclick: move |_| active_tab.set(0),
                            "{i18n::t(\"back_to_previous\")}"
                        }
                        button {
                            class: if *active_tab.read() == 1 {
                                "px-4 py-2 text-xs font-medium tracking-wider text-white border-b-2 border-white"
                            } else {
                                "px-4 py-2 text-xs font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                            },
                            onclick: move |_| active_tab.set(1),
                            "{i18n::t(\"up_next\")}"
                        }
                        button {
                            class: if *active_tab.read() == 2 {
                                "px-4 py-2 text-xs font-medium tracking-wider text-white border-b-2 border-white"
                            } else {
                                "px-4 py-2 text-xs font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                            },
                            onclick: move |_| active_tab.set(2),
                            "{i18n::t(\"lyrics\")}"
                        }
                    }

                    div {
                        class: "flex-1 overflow-y-auto px-4 py-2 space-y-1",

                        if *active_tab.read() == 2 {
                            div {
                                class: "text-white/70 text-center py-4 px-8 leading-relaxed font-medium text-lg w-full max-w-2xl mx-auto flex flex-col gap-4",
                                match &*lyrics.read() {
                                    Some(Some(utils::lyrics::Lyrics::Synced(lines))) => {
                                        let active_idx = active_lyric_index();

                                        rsx! {
                                            for (i, line) in lines.iter().enumerate() {
                                                div {
                                                    key: "{i}",
                                                    id: if i == active_idx { "active-lyric" } else { "" },
                                                    class: if i == active_idx {
                                                        "text-white text-2xl font-bold transition-all duration-300"
                                                    } else {
                                                        "text-white/40 transition-all duration-300 hover:text-white/60"
                                                    },
                                                    "{line.text}"
                                                }
                                            }
                                        }
                                    }
                                    Some(Some(utils::lyrics::Lyrics::Plain(text))) => {
                                        rsx! {
                                            div { class: "whitespace-pre-wrap", "{text}" }
                                        }
                                    }
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
                                            class: "flex items-center gap-4 px-4 py-3 hover:bg-white/5 cursor-pointer rounded-lg transition-colors group",
                                            onclick: move |_| play_song_at_index(track_idx),
                                            div {
                                                class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                                                style: "width: 48px; height: 48px;",
                                                if let Some(ref url) = cover_url {
                                                    img { src: "{url.as_ref()}", class: "w-full h-full object-cover" }
                                                } else {
                                                    div {
                                                        class: "w-full h-full flex items-center justify-center",
                                                        i { class: "fa-solid fa-music text-white/20", style: "font-size: 14px;" }
                                                    }
                                                }
                                            }
                                            div {
                                                class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                                                div { class: "text-base text-white truncate font-medium", "{track.title}" }
                                                div { class: "text-sm text-white/50 truncate group-hover:text-white/70", "{track.artist}" }
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
                                    class: "px-4 pt-2 pb-3 text-xs uppercase tracking-[0.18em] text-white/45",
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
                                            class: "flex items-center gap-4 px-4 py-3 hover:bg-white/5 cursor-pointer rounded-lg transition-colors group",
                                            onclick: move |_| play_song_at_index(track_idx),
                                            div {
                                                class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                                                style: "width: 48px; height: 48px;",
                                                if let Some(ref url) = cover_url {
                                                    img { src: "{url.as_ref()}", class: "w-full h-full object-cover" }
                                                } else {
                                                    div {
                                                        class: "w-full h-full flex items-center justify-center",
                                                        i { class: "fa-solid fa-music text-white/20", style: "font-size: 14px;" }
                                                    }
                                                }
                                            }
                                            div {
                                                class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                                                div { class: "text-base text-white truncate font-medium", "{track.title}" }
                                                div { class: "text-sm text-white/50 truncate group-hover:text-white/70", "{track.artist}" }
                                            }
                                            ReorderButtons {
                                                can_move_up,
                                                can_move_down,
                                                class: "flex flex-col pr-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity".to_string(),
                                                icon_class: "text-[10px]".to_string(),
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
                } // flex-1 panels row
            }
        }
    }
}
