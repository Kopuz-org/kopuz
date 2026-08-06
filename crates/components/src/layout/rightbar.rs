use crate::lyrics_view::LyricsView;
use crate::queue_list_view::QueueListView;
use config::AppConfig;
use dioxus::document::eval;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;
use serde_json::Value;
use tracing::Instrument;

#[component]
pub fn Rightbar(
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
    let mut active_tab = use_signal(|| 0usize);
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

        // Radio has no lyrics; querying providers with station names only
        // produces junk matches.
        if title.is_empty() || hooks::playback_ref::PlaybackItemRef::parse(&track_path).is_radio() {
            lyrics.set(Some(None));
            return;
        }

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

        spawn(
            async move {
                let mut last_displayed: Option<utils::lyrics::Lyrics> = None;
                let result = utils::lyrics::fetch_lyrics_progressive_for_request(
                    &lyrics_request,
                    |partial| {
                        if *fetch_gen.peek() == fetch_id
                            && last_displayed.as_ref() != Some(&partial)
                        {
                            last_displayed = Some(partial.clone());
                            lyrics.set(Some(Some(partial)));
                        }
                    },
                )
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
            }
            .instrument(tracing::info_span!("lyrics.load")),
        );
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
                        let new_width = w.clamp(280.0, 600.0);
                        width.set(new_width as usize);
                    } else if val.as_str() == Some("stop") {
                        is_resizing.set(false);
                        break;
                    }
                }
            });
        }
    });

    let up_next_text = i18n::t("up_next").to_string();
    let lyrics_text = i18n::t("lyrics").to_string();

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

    if !*is_rightbar_open.read() {
        return rsx! { div {} };
    }

    rsx! {
        div {
            id: "rightbar-root",
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
                class: "flex items-center justify-between px-4 py-3 border-b border-white/10",
                div {
                    class: "flex items-center gap-1 p-1 rounded-lg bg-white/10",
                    button {
                        class: if *active_tab.read() == 0 {
                            "px-4 py-1.5 text-xs font-medium rounded-md bg-white/20 text-white transition-colors"
                        } else {
                            "px-4 py-1.5 text-xs font-medium rounded-md text-white/50 hover:text-white/80 transition-colors"
                        },
                        onclick: move |_| active_tab.set(0),
                        "{up_next_text}"
                    }
                    button {
                        class: if *active_tab.read() == 1 {
                            "px-4 py-1.5 text-xs font-medium rounded-md bg-white/20 text-white transition-colors"
                        } else {
                            "px-4 py-1.5 text-xs font-medium rounded-md text-white/50 hover:text-white/80 transition-colors"
                        },
                        onclick: move |_| active_tab.set(1),
                        "{lyrics_text}"
                    }
                }
                button {
                    class: "w-9 h-9 rounded-full flex items-center justify-center text-white/40 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                    onclick: move |_| is_rightbar_open.set(false),
                    i { class: "fa-solid fa-xmark text-sm" }
                }
            }

            if *active_tab.read() == 0 {
                QueueListView {
                    items,
                    config,
                    current_queue_index,
                    layout: crate::queue_list_view::LayoutMode::Rightbar,
                }
            } else if *active_tab.read() == 1 {
                LyricsView {
                    lyrics,
                    current_song_progress,
                    config,
                    layout: crate::lyrics_view::LayoutMode::Rightbar,
                }
            }
        }
    }
}
