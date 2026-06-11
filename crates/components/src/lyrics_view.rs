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
const FULLSCREEN_MAIN_LYRIC_CLASS: &str = "text-white/40 text-2xl font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap text-left w-full";
const FULLSCREEN_ACTIVE_MAIN_LYRIC_CLASS: &str = "text-white text-2xl font-semibold transition-colors duration-300 whitespace-pre-wrap text-left w-full";
const RIGHTBAR_MAIN_LYRIC_CLASS: &str = "text-white/40 text-lg font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap text-left w-full";
const RIGHTBAR_ACTIVE_MAIN_LYRIC_CLASS: &str = "text-white text-lg font-semibold transition-colors duration-300 whitespace-pre-wrap text-left w-full";
const FULLSCREEN_CENTER_LYRIC_CLASS: &str = "text-white/40 text-2xl font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap text-center w-full";
const FULLSCREEN_ACTIVE_CENTER_LYRIC_CLASS: &str = "text-white text-2xl font-semibold transition-colors duration-300 whitespace-pre-wrap text-center w-full";
const RIGHTBAR_CENTER_LYRIC_CLASS: &str = "text-white/40 text-lg font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap text-center w-full";
const RIGHTBAR_ACTIVE_CENTER_LYRIC_CLASS: &str = "text-white text-lg font-semibold transition-colors duration-300 whitespace-pre-wrap text-center w-full";
const LYRIC_STYLE: &str = "box-sizing: border-box; overflow-wrap: normal; word-break: normal; transform: scale(1); transition: color 300ms, transform 300ms, opacity 180ms, max-height 180ms, margin-top 180ms;";
const FULLSCREEN_BACKGROUND_LYRIC_CLASS: &str = "text-white/25 text-xl font-medium transition-colors duration-300 whitespace-pre-wrap text-left w-full pl-6 leading-snug";
const FULLSCREEN_ACTIVE_BACKGROUND_LYRIC_CLASS: &str = "text-white/70 text-xl font-medium transition-colors duration-300 whitespace-pre-wrap text-left w-full pl-6 leading-snug";
const RIGHTBAR_BACKGROUND_LYRIC_CLASS: &str = "text-white/25 text-sm font-medium transition-colors duration-300 whitespace-pre-wrap text-left w-full pl-4 leading-snug";
const RIGHTBAR_ACTIVE_BACKGROUND_LYRIC_CLASS: &str = "text-white/70 text-sm font-medium transition-colors duration-300 whitespace-pre-wrap text-left w-full pl-4 leading-snug";
const FULLSCREEN_BACKGROUND_OPPOSITE_LYRIC_CLASS: &str = "text-white/25 text-xl font-medium transition-colors duration-300 whitespace-pre-wrap text-right w-full pr-6 leading-snug";
const FULLSCREEN_ACTIVE_BACKGROUND_OPPOSITE_LYRIC_CLASS: &str = "text-white/70 text-xl font-medium transition-colors duration-300 whitespace-pre-wrap text-right w-full pr-6 leading-snug";
const RIGHTBAR_BACKGROUND_OPPOSITE_LYRIC_CLASS: &str = "text-white/25 text-sm font-medium transition-colors duration-300 whitespace-pre-wrap text-right w-full pr-4 leading-snug";
const RIGHTBAR_ACTIVE_BACKGROUND_OPPOSITE_LYRIC_CLASS: &str = "text-white/70 text-sm font-medium transition-colors duration-300 whitespace-pre-wrap text-right w-full pr-4 leading-snug";
const FULLSCREEN_OPPOSITE_LYRIC_CLASS: &str = "text-white/40 text-2xl italic font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap text-right w-full";
const FULLSCREEN_ACTIVE_OPPOSITE_LYRIC_CLASS: &str = "text-white text-2xl italic font-semibold transition-colors duration-300 whitespace-pre-wrap text-right w-full";
const RIGHTBAR_OPPOSITE_LYRIC_CLASS: &str = "text-white/40 text-lg italic font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap text-right w-full";
const RIGHTBAR_ACTIVE_OPPOSITE_LYRIC_CLASS: &str = "text-white text-lg italic font-semibold transition-colors duration-300 whitespace-pre-wrap text-right w-full";

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

