use config::AppConfig;
use dioxus::{document::eval, prelude::*};
use hooks::PlayerController;

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
const FULLSCREEN_BACKGROUND_CENTER_LYRIC_CLASS: &str = "text-white/25 text-xl font-medium transition-colors duration-300 whitespace-pre-wrap text-center w-full leading-snug";
const FULLSCREEN_ACTIVE_BACKGROUND_CENTER_LYRIC_CLASS: &str = "text-white/70 text-xl font-medium transition-colors duration-300 whitespace-pre-wrap text-center w-full leading-snug";
const RIGHTBAR_BACKGROUND_CENTER_LYRIC_CLASS: &str = "text-white/25 text-sm font-medium transition-colors duration-300 whitespace-pre-wrap text-center w-full leading-snug";
const RIGHTBAR_ACTIVE_BACKGROUND_CENTER_LYRIC_CLASS: &str = "text-white/70 text-sm font-medium transition-colors duration-300 whitespace-pre-wrap text-center w-full leading-snug";
const FULLSCREEN_BACKGROUND_OPPOSITE_LYRIC_CLASS: &str = "text-white/25 text-xl font-medium transition-colors duration-300 whitespace-pre-wrap text-right w-full pr-6 leading-snug";
const FULLSCREEN_ACTIVE_BACKGROUND_OPPOSITE_LYRIC_CLASS: &str = "text-white/70 text-xl font-medium transition-colors duration-300 whitespace-pre-wrap text-right w-full pr-6 leading-snug";
const RIGHTBAR_BACKGROUND_OPPOSITE_LYRIC_CLASS: &str = "text-white/25 text-sm font-medium transition-colors duration-300 whitespace-pre-wrap text-right w-full pr-4 leading-snug";
const RIGHTBAR_ACTIVE_BACKGROUND_OPPOSITE_LYRIC_CLASS: &str = "text-white/70 text-sm font-medium transition-colors duration-300 whitespace-pre-wrap text-right w-full pr-4 leading-snug";
const FULLSCREEN_OPPOSITE_LYRIC_CLASS: &str = "text-white/40 text-2xl italic font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap text-right w-full";
const FULLSCREEN_ACTIVE_OPPOSITE_LYRIC_CLASS: &str = "text-white text-2xl italic font-semibold transition-colors duration-300 whitespace-pre-wrap text-right w-full";
const RIGHTBAR_OPPOSITE_LYRIC_CLASS: &str = "text-white/40 text-lg italic font-semibold transition-colors duration-300 hover:text-white/60 cursor-pointer whitespace-pre-wrap text-right w-full";
const RIGHTBAR_ACTIVE_OPPOSITE_LYRIC_CLASS: &str = "text-white text-lg italic font-semibold transition-colors duration-300 whitespace-pre-wrap text-right w-full";
const LYRIC_SEAMLESS_GAP_SECONDS: f64 = 3.0;
pub use crate::shared::LayoutMode;

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
        (LayoutMode::Fullscreen, true, false, true, false) => FULLSCREEN_BACKGROUND_LYRIC_CLASS,
        (LayoutMode::Fullscreen, true, false, true, true) => {
            FULLSCREEN_ACTIVE_BACKGROUND_LYRIC_CLASS
        }
        (LayoutMode::Rightbar, true, false, true, false) => RIGHTBAR_BACKGROUND_LYRIC_CLASS,
        (LayoutMode::Rightbar, true, false, true, true) => RIGHTBAR_ACTIVE_BACKGROUND_LYRIC_CLASS,
        (LayoutMode::Fullscreen, true, false, false, false) => {
            FULLSCREEN_BACKGROUND_CENTER_LYRIC_CLASS
        }
        (LayoutMode::Fullscreen, true, false, false, true) => {
            FULLSCREEN_ACTIVE_BACKGROUND_CENTER_LYRIC_CLASS
        }
        (LayoutMode::Rightbar, true, false, false, false) => RIGHTBAR_BACKGROUND_CENTER_LYRIC_CLASS,
        (LayoutMode::Rightbar, true, false, false, true) => {
            RIGHTBAR_ACTIVE_BACKGROUND_CENTER_LYRIC_CLASS
        }
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

