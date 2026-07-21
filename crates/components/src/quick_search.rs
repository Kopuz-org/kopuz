use crate::shared::fmt_time;
use config::AppConfig;
use dioxus::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

const SHOWN_RESULTS: usize = 8;

/// Spotlight-style palette: type, Enter, and the track plays. The actual
/// playback (full library as the queue, so shuffle keeps covering the whole
/// library) happens in the parent via `on_play`, which outlives this palette.
/// Enter before the search resolves queues an auto-play of the top match once
/// results land.
#[component]
pub fn QuickSearch(
    mut show: Signal<bool>,
    on_play: EventHandler<(reader::Track, Vec<reader::Track>)>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let mut input_text = use_signal(String::new);
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);
    let mut pending_play = use_signal(|| false);
    let mut cached: Signal<Vec<(reader::Track, Option<utils::CoverUrl>)>> = use_signal(Vec::new);
    let mut results_ready = use_signal(|| false);
    let debounce_gen = use_hook(|| Arc::new(AtomicU64::new(0)));
    let search_data = hooks::use_search_data(query, config);

    use_effect(move || {
        let results = search_data.search_results.read();
        match results.as_ref() {
            Some(Some((tracks, _))) => {
                cached.set(tracks.clone());
                results_ready.set(true);
            }
            Some(None) => {
                cached.set(Vec::new());
                results_ready.set(false);
            }
            None => {}
        }
    });

    let play_track = use_callback(
        move |(track, fallback): (reader::Track, Vec<reader::Track>)| {
            show.set(false);
            on_play.call((track, fallback));
        },
    );

    let play_selected = use_callback(move |_: ()| {
        let (track, fallback) = {
            let list = cached.peek();
            let sel = (*selected.peek()).min(list.len().saturating_sub(1));
            (
                list.get(sel).map(|(t, _)| t.clone()),
                list.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
            )
        };
        if let Some(track) = track {
            play_track.call((track, fallback));
        }
    });

    use_effect(move || {
        if *pending_play.read() && *results_ready.read() {
            pending_play.set(false);
            selected.set(0);
            play_selected.call(());
        }
    });

    let top: Vec<(reader::Track, Option<utils::CoverUrl>)> =
        cached.read().iter().take(SHOWN_RESULTS).cloned().collect();
    let has_query = !query.read().trim().is_empty();
    let sel = (*selected.read()).min(top.len().saturating_sub(1));

    rsx! {
        div {
            class: "fixed inset-0 bg-black/60 flex items-start justify-center pt-32 px-4",
            style: "z-index: 70;",
            onclick: move |_| show.set(false),
            div {
                class: "bg-neutral-900 rounded-xl border border-white/10 w-full max-w-xl shadow-2xl overflow-hidden",
                onclick: move |evt| evt.stop_propagation(),
                div {
                    class: "flex items-center gap-3 px-5 border-b border-white/10",
                    i { class: "fa-solid fa-magnifying-glass text-slate-500" }
                    input {
                        id: "quick-search-input",
                        r#type: "text",
                        placeholder: "{i18n::t(\"quick_search_placeholder\")}",
                        class: "w-full bg-transparent text-white py-4 text-base focus:outline-none placeholder:text-white/30",
                        autofocus: true,
                        onmounted: move |_| {
                            let _ = dioxus::document::eval(
                                "document.getElementById('quick-search-input').focus();",
                            );
                        },
                        oninput: move |evt| {
                            selected.set(0);
                            pending_play.set(false);
                            let value = evt.value();
                            input_text.set(value.clone());
                            let debounce_gen = debounce_gen.clone();
                            let tick = debounce_gen.fetch_add(1, Ordering::Relaxed) + 1;
                            spawn(async move {
                                tokio::time::sleep(Duration::from_millis(150)).await;
                                if debounce_gen.load(Ordering::Relaxed) == tick {
                                    query.set(value);
                                }
                            });
                        },
                        onkeydown: move |evt| {
                            match evt.key() {
                                Key::Escape => show.set(false),
                                Key::Enter => {
                                    let has_results = !cached.peek().is_empty();
                                    if has_results {
                                        play_selected.call(());
                                    } else {
                                        let text = input_text.peek().clone();
                                        if !text.trim().is_empty() {
                                            query.set(text);
                                            pending_play.set(true);
                                        }
                                    }
                                }
                                Key::ArrowDown => {
                                    let max = cached
                                        .peek()
                                        .len()
                                        .min(SHOWN_RESULTS)
                                        .saturating_sub(1);
                                    let cur = *selected.peek();
                                    selected.set((cur + 1).min(max));
                                    evt.prevent_default();
                                }
                                Key::ArrowUp => {
                                    let cur = *selected.peek();
                                    selected.set(cur.saturating_sub(1));
                                    evt.prevent_default();
                                }
                                _ => {}
                            }
                            evt.stop_propagation();
                        },
                    }
                }
                if !top.is_empty() {
                    div {
                        class: "py-2 max-h-96 overflow-y-auto",
                        for (i, (track, cover_url)) in top.iter().enumerate() {
                            {
                                let track_click = track.clone();
                                let fallback: Vec<reader::Track> =
                                    cached.read().iter().map(|(t, _)| t.clone()).collect();
                                rsx! {
                                    div {
                                        key: "{track.id.uid()}",
                                        class: if i == sel {
                                            "flex items-center gap-3 px-4 py-2 bg-white/10 cursor-pointer"
                                        } else {
                                            "flex items-center gap-3 px-4 py-2 hover:bg-white/5 cursor-pointer"
                                        },
                                        onmouseenter: move |_| selected.set(i),
                                        onclick: move |_| play_track.call((track_click.clone(), fallback.clone())),
                                        if config.read().show_row_images {
                                            div {
                                                class: "w-9 h-9 bg-white/5 rounded overflow-hidden shrink-0 flex items-center justify-center",
                                                if let Some(url) = cover_url {
                                                    img {
                                                        src: "{url.as_ref()}",
                                                        class: "w-full h-full object-cover",
                                                        loading: "lazy",
                                                        decoding: "async",
                                                    }
                                                } else {
                                                    i { class: "fa-solid fa-music text-white/20 text-xs" }
                                                }
                                            }
                                        }
                                        div {
                                            class: "flex flex-col min-w-0 flex-1",
                                            span { class: "text-sm text-white/90 truncate", "{track.title}" }
                                            span { class: "text-xs text-slate-400 truncate", "{track.artist}" }
                                        }
                                        span { class: "text-xs text-slate-500 font-mono shrink-0", "{fmt_time(track.duration)}" }
                                    }
                                }
                            }
                        }
                    }
                } else if has_query && *results_ready.read() {
                    div {
                        class: "px-5 py-6 text-sm text-white/40",
                        {i18n::t_with("no_results_found", &[("query", query.read().clone())])}
                    }
                }
            }
        }
    }
}
