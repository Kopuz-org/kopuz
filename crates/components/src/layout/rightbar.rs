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
    let api = use_context::<std::sync::Arc<dyn api::KopuzApi>>();

    let mut lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>> = use_signal(|| None);
    let mut fetch_gen: Signal<u32> = use_signal(|| 0);
    let mut last_key: Signal<String> = use_signal(String::new);
    let mut fetch_task = use_signal(|| None::<dioxus_core::Task>);

    use_effect(move || {
        let current_track = ctrl.current_track_snapshot.read().clone();
        let key = current_track
            .as_ref()
            .map(|track| track.id.key().into_owned())
            .unwrap_or_default();
        let title = current_track
            .as_ref()
            .map(|track| track.title.clone())
            .unwrap_or_else(|| current_song_title.read().clone());
        let new_key = format!("{title}|{key}");
        if *last_key.peek() == new_key {
            return;
        }
        last_key.set(new_key);
        if let Some(task) = fetch_task.take() {
            task.cancel();
        }
        let fetch_id = fetch_gen.peek().wrapping_add(1);
        fetch_gen.set(fetch_id);
        if title.is_empty()
            || current_track
                .as_ref()
                .is_some_and(|track| track.duration == u64::MAX)
            || key.is_empty()
        {
            lyrics.set(Some(None));
            return;
        }
        lyrics.set(None);
        let api = api.clone();
        let task = spawn(
            async move {
                use futures_util::StreamExt as _;
                let mut last_displayed: Option<utils::lyrics::Lyrics> = None;
                let mut stream = api.lyrics_stream(key);
                while let Some(result) = stream.next().await {
                    if *fetch_gen.peek() != fetch_id {
                        return;
                    }
                    match result {
                        Ok(value) => {
                            let display = crate::lyrics_view::from_api(value);
                            if last_displayed.as_ref() != Some(&display) {
                                last_displayed = Some(display.clone());
                                lyrics.set(Some(Some(display)));
                            }
                        }
                        Err(_) if last_displayed.is_none() => {
                            lyrics.set(Some(Some(utils::lyrics::Lyrics::Plain(
                                i18n::t("lyrics_not_found").to_string(),
                            ))));
                        }
                        Err(_) => {}
                    }
                }
            }
            .instrument(tracing::info_span!("lyrics.load")),
        );
        fetch_task.set(Some(task));
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