fn next_main_line_start(
    lines: &[utils::lyrics::LyricLine],
    main_line_indices: &[usize],
    line_index: usize,
) -> Option<f64> {
    main_line_indices
        .iter()
        .position(|&index| index == line_index)
        .and_then(|position| main_line_indices.get(position.saturating_add(1)))
        .map(|&next_index| lines[next_index].start_time)
}

fn line_active_at(
    line: &utils::lyrics::LyricLine,
    current_time: f64,
    next_main_start: Option<f64>,
) -> bool {
    if current_time < line.start_time {
        return false;
    }

    let Some(end_time) = line.end_time else {
        return next_main_start
            .map(|next_start| current_time < next_start)
            .unwrap_or(true);
    };

    if current_time <= end_time {
        return true;
    }

    next_main_start
        .filter(|&next_start| {
            next_start > end_time && next_start - end_time <= LYRIC_SEAMLESS_GAP_SECONDS
        })
        .is_some_and(|next_start| current_time < next_start)
}

fn active_main_line_index(
    lines: &[utils::lyrics::LyricLine],
    main_line_indices: &[usize],
    current_time: f64,
) -> Option<usize> {
    main_line_indices
        .iter()
        .copied()
        .take_while(|&index| lines[index].start_time <= current_time)
        .filter(|&index| {
            line_active_at(
                &lines[index],
                current_time,
                next_main_line_start(lines, main_line_indices, index),
            )
        })
        .last()
}