fn lyric_line_class(
    layout: LayoutMode,
    line: &utils::lyrics::LyricLine,
    active: bool,
    has_opposite_turn: bool,
) -> &'static str {
    match (
        layout,
        line.background,
        line.opposite_turn,
        has_opposite_turn,
        active,
    ) {
        (LayoutMode::Fullscreen, true, false, _, false) => FULLSCREEN_BACKGROUND_LYRIC_CLASS,
        (LayoutMode::Fullscreen, true, false, _, true) => FULLSCREEN_ACTIVE_BACKGROUND_LYRIC_CLASS,
        (LayoutMode::Rightbar, true, false, _, false) => RIGHTBAR_BACKGROUND_LYRIC_CLASS,
        (LayoutMode::Rightbar, true, false, _, true) => RIGHTBAR_ACTIVE_BACKGROUND_LYRIC_CLASS,
        (LayoutMode::Fullscreen, true, true, _, false) => {
            FULLSCREEN_BACKGROUND_OPPOSITE_LYRIC_CLASS
        }
        (LayoutMode::Fullscreen, true, true, _, true) => {
            FULLSCREEN_ACTIVE_BACKGROUND_OPPOSITE_LYRIC_CLASS
        }
        (LayoutMode::Rightbar, true, true, _, false) => RIGHTBAR_BACKGROUND_OPPOSITE_LYRIC_CLASS,
        (LayoutMode::Rightbar, true, true, _, true) => {
            RIGHTBAR_ACTIVE_BACKGROUND_OPPOSITE_LYRIC_CLASS
        }
        (LayoutMode::Fullscreen, false, true, _, false) => FULLSCREEN_OPPOSITE_LYRIC_CLASS,
        (LayoutMode::Fullscreen, false, true, _, true) => FULLSCREEN_ACTIVE_OPPOSITE_LYRIC_CLASS,
        (LayoutMode::Rightbar, false, true, _, false) => RIGHTBAR_OPPOSITE_LYRIC_CLASS,
        (LayoutMode::Rightbar, false, true, _, true) => RIGHTBAR_ACTIVE_OPPOSITE_LYRIC_CLASS,
        (LayoutMode::Fullscreen, false, false, true, false) => FULLSCREEN_MAIN_LYRIC_CLASS,
        (LayoutMode::Fullscreen, false, false, true, true) => FULLSCREEN_ACTIVE_MAIN_LYRIC_CLASS,
        (LayoutMode::Rightbar, false, false, true, false) => RIGHTBAR_MAIN_LYRIC_CLASS,
        (LayoutMode::Rightbar, false, false, true, true) => RIGHTBAR_ACTIVE_MAIN_LYRIC_CLASS,
        (LayoutMode::Fullscreen, false, false, false, false) => FULLSCREEN_CENTER_LYRIC_CLASS,
        (LayoutMode::Fullscreen, false, false, false, true) => FULLSCREEN_ACTIVE_CENTER_LYRIC_CLASS,
        (LayoutMode::Rightbar, false, false, false, false) => RIGHTBAR_CENTER_LYRIC_CLASS,
        (LayoutMode::Rightbar, false, false, false, true) => RIGHTBAR_ACTIVE_CENTER_LYRIC_CLASS,
    }
}

fn lyric_line_active_scale(
    line: &utils::lyrics::LyricLine,
    has_opposite_turn: bool,
) -> &'static str {
    if line.background {
        "1.02"
    } else if line.opposite_turn || has_opposite_turn {
        "1.06"
    } else {
        "1.12"
    }
}

fn lyric_line_transform_origin(
    line: &utils::lyrics::LyricLine,
    has_opposite_turn: bool,
) -> &'static str {
    if line.opposite_turn {
        "right center"
    } else if has_opposite_turn {
        "left center"
    } else {
        "center"
    }
}

