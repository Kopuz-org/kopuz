use config::AppConfig;
use dioxus::{document::eval, prelude::*};
use hooks::PlayerController;
use std::fmt;

const FULLSCREEN_LYRIC_CLASS: &str = "text-white/40 text-2xl font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap";
const FULLSCREEN_ACTIVE_LYRIC_CLASS: &str =
    "text-white text-2xl font-semibold transition-colors duration-300 whitespace-pre-wrap";
const RIGHTBAR_LYRIC_CLASS: &str = "text-white/40 text-lg font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap";
const RIGHTBAR_ACTIVE_LYRIC_CLASS: &str =
    "text-white text-lg font-semibold transition-colors duration-300 whitespace-pre-wrap";
const LYRIC_STYLE: &str =
    "transform: scale(1); transform-origin: center; transition: color 300ms, transform 300ms;";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutMode {
    Rightbar,
    Fullscreen,
}

impl fmt::Display for LayoutMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutMode::Rightbar => write!(f, "rightbar"),
            LayoutMode::Fullscreen => write!(f, "fullscreen"),
        }
    }
}

#[component]
pub fn LyricsView(
    lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>>,
    current_song_progress: Signal<u64>,
    config: Signal<AppConfig>,
    layout: LayoutMode,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    // Clear functions when the component is dropped
    use_drop(move || {
        let _cleanup = eval(&format!(
            "if (window.__{layout}_updateLyrics) delete window.__{layout}_updateLyrics"
        ));
    });

    use_hook(move || {
        let (inactive_class, active_class) = match layout {
            LayoutMode::Fullscreen => (FULLSCREEN_LYRIC_CLASS, FULLSCREEN_ACTIVE_LYRIC_CLASS),
            LayoutMode::Rightbar => (RIGHTBAR_LYRIC_CLASS, RIGHTBAR_ACTIVE_LYRIC_CLASS),
        };

        let _update_func = eval(&format!(
            r#"
                let currEl;
                let activeClass = "{active_class}";
                let inactiveClass = "{inactive_class}";

                const resetWords = (lineEl) => {{
                    lineEl?.querySelectorAll('[data-lyric-word]').forEach((word) => {{
                        word.style.opacity = '';
                        word.style.textShadow = '';
                    }});
                }};

                const updateWords = (lineEl, activeWordIndex) => {{
                    lineEl?.querySelectorAll('[data-lyric-word]').forEach((word, index) => {{
                        if (activeWordIndex >= 0 && index <= activeWordIndex) {{
                            word.style.opacity = '1';
                            word.style.textShadow = '0 0 12px rgba(255,255,255,0.72)';
                        }} else {{
                            word.style.opacity = '0.45';
                            word.style.textShadow = '';
                        }}
                    }});
                }};

                window.__{layout}_updateLyrics = (nextIndex, nextWordIndex) => {{
                    let nextEl = document.getElementById(`{layout}-lyrics-${{nextIndex}}`)
                    if (currEl != nextEl) {{
                        if (currEl) {{
                            currEl.className = inactiveClass;
                            currEl.style.transform = 'scale(1)';
                            resetWords(currEl);
                        }}

                        if (nextEl) {{
                            nextEl.className = activeClass;
                            nextEl.style.transform = 'scale(1.12)';
                            nextEl.scrollIntoView({{ behavior: 'smooth', block: 'center' }});
                        }}

                        currEl = nextEl;
                    }}

                    if (nextEl && nextEl.querySelector('[data-lyric-word]')) {{
                        updateWords(nextEl, nextWordIndex);
                    }}
                }}
            "#,
        ));
    });

    use_resource(move || {
        let lyrics = lyrics.read().clone();

        // scroll to top on lyrics change
        let _scroll_to_top = eval(&format!(
            "document.getElementById('{layout}-lyrics-content')?.scrollTo({{ top: 0, left: 0, behavior: 'smooth' }});"
        ));

        async move {
            if let Some(Some(utils::lyrics::Lyrics::Synced(lines))) = lyrics {
                let mut sleep_duration_ms: u64;

                let times = lines.iter().map(|l| l.start_time).collect::<Vec<_>>();

                loop {
                    let current_time = ctrl.displayed_progress_secs_f64();
                    // Binary search to find the active line.
                    // `partition_point(|t| t <= current_time)` returns the index of the first
                    // timestamp greater than `current_time`.
                    // Therefore `n - 1` is the last timestamp less than or equal to it.
                    // If the result is 0, we are before the first line.
                    if let Some(current_line_index) =
                        match times.partition_point(|&t| t <= current_time) {
                            0 => None,
                            n => Some(n - 1),
                        }
                    {
                        let current_word_index = lines[current_line_index]
                            .words
                            .partition_point(|w| w.start_time <= current_time)
                            .checked_sub(1)
                            .map(|i| i as i64)
                            .unwrap_or(-1);
                        let _ = eval(&format!(
                            "window.__{layout}_updateLyrics({current_line_index}, {current_word_index})"
                        ));

                        sleep_duration_ms = times
                            .get(current_line_index.saturating_add(1))
                            .map(|next_time| {
                                ((*next_time - current_time) * 1000.0).max(16.0).min(50.0) as u64
                            })
                            .unwrap_or(50);
                    } else {
                        // we are before the first line, invalidate current line
                        let _ = eval(&format!("window.__{layout}_updateLyrics(-1, -1)"));
                        sleep_duration_ms = 50;
                    }

                    utils::sleep(std::time::Duration::from_millis(sleep_duration_ms)).await;
                }
            }
        }
    });

    rsx! {
        div {
            id: "{layout}-lyrics-content",
            class: match layout {
                LayoutMode::Fullscreen => "flex-1 overflow-y-auto px-4 py-2 space-y-1",
                LayoutMode::Rightbar => "flex-1 overflow-y-auto px-2 py-2 space-y-1",
            },

            div {
                class: match layout {
                    LayoutMode::Fullscreen => "text-white/70 text-center py-4 px-8 leading-relaxed font-medium text-lg w-full max-w-2xl mx-auto flex flex-col gap-4",
                    LayoutMode::Rightbar =>
                    "text-white/70 text-center py-4 px-4 leading-relaxed font-medium text-sm flex flex-col gap-4"
                },
                match &*lyrics.read() {
                    Some(Some(utils::lyrics::Lyrics::Synced(lines))) => {
                        rsx! {
                            for (i, line) in lines.iter().enumerate() {
                                div {
                                    key: "{i}",
                                    id: "{layout}-lyrics-{i}",
                                    class: match layout {
                                        LayoutMode::Fullscreen => FULLSCREEN_LYRIC_CLASS,
                                        LayoutMode::Rightbar => RIGHTBAR_LYRIC_CLASS,
                                    },
                                    style: LYRIC_STYLE,
                                    onclick: {
                                        let st = line.start_time;
                                        move |_| {
                                            ctrl.player.write().seek(std::time::Duration::from_secs_f64(st));
                                            current_song_progress.set(st as u64);
                                        }
                                    },
                                    if line.words.is_empty() {
                                        "{line.text}"
                                    } else {
                                        for (word_i, word) in line.words.iter().enumerate() {
                                            span {
                                                key: "{word_i}",
                                                id: "{layout}-lyrics-{i}-word-{word_i}",
                                                "data-lyric-word": "true",
                                                class: "transition-opacity duration-150",
                                                "{word.text}"
                                            }
                                        }
                                    }
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
}
