use crate::NavigationController;
use crate::lyrics_view::LyricsView;
use crate::queue_list_view::QueueListView;
use crate::shared::fmt_time;
use crate::titlebar::Titlebar;
use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_player_controller::{LoopMode, PlayerController};
use player::player::Player;

#[component]
fn ProgressBarControl(
    mut player: Signal<Player>,
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
) -> Element {
    let mut is_dragging = use_signal(|| false);
    let mut drag_progress = use_signal(|| 0u64);

    let display_progress = if *is_dragging.read() {
        *drag_progress.read()
    } else {
        *current_song_progress.read()
    };

    let progress_percent = if *current_song_duration.read() > 0 {
        (display_progress as f64 / *current_song_duration.read() as f64) * 100.0
    } else {
        0.0
    };

    let is_radio = *current_song_duration.read() == u64::MAX;

    rsx! {
        div {
            class: "w-full mb-6",
            style: "max-width: 520px;",
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
    }
}

#[component]
fn VolumeControl(
    mut player: Signal<Player>,
    config: Signal<AppConfig>,
    persisted_volume: Signal<f32>,
    volume: Signal<f32>,
) -> Element {
    let volume_percent = *volume.read() * 100.0;

    rsx! {
        div {
            class: "flex items-center gap-5 w-full",
            style: "max-width: 520px;",
            i { class: "fa-solid fa-volume-low text-white/40" }
            div {
                class: "flex-1 cursor-pointer relative",
                style: "height: 20px;",
                onwheel: move |evt| {
                    evt.stop_propagation();
                    let dy = evt.delta().strip_units().y;
                    if dy.abs() < f64::EPSILON {
                        return;
                    }
                    let step = config.read().volume_scroll_step.max(0.0);
                    let dir = if dy < 0.0 { 1.0 } else { -1.0 };
                    let current = *volume.read();
                    let new_val = (current + dir * step).clamp(0.0, 1.0);
                    player.write().set_volume(new_val);
                    volume.set(new_val);
                    persisted_volume.set(new_val);
                },
                div {
                    class: "absolute bg-white/20 rounded-full",
                    style: "height: 4px; top: 8px; left: 0; right: 0;"
                }
                div {
                    class: "absolute bg-white rounded-full pointer-events-none",
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
    }
}

#[component]
fn PlaybackControl(mut is_playing: Signal<bool>) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    rsx! {
        div {
            class: "flex items-center justify-between w-full mb-8",
            style: "max-width: 520px;",
            button {
                class: format!("{} transition-all active:scale-95 relative flex-shrink-0", if *ctrl.shuffle.read() { "text-white" } else { "text-white/70 hover:text-white" }),
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
                        LoopMode::None => "text-white/70 hover:text-white",
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
    }
}

#[component]
fn TrackMetadata(
    mut is_fullscreen: Signal<bool>,
    current_song_cover_url: Signal<String>,
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_album: Signal<String>,
    current_song_bitrate: Signal<u16>,
) -> Element {
    let ctrl = use_context::<PlayerController>();
    let nav_ctrl = use_context::<NavigationController>();
    let current_track_snapshot = ctrl.current_track_snapshot.read().clone();

    rsx! {
        div {
            class: "rounded-lg overflow-hidden mb-8",
            style: "width: 100%; max-width: 520px; aspect-ratio: 1/1; box-shadow: 0 25px 60px -15px rgba(0,0,0,0.55);",
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
                            class: "w-full h-full object-contain"
                        }
                    }
                }
            }
        }

        div {
            class: "flex flex-col items-start w-full mb-2",
            style: "max-width: 520px;",
            h1 { class: "text-3xl font-bold text-white mb-2 line-clamp-2 w-full", "{current_song_title}" }
            div {
                class: "flex flex-wrap items-center gap-x-2 gap-y-1 w-full",
                button {
                    class: "text-xl text-white/70 font-medium line-clamp-2 max-w-full hover:text-white hover:underline text-left transition-colors",
                    onclick: move |_| {
                        let artist = current_song_artist.read().clone();
                        if artist.is_empty() {
                            return;
                        }
                        is_fullscreen.set(false);
                        nav_ctrl.navigate_to_artist(artist);
                    },
                    "{current_song_artist}"
                }
                span { class: "text-white/30 flex-shrink-0", "•" }
                button {
                    class: "text-lg text-white/50 line-clamp-2 max-w-full hover:text-white/80 hover:underline text-left transition-colors",
                    onclick: move |_| {
                        let album_id = current_track_snapshot
                            .as_ref()
                            .map(|track| track.album_id.clone())
                            .unwrap_or_default();
                        if album_id.is_empty() {
                            return;
                        }
                        is_fullscreen.set(false);
                        nav_ctrl.navigate_to_album(album_id);
                    },
                    "{current_song_album}"
                }
            }
        }

        div {
            class: "flex items-center gap-4 text-xs text-white/50 mb-6 w-full",
            style: "max-width: 520px;",
            if current_song_bitrate() > 0 {
                span { style: "font-size: 10px;", "{current_song_bitrate} kbps" }
            }
        }
    }
}

#[component]
fn Tabs(
    config: Signal<AppConfig>,
    items: Vec<reader::Track>,
    current_queue_index: Signal<usize>,
    lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>>,
    current_song_progress: Signal<u64>,
    player: Signal<Player>,
    volume: Signal<f32>,
    persisted_volume: Signal<f32>,
) -> Element {
    let mut active_tab = use_signal(|| 0usize);

    rsx! {
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
                    "{i18n::t(\"up_next\")}"
                }

                button {
                    class: if *active_tab.read() == 1 {
                        "px-4 py-2 text-xs font-medium tracking-wider text-white border-b-2 border-white"
                    } else {
                        "px-4 py-2 text-xs font-medium tracking-wider text-white/40 hover:text-white/70 transition-colors"
                    },
                    onclick: move |_| active_tab.set(1),
                    "{i18n::t(\"lyrics\")}"
                }

                div {
                    class: "ml-auto flex items-center",
                    style: "width: 160px;",
                    VolumeControl { player, config, volume, persisted_volume }
                }
            }

            if *active_tab.read() == 0 {
                QueueListView {
                    items,
                    config,
                    current_queue_index,
                    layout: crate::queue_list_view::LayoutMode::Fullscreen,
                }
            } else if *active_tab.read() == 1 {
                LyricsView {
                    lyrics,
                    current_song_progress,
                    config,
                    layout: crate::lyrics_view::LayoutMode::Fullscreen,
                }
            }
        } // flex-1 panels row
    }
}

