use config::AppConfig;
use dioxus::document::eval;
use dioxus::prelude::*;
use hooks::use_player_controller::{LoopMode, PlayerController};
use player::player::Player;
use reader::Library;

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
    mut current_song_bitrate: Signal<u8>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_album: Signal<String>,
    mut volume: Signal<f32>,
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
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            exact_progress.set(player.peek().get_position().as_secs_f64());
        }
    });

    let format_time = |seconds: u64| {
        let minutes = seconds / 60;
        let seconds = seconds % 60;
        format!("{}:{:02}", minutes, seconds)
    };

    let progress_percent = if *current_song_duration.read() > 0 {
        (display_progress as f64 / *current_song_duration.read() as f64) * 100.0
    } else {
        0.0
    };

    let volume_percent = *volume.read() * 100.0;

    let mut play_song_at_index = move |index: usize| {
        ctrl.play_track(index);
    };

    let mut config = use_context::<Signal<AppConfig>>();

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
                    let el = document.getElementById('active-lyric');
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
                return match provider {
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
            }
            None
        } else {
            lib.albums
                .iter()
                .find(|a| a.id == track.album_id)
                .and_then(|album| utils::format_artwork_url(album.cover_path.as_ref()))
        }
    };

    let background_style = if config.read().theme == "album-art" {
        utils::color::get_background_style(palette.read().as_deref())
    } else {
        "background-color: var(--color-black); background-image: none;".to_string()
    };

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex text-white select-none",
            style: "{background_style}",

            div {
                class: "flex flex-col items-center justify-center p-8 lg:p-12 relative flex-shrink-0",
                style: "width: 50%; max-width: 600px;",

                div {
                    class: "rounded-2xl overflow-hidden mb-8 shadow-2xl",
                    style: "width: 100%; max-width: 420px; aspect-ratio: 1/1;",
                    if current_song_cover_url.read().is_empty() {
                        div {
                            class: "w-full h-full flex items-center justify-center bg-black/30",
                            i { class: "fa-solid fa-music text-5xl text-white/20" }
                        }
                    } else {
                        img {
                            src: "{current_song_cover_url}",
                            class: "w-full h-full object-cover"
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
                    span { style: "font-size: 10px;", "{current_song_khz} / {current_song_bitrate}" }
                }

                div {
                    class: "w-full mb-6",
                    style: "max-width: 420px;",
                    div {
                        class: "flex items-center gap-3",
                        span { class: "text-xs text-white/70 font-mono", style: "width: 50px; text-align: left;", "{format_time(display_progress)}" }
                        div {
                            class: "flex-1 cursor-pointer relative",
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
                                class: "absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer",
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
                        span { class: "text-xs text-white/70 font-mono", style: "width: 50px; text-align: right;", "{format_time(*current_song_duration.read())}" }
                    }
                }

                div {
                    class: "flex items-center justify-between w-full mb-8",
                    style: "max-width: 420px;",
                    button {
                        class: format!("{} transition-all active:scale-95 relative flex-shrink-0", if *ctrl.shuffle.read() { "text-white" } else { "text-white/50 hover:text-white" }),
                        onclick: move |_| ctrl.toggle_shuffle(),
                        title: if *ctrl.shuffle.read() { "Shuffle: On" } else { "Shuffle: Off" },
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
                            LoopMode::None => "Repeat: Off",
                            LoopMode::Queue => "Repeat: Queue",
                            LoopMode::Track => "Repeat: Track",
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
                                    config.write().volume = val;
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
                        "BACK TO"
                    }
                    button {
                        class: if *active_tab.read() == 1 {
                            "px-4 py-2 text-xs font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-4 py-2 text-xs font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(1),
                        "UP NEXT"
                    }
                    button {
                        class: if *active_tab.read() == 2 {
                            "px-4 py-2 text-xs font-medium tracking-wider text-white border-b-2 border-white"
                        } else {
                            "px-4 py-2 text-xs font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                        },
                        onclick: move |_| active_tab.set(2),
                        "LYRICS"
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
                                        class: "flex items-center gap-4 px-4 py-3 hover:bg-white/5 cursor-pointer rounded-lg transition-colors group",
                                        onclick: move |_| play_song_at_index(i),
                                        div {
                                            class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                                            style: "width: 48px; height: 48px;",
                                            if let Some(ref url) = cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover" }
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
                                        class: "flex items-center gap-4 px-4 py-3 hover:bg-white/5 cursor-pointer rounded-lg transition-colors group",
                                        onclick: move |_| play_song_at_index(i),
                                        div {
                                            class: "rounded-md overflow-hidden bg-black/30 flex-shrink-0 shadow-sm",
                                            style: "width: 48px; height: 48px;",
                                            if let Some(ref url) = cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover" }
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
                }
            }
        }
    }
}