fn active_secondary_lines(
    lines: &[utils::lyrics::LyricLine],
    main_line_indices: &[usize],
    current_time: f64,
    main_line_index: usize,
) -> String {
    let entries = lines
        .iter()
        .enumerate()
        .filter(|(index, line)| {
            let next_start = (!line.background)
                .then(|| next_main_line_start(lines, main_line_indices, *index))
                .flatten();
            if *index == main_line_index || !line_active_at(line, current_time, next_start) {
                return false;
            }

            (line.background && line.parent_line_index == Some(main_line_index))
                || (!line.background && main_line_index != usize::MAX)
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
    let mut auto_sync = use_signal(|| true);

    // Clear functions when the component is dropped
    use_drop(move || {
        let _cleanup = eval(&format!(
            "for (const key of ['updateLyrics', 'resetLyrics', 'setAutoSync', 'autoSync', 'programmaticScroll']) delete window[`__{layout}_${{key}}`];"
        ));
    });

    // Hand scroll control back to the user the moment they scroll the lyrics
    // themselves; the sync button re-arms auto-scroll.
    use_future(move || async move {
        let mut listener = eval(&format!(
            r#"
                const attach = () => {{
                    const container = document.getElementById('{layout}-lyrics-content');
                    if (!container) {{ requestAnimationFrame(attach); return; }}
                    container.addEventListener('scroll', () => {{
                        if (window.__{layout}_programmaticScroll) return;
                        if (window.__{layout}_autoSync === false) return;
                        window.__{layout}_autoSync = false;
                        dioxus.send('user_scroll');
                    }});
                }};
                attach();
            "#
        ));

        while let Ok(val) = listener.recv::<serde_json::Value>().await {
            if val.as_str() == Some("user_scroll") {
                auto_sync.set(false);
            }
        }
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
                window.__{layout}_autoSync = true;
                window.__{layout}_programmaticScroll = false;

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
                            word.style.textShadow = '0 0 6px rgba(255,255,255,0.35)';
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
                    if (!window.__{layout}_autoSync) return;
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

                    window.__{layout}_programmaticScroll = true;
                    const step = (now) => {{
                        const progress = Math.min(1, (now - startedAt) / durationMs);
                        container.scrollTop = startTop + distance * easeOutCubic(progress);
                        if (progress < 1) {{
                            scrollAnimationFrame = requestAnimationFrame(step);
                        }} else {{
                            scrollAnimationFrame = null;
                            setTimeout(() => {{ window.__{layout}_programmaticScroll = false; }}, 80);
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

                window.__{layout}_setAutoSync = (val) => {{
                    window.__{layout}_autoSync = val;
                    if (val && currEl) {{
                        scrollLineIntoComfortView(currEl);
                    }}
                }}

                window.__{layout}_resetLyrics = () => {{
                    if (scrollAnimationFrame) {{
                        cancelAnimationFrame(scrollAnimationFrame);
                        scrollAnimationFrame = null;
                    }}
                    document
                        .getElementById('{layout}-lyrics-content')
                        ?.querySelectorAll('[data-lyric-line]')
                        .forEach((lineEl) => deactivateLine(lineEl));
                    currEl = null;
                    activeSecondaryEls = new Set();
                }}
            "#,
        ));
    });

    use_resource(move || {
        let lyrics = lyrics.read().clone();

        // a fresh track re-arms auto-scroll
        auto_sync.set(true);

        // scroll to top on lyrics change
        let _scroll_to_top = eval(&format!(
            "if (window.__{layout}_autoSync !== undefined) window.__{layout}_autoSync = true; window.__{layout}_resetLyrics?.(); document.getElementById('{layout}-lyrics-content')?.scrollTo({{ top: 0, left: 0 }});"
        ));

        async move {
            if let Some(Some(utils::lyrics::Lyrics::Synced(lines))) = lyrics {
                let mut sleep_duration_ms: u64;

                let main_line_indices = main_line_indices(&lines);

                loop {
                    let current_time = ctrl.displayed_progress_secs_f64();
                    if let Some(current_line_index) =
                        active_main_line_index(&lines, &main_line_indices, current_time)
                    {
                        let current_chunk_index =
                            active_chunk_index(&lines[current_line_index], current_time);
                        let active_secondary_lines = active_secondary_lines(
                            &lines,
                            &main_line_indices,
                            current_time,
                            current_line_index,
                        );
                        let _ = eval(&format!(
                            "window.__{layout}_updateLyrics({current_line_index}, {current_chunk_index}, '{}')",
                            active_secondary_lines
                        ));

                        let active_main_position = main_line_indices
                            .iter()
                            .position(|&index| index == current_line_index)
                            .unwrap_or(0);
                        sleep_duration_ms = main_line_indices
                            .get(active_main_position.saturating_add(1))
                            .map(|&next_index| lines[next_index].start_time)
                            .map(|next_time| {
                                ((next_time - current_time) * 1000.0).clamp(16.0, 50.0) as u64
                            })
                            .unwrap_or(50);
                    } else {
                        // we are before the first line, invalidate current line
                        let active_secondary_lines = active_secondary_lines(
                            &lines,
                            &main_line_indices,
                            current_time,
                            usize::MAX,
                        );
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

    let show_sync_button = !auto_sync()
        && matches!(
            &*lyrics.read(),
            Some(Some(utils::lyrics::Lyrics::Synced(_)))
        );

    rsx! {
        div { class: "relative flex flex-col flex-1 min-h-0",
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
                                    key: "{i}-{line.start_time}-{line.text}",
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
                                            ctrl.seek(std::time::Duration::from_secs_f64(st));
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

        if show_sync_button {
            button {
                class: "absolute bottom-4 right-4 z-10 flex items-center justify-center w-9 h-9 rounded-full bg-black/40 hover:bg-black/60 backdrop-blur text-white/90 shadow-lg ring-1 ring-white/10 transition-colors",
                onclick: move |_| {
                    auto_sync.set(true);
                    let _ = eval(&format!("window.__{layout}_setAutoSync?.(true)"));
                },
                svg {
                    class: "w-5 h-5",
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "2",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    path { d: "M21 12a9 9 0 1 1-2.64-6.36" }
                    polyline { points: "21 3 21 9 15 9" }
                }
            }
        }
        }
    }
}