#[component]
pub fn Fullscreen(
    mut player: Signal<Player>,
    mut is_playing: Signal<bool>,
    mut is_fullscreen: Signal<bool>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    queue: Signal<Vec<reader::Track>>,
    mut current_queue_index: Signal<usize>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_bitrate: Signal<u16>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_album: Signal<String>,
    mut volume: Signal<f32>,
    mut persisted_volume: Signal<f32>,
    palette: Signal<Option<Vec<utils::color::Color>>>,
) -> Element {
    if !*is_fullscreen.read() {
        return rsx! { div {} };
    }

    let ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();

    let mut lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>> = use_signal(|| None);
    let mut fetch_gen: Signal<u32> = use_signal(|| 0);
    let mut last_key: Signal<String> = use_signal(String::new);

    use_effect(move || {
        let current_track = ctrl.current_track_snapshot.read().clone();

        let (title, artist, album, duration, track_path) = if let Some(track) = current_track {
            (
                track.title,
                track.artist,
                track.album,
                track.duration,
                track.id.uid(),
            )
        } else {
            (
                current_song_title.read().clone(),
                current_song_artist.read().clone(),
                current_song_album.read().clone(),
                *current_song_duration.read(),
                String::new(),
            )
        };

        let new_key = format!("{title}|{track_path}");
        if *last_key.peek() == new_key {
            return;
        }
        last_key.set(new_key);
        let (server_url, server_token, server_user_id, prefer_local, enable_musixmatch) = {
            let conf = config.peek();
            let prefer_local = conf.prefer_local_lyrics;
            let enable_musixmatch = conf.enable_musixmatch_lyrics;
            if let Some(server) = &conf.server {
                (
                    Some(server.url.clone()),
                    server.access_token.clone(),
                    server.user_id.clone(),
                    prefer_local,
                    enable_musixmatch,
                )
            } else {
                (None, None, None, prefer_local, enable_musixmatch)
            }
        };

        let fetch_id = fetch_gen.peek().wrapping_add(1);
        fetch_gen.set(fetch_id);

        if title.is_empty() {
            lyrics.set(Some(None));
            return;
        }

        let track_path_for_spawn = track_path.clone();
        let lyrics_request =
            utils::lyrics::LyricsRequest::new(artist, title, album, duration, track_path)
                .with_server(
                    server_url.as_deref(),
                    server_token.as_deref(),
                    server_user_id.as_deref(),
                )
                .prefer_local(prefer_local)
                .enable_musixmatch(enable_musixmatch);

        if let Some(cached) = utils::lyrics::cached_lyrics_for_request(&lyrics_request) {
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
            // Lazily attach Apple Music auth for the lyrics provider.
            let lyrics_request = if track_path_for_spawn.starts_with("applemusic:") {
                let am_auth = config.peek().server.as_ref().and_then(|server| {
                    if server.service != config::MusicService::AppleMusic {
                        return None;
                    }
                    let token = server.access_token.clone()?;
                    let catalog_id = track_path_for_spawn
                        .strip_prefix("applemusic:")
                        .unwrap_or(&track_path_for_spawn)
                        .to_string();
                    Some(utils::lyrics::AppleMusicLyricsAuth {
                        token,
                        bearer_token: String::new(),
                        storefront: server.apple_music_storefront.clone(),
                        language: server.apple_music_language.clone(),
                        catalog_id,
                    })
                });
                if let Some(mut auth) = am_auth {
                    if let Ok(bt) = ::server::applemusic::auth::get_bearer_token().await {
                        auth.bearer_token = bt;
                    }
                    lyrics_request.apple_music_auth(auth)
                } else {
                    lyrics_request
                }
            } else {
                lyrics_request
            };
            let mut last_displayed: Option<utils::lyrics::Lyrics> = None;
            let result =
                utils::lyrics::fetch_lyrics_progressive_for_request(&lyrics_request, |partial| {
                    if *fetch_gen.peek() == fetch_id && last_displayed.as_ref() != Some(&partial) {
                        last_displayed = Some(partial.clone());
                        lyrics.set(Some(Some(partial)));
                    }
                })
                .await;
            if *fetch_gen.peek() == fetch_id {
                let display = result.or_else(|| {
                    Some(utils::lyrics::Lyrics::Plain(
                        i18n::t("lyrics_not_found").to_string(),
                    ))
                });
                if display.as_ref() != last_displayed.as_ref() {
                    lyrics.set(Some(display));
                }
            }
        });
    });

    let background_style = use_memo(move || {
        if config.read().theme == "album-art" {
            utils::color::get_background_style(palette.read().as_deref())
        } else {
            "background-color: var(--color-black); background-image: none;".to_string()
        }
    });

    let items = {
        let q = queue.read();
        let is_shuffle = *ctrl.shuffle.read();

        if is_shuffle {
            ctrl.shuffle_order
                .read()
                .iter()
                .filter_map(|&qi| q.get(qi).cloned())
                .collect::<Vec<_>>()
        } else {
            (0..q.len())
                .filter_map(|qi| q.get(qi).cloned())
                .collect::<Vec<_>>()
        }
    };

    let mut active_tab = use_signal(|| 0usize);
    if cfg!(target_os = "android") {
        let tab = *active_tab.read();
        let close_text = i18n::t("close").to_string();
        let music_text = i18n::t("music").to_string();
        let up_next_text = i18n::t("up_next").to_string();
        let lyrics_text = i18n::t("lyrics").to_string();
        let tab_btn = |idx: usize, icon: &'static str, label: String| {
            let cls = if tab == idx {
                "flex-1 h-10 flex items-center justify-center text-white border-b-2 border-white"
            } else {
                "flex-1 h-10 flex items-center justify-center text-white/40 border-b-2 border-transparent"
            };
            rsx! {
                button { class: "{cls}", "aria-label": label, onclick: move |_| active_tab.set(idx),
                    i { class: "{icon} text-base", "aria-hidden": "true" }
                }
            }
        };
        return rsx! {
            div {
                class: "fixed inset-0 z-50 flex flex-col text-white select-none",
                style: "{background_style.read()}",

                div {
                    class: "flex items-center gap-2 px-3 pt-[env(safe-area-inset-top)] pb-1 shrink-0",
                    button {
                        class: "w-10 h-10 flex items-center justify-center text-white/60 active:scale-95 transition-all shrink-0",
                        "aria-label": "{close_text}",
                        onclick: move |_| is_fullscreen.set(false),
                        i { class: "fa-solid fa-chevron-down text-xl", "aria-hidden": "true" }
                    }
                    div { class: "flex flex-1 items-center",
                        {tab_btn(0, "fa-solid fa-compact-disc", music_text.clone())}
                        {tab_btn(1, "fa-solid fa-list", up_next_text.clone())}
                        {tab_btn(2, "fa-solid fa-align-left", lyrics_text.clone())}
                    }
                }

                if tab == 0 {
                    div {
                        class: "flex-1 overflow-y-auto flex flex-col items-center justify-center px-6 pb-[calc(env(safe-area-inset-bottom)_+_1.5rem)]",
                        TrackMetadata {
                            is_fullscreen,
                            current_song_cover_url,
                            current_song_title,
                            current_song_artist,
                            current_song_album,
                            current_song_bitrate,
                        }
                        ProgressBarControl { player, current_song_duration, current_song_progress }
                        PlaybackControl { is_playing }
                        VolumeControl { player, config, volume, persisted_volume }
                    }
                } else if tab == 1 {
                    QueueListView {
                        items,
                        config,
                        current_queue_index,
                        layout: crate::queue_list_view::LayoutMode::Fullscreen,
                    }
                } else {
                    LyricsView {
                        lyrics,
                        current_song_progress,
                        config,
                        layout: crate::lyrics_view::LayoutMode::Fullscreen,
                    }
                }
            }
        };
    }

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex flex-col text-white select-none",
            style: "{background_style.read()}",

            if cfg!(any(target_os = "linux", target_os = "windows")) {
                div { dir: "ltr", Titlebar {} }
            }

            div {
                class: "flex flex-1 overflow-hidden",

                div {
                    class: "flex flex-col items-center justify-center p-8 lg:p-12 relative flex-shrink-0 overflow-hidden",
                    style: "width: 50%; max-width: 600px;",

                    button {
                        class: "absolute top-8 left-8 text-white/30 hover:text-white transition-colors z-10",
                        onclick: move |_| is_fullscreen.set(false),
                        i { class: "fa-solid fa-chevron-down text-2xl" }
                    }

                    TrackMetadata {
                        is_fullscreen,
                        current_song_cover_url,
                        current_song_title,
                        current_song_artist,
                        current_song_album,
                        current_song_bitrate,
                    }

                    ProgressBarControl {
                        player,
                        current_song_duration,
                        current_song_progress,
                    }

                    PlaybackControl {
                        is_playing
                    }
                }

                Tabs {
                    config,
                    items,
                    current_queue_index,
                    lyrics,
                    current_song_progress,
                    player,
                    volume,
                    persisted_volume,
                }
            }
        }
    }
}