fn lyric_line_max_width(
    layout: LayoutMode,
    line: &utils::lyrics::LyricLine,
    has_opposite_turn: bool,
) -> &'static str {
    match (layout, line.opposite_turn || has_opposite_turn) {
        (LayoutMode::Fullscreen, true) => "min(90%, 34rem)",
        (LayoutMode::Fullscreen, false) => "min(100%, 38rem)",
        (LayoutMode::Rightbar, true) => "min(90%, 18rem)",
        (LayoutMode::Rightbar, false) => "min(100%, 20rem)",
    }
}

fn lyric_line_style(
    layout: LayoutMode,
    line: &utils::lyrics::LyricLine,
    has_opposite_turn: bool,
) -> String {
    let max_width = lyric_line_max_width(layout, line, has_opposite_turn);
    let margin_style = if line.opposite_turn {
        "margin-left: auto; margin-right: 0;"
    } else if has_opposite_turn {
        "margin-left: 0; margin-right: auto;"
    } else {
        "margin-left: auto; margin-right: auto;"
    };

    format!("{LYRIC_STYLE} width: {max_width}; max-width: {max_width}; {margin_style}")
}

fn main_line_indices(lines: &[utils::lyrics::LyricLine]) -> Vec<usize> {
    let foreground = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (!line.background).then_some(index))
        .collect::<Vec<_>>();
    if !foreground.is_empty() {
        return foreground;
    }

    (0..lines.len()).collect()
}

fn active_secondary_lines(
    lines: &[utils::lyrics::LyricLine],
    current_time: f64,
    main_line_index: usize,
) -> String {
    let entries = lines
        .iter()
        .enumerate()
        .filter(|(index, line)| {
            *index != main_line_index
                && line.background
                && line.parent_line_index == Some(main_line_index)
        })
        .map(|(index, line)| format!("[{},{}]", index, active_chunk_index(line, current_time)))
        .collect::<Vec<_>>()
        .join(",");

    format!("[{}]", entries)
}

fn active_chunk_index(line: &utils::lyrics::LyricLine, current_time: f64) -> i64 {
    line.chunks
        .partition_point(|word| word.start_time <= current_time)
        .checked_sub(1)
        .map(|index| index as i64)
        .unwrap_or(-1)
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
                let activeSecondaryEls = new Set();
                let scrollAnimationFrame;
                let activeClass = "{active_class}";
                let inactiveClass = "{inactive_class}";

                const resetWords = (lineEl) => {{
                    lineEl?.querySelectorAll('[data-lyric-chunk]').forEach((word) => {{
                        word.style.opacity = '';
                        word.style.textShadow = '';
                    }});
                }};

                const updateWords = (lineEl, activeChunkIndex) => {{
                    lineEl?.querySelectorAll('[data-lyric-chunk]').forEach((word, index) => {{
                        if (activeChunkIndex >= 0 && index <= activeChunkIndex) {{
                            word.style.opacity = '1';
                            word.style.textShadow = '0 0 12px rgba(255,255,255,0.72)';
                        }} else {{
                            word.style.opacity = '0.45';
                            word.style.textShadow = '';
                        }}
                    }});
                }};

                const inactiveFor = (lineEl) => lineEl?.dataset?.inactiveClass || inactiveClass;
                const activeFor = (lineEl) => lineEl?.dataset?.activeClass || activeClass;
                const activeScaleFor = (lineEl) => lineEl?.dataset?.activeScale || '1.06';
                const maxWidthFor = (lineEl) => lineEl?.dataset?.maxLineWidth || '100%';

                const applyLineLayout = (lineEl) => {{
                    if (!lineEl) return;
                    const origin = lineEl.dataset.transformOrigin || 'center';
                    const maxWidth = maxWidthFor(lineEl);
                    lineEl.style.boxSizing = 'border-box';
                    lineEl.style.maxWidth = maxWidth;
                    lineEl.style.width = maxWidth;
                    lineEl.style.overflowWrap = 'normal';
                    lineEl.style.wordBreak = 'normal';
                    if (origin.startsWith('right')) {{
                        lineEl.style.marginLeft = 'auto';
                        lineEl.style.marginRight = '0';
                    }} else if (origin.startsWith('left')) {{
                        lineEl.style.marginLeft = '0';
                        lineEl.style.marginRight = 'auto';
                    }} else {{
                        lineEl.style.marginLeft = 'auto';
                        lineEl.style.marginRight = 'auto';
                    }}
                }};

                const scrollLineIntoComfortView = (lineEl) => {{
                    const container = document.getElementById('{layout}-lyrics-content');
                    if (!container || !lineEl) return;

                    const containerRect = container.getBoundingClientRect();
                    const lineRect = lineEl.getBoundingClientRect();
                    const currentOffset = lineRect.top - containerRect.top;
                    const targetOffset = container.clientHeight * 0.42;
                    const nextTop = container.scrollTop + currentOffset - targetOffset;

                    if (scrollAnimationFrame) {{
                        cancelAnimationFrame(scrollAnimationFrame);
                    }}

                    const startTop = container.scrollTop;
                    const distance = nextTop - startTop;
                    const durationMs = 720;
                    const startedAt = performance.now();
                    const easeOutCubic = (t) => 1 - Math.pow(1 - t, 3);

                    const step = (now) => {{
                        const progress = Math.min(1, (now - startedAt) / durationMs);
                        container.scrollTop = startTop + distance * easeOutCubic(progress);
                        if (progress < 1) {{
                            scrollAnimationFrame = requestAnimationFrame(step);
                        }} else {{
                            scrollAnimationFrame = null;
                        }}
                    }};

                    scrollAnimationFrame = requestAnimationFrame(step);
                }};

                const fadeLineIn = (lineEl) => {{
                    if (!lineEl?.animate) return;
                    lineEl.animate(
                        [{{ opacity: 0.68 }}, {{ opacity: 1 }}],
                        {{ duration: 260, easing: 'ease-out' }}
                    );
                }};

                const deactivateLine = (lineEl) => {{
                    if (!lineEl) return;
                    lineEl.className = inactiveFor(lineEl);
                    lineEl.style.transformOrigin = lineEl.dataset.transformOrigin || 'center';
                    applyLineLayout(lineEl);
                    lineEl.style.transform = 'scale(1)';
                    resetWords(lineEl);
                }};

                const activateLine = (lineEl, chunkIndex, scale = null) => {{
                    if (!lineEl) return;
                    const scaleValue = scale || activeScaleFor(lineEl);
                    const origin = lineEl.dataset.transformOrigin || 'center';
                    lineEl.className = activeFor(lineEl);
                    lineEl.style.transformOrigin = origin;
                    applyLineLayout(lineEl);
                    lineEl.style.transform = `scale(${{scaleValue}})`;
                    if (lineEl.querySelector('[data-lyric-chunk]')) {{
                        updateWords(lineEl, chunkIndex);
                    }}
                }};

                window.__{layout}_updateLyrics = (nextIndex, nextChunkIndex, activeLinesJson = '[]') => {{
                    let nextEl = document.getElementById(`{layout}-lyrics-${{nextIndex}}`)
                    let nextSecondary = new Map(JSON.parse(activeLinesJson));
                    for (const lineEl of activeSecondaryEls) {{
                        const idx = Number(lineEl.dataset.lyricIndex);
                        if (!nextSecondary.has(idx) && lineEl !== nextEl) {{
                            deactivateLine(lineEl);
                        }}
                    }}
                    activeSecondaryEls = new Set();

                    if (currEl != nextEl) {{
                        if (currEl) {{
                            deactivateLine(currEl);
                        }}

                        if (nextEl) {{
                            activateLine(nextEl, nextChunkIndex);
                            fadeLineIn(nextEl);
                            scrollLineIntoComfortView(nextEl);
                        }}

                        currEl = nextEl;
                    }}

                    if (nextEl) {{
                        activateLine(nextEl, nextChunkIndex);
                    }}

                    for (const [idx, chunkIndex] of nextSecondary.entries()) {{
                        const lineEl = document.getElementById(`{layout}-lyrics-${{idx}}`);
                        if (!lineEl || lineEl === nextEl) continue;
                        activateLine(lineEl, chunkIndex);
                        activeSecondaryEls.add(lineEl);
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

                let main_line_indices = main_line_indices(&lines);
                let main_times = main_line_indices
                    .iter()
                    .map(|&i| lines[i].start_time)
                    .collect::<Vec<_>>();

                loop {
                    let current_time = ctrl.displayed_progress_secs_f64();
                    // Binary search to find the active line.
                    // `partition_point(|t| t <= current_time)` returns the index of the first
                    // timestamp greater than `current_time`.
                    // Therefore `n - 1` is the last timestamp less than or equal to it.
                    // If the result is 0, we are before the first line.
                    if let Some(current_line_index) =
                        match main_times.partition_point(|&t| t <= current_time) {
                            0 => None,
                            n => main_line_indices.get(n - 1).copied(),
                        }
                    {
                        let current_chunk_index =
                            active_chunk_index(&lines[current_line_index], current_time);
                        let active_secondary_lines =
                            active_secondary_lines(&lines, current_time, current_line_index);
                        let _ = eval(&format!(
                            "window.__{layout}_updateLyrics({current_line_index}, {current_chunk_index}, '{}')",
                            active_secondary_lines
                        ));

                        let active_main_position = main_line_indices
                            .iter()
                            .position(|&index| index == current_line_index)
                            .unwrap_or(0);
                        sleep_duration_ms = main_times
                            .get(active_main_position.saturating_add(1))
                            .map(|next_time| {
                                ((*next_time - current_time) * 1000.0).max(16.0).min(50.0) as u64
                            })
                            .unwrap_or(50);
                    } else {
                        // we are before the first line, invalidate current line
                        let active_secondary_lines =
                            active_secondary_lines(&lines, current_time, usize::MAX);
                        let _ = eval(&format!(
                            "window.__{layout}_updateLyrics(-1, -1, '{}')",
                            active_secondary_lines
                        ));
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
                LayoutMode::Fullscreen => "flex-1 overflow-y-auto overflow-x-hidden px-4 py-2 space-y-1",
                LayoutMode::Rightbar => "flex-1 overflow-y-auto overflow-x-hidden px-2 py-2 space-y-1",
            },

            div {
                class: match layout {
                    LayoutMode::Fullscreen => "text-white/70 text-center py-4 px-8 leading-relaxed font-medium text-lg w-full max-w-2xl mx-auto flex flex-col gap-4 overflow-x-hidden",
                    LayoutMode::Rightbar =>
                    "text-white/70 text-center py-4 px-4 leading-relaxed font-medium text-sm flex flex-col gap-4 overflow-x-hidden"
                },
                match &*lyrics.read() {
                    Some(Some(utils::lyrics::Lyrics::Synced(lines))) => {
                        let has_opposite_turn = lines.iter().any(|line| line.opposite_turn);
                        rsx! {
                            for (i, line) in lines.iter().enumerate() {
                                div {
                                    key: "{i}",
                                    id: "{layout}-lyrics-{i}",
                                    "data-lyric-line": "true",
                                    "data-lyric-index": "{i}",
                                    "data-background-line": "{line.background}",
                                    "data-max-line-width": "{lyric_line_max_width(layout, line, has_opposite_turn)}",
                                    "data-inactive-class": "{lyric_line_class(layout, line, false, has_opposite_turn)}",
                                    "data-active-class": "{lyric_line_class(layout, line, true, has_opposite_turn)}",
                                    "data-active-scale": "{lyric_line_active_scale(line, has_opposite_turn)}",
                                    "data-transform-origin": "{lyric_line_transform_origin(line, has_opposite_turn)}",
                                    class: "{lyric_line_class(layout, line, false, has_opposite_turn)}",
                                    style: lyric_line_style(layout, line, has_opposite_turn),
                                    onclick: {
                                        let st = line.start_time;
                                        move |_| {
                                            ctrl.player.write().seek(std::time::Duration::from_secs_f64(st));
                                            current_song_progress.set(st as u64);
                                        }
                                    },
                                    if line.chunks.is_empty() {
                                        "{line.text}"
                                    } else {
                                        for (chunk_i, word) in line.chunks.iter().enumerate() {
                                            span {
                                                key: "{chunk_i}",
                                                id: "{layout}-lyrics-{i}-word-{chunk_i}",
                                                "data-lyric-chunk": "true",
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
