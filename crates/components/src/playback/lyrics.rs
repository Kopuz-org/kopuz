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
const LYRIC_STYLE: &str = "box-sizing: border-box; overflow-wrap: normal; word-break: normal; transform: scale(1); filter: blur(0px); transition: color 300ms, transform 300ms, filter 300ms, opacity 180ms, max-height 180ms, margin-top 180ms;";
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
const LYRIC_COMFORT_OFFSET_PERCENT: u32 = 42;
const LYRIC_TAIL_SPACER_PERCENT: u32 = 100 - LYRIC_COMFORT_OFFSET_PERCENT;
const LYRIC_SEAMLESS_GAP_SECONDS: f64 = 3.0;
const LYRIC_CHUNK_FALLBACK_SECONDS: f64 = 0.35;
/// A silence shorter than this is a breath between lines, not an interlude.
const LYRIC_INTERLUDE_MIN_SECONDS: f64 = 5.0;
/// Only paxsenix and Apple Music timestamp a line's end, so the rest need a
/// guess. A sung line rarely runs longer than this.
const LYRIC_LINE_ASSUMED_MAX_SECONDS: f64 = 7.0;
const INTERLUDE_LYRIC_CLASS: &str = "flex w-full items-center py-2 opacity-40 hover:opacity-80 cursor-pointer transition-opacity duration-300";
const INTERLUDE_ACTIVE_LYRIC_CLASS: &str =
    "flex w-full items-center py-2 opacity-100 cursor-pointer transition-opacity duration-300";
// Depth-of-field blur, keyed by layout since the rightbar's smaller type
// turns mushy at the fullscreen step. Roughly a third of the font size at
// full clamp keeps the farthest lines legible instead of a smear.
const FULLSCREEN_DEPTH_BLUR_STEP_PX: f64 = 1.5;
const FULLSCREEN_DEPTH_BLUR_MAX_PX: f64 = 8.0;
const RIGHTBAR_DEPTH_BLUR_STEP_PX: f64 = 1.1;
const RIGHTBAR_DEPTH_BLUR_MAX_PX: f64 = 6.0;
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

/// Per-line-of-distance blur step and the clamp, in px, for a layout's font size.
fn lyric_depth_blur_ramp(layout: LayoutMode) -> (f64, f64) {
    match layout {
        LayoutMode::Fullscreen => (FULLSCREEN_DEPTH_BLUR_STEP_PX, FULLSCREEN_DEPTH_BLUR_MAX_PX),
        LayoutMode::Rightbar => (RIGHTBAR_DEPTH_BLUR_STEP_PX, RIGHTBAR_DEPTH_BLUR_MAX_PX),
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

/// A background line carries its own timing and often outlasts the line it
/// was attached to, or overlaps the next one (Apple starts the next main row
/// while the backing vocal is still going). Judge it on that timing alone,
/// not on which main line is current. Without an end time it runs until the
/// next main line starts after it.
fn background_line_bound(
    lines: &[utils::lyrics::LyricLine],
    main_line_indices: &[usize],
    line: &utils::lyrics::LyricLine,
) -> Option<f64> {
    if line.end_time.is_some() {
        return None;
    }
    main_line_indices
        .iter()
        .map(|&index| lines[index].start_time)
        .find(|&start| start > line.start_time)
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
            let next_start = if line.background {
                background_line_bound(lines, main_line_indices, line)
            } else {
                next_main_line_start(lines, main_line_indices, *index)
            };
            if *index == main_line_index || !line_active_at(line, current_time, next_start) {
                return false;
            }

            line.background || main_line_index != usize::MAX
        })
        .map(|(index, _)| index.to_string())
        .collect::<Vec<_>>()
        .join(",");

    format!("[{}]", entries)
}

/// Providers only timestamp the start of a chunk, so a chunk runs until the
/// next one starts and the last one until the line ends. The wipe needs a span
/// to interpolate over, hence the fallback when neither is available.
fn chunk_end_time(line: &utils::lyrics::LyricLine, index: usize) -> f64 {
    let start = line.chunks[index].start_time;
    line.chunks
        .get(index.saturating_add(1))
        .map(|next| next.start_time)
        .or(line.end_time)
        .filter(|&end| end > start)
        .unwrap_or(start + LYRIC_CHUNK_FALLBACK_SECONDS)
}

fn interlude_line_class(has_opposite_turn: bool, active: bool) -> String {
    let base = if active {
        INTERLUDE_ACTIVE_LYRIC_CLASS
    } else {
        INTERLUDE_LYRIC_CLASS
    };
    let justify = if has_opposite_turn {
        "justify-start"
    } else {
        "justify-center"
    };

    format!("{base} {justify}")
}

fn line_end_estimate(line: &utils::lyrics::LyricLine) -> f64 {
    line.end_time
        .or_else(|| {
            line.chunks
                .last()
                .map(|chunk| chunk.start_time + LYRIC_CHUNK_FALLBACK_SECONDS)
        })
        .unwrap_or(line.start_time + LYRIC_LINE_ASSUMED_MAX_SECONDS)
}

/// Providers emit nothing for an instrumental stretch, so the view sits blank
/// through it. Synthesize a line for every long gap; it is a plain foreground
/// line so the existing activation, scroll and seek paths handle it unchanged.
/// The returned flags mark which entries are synthesized.
fn build_display_lines(
    lines: &[utils::lyrics::LyricLine],
) -> (Vec<utils::lyrics::LyricLine>, Vec<bool>) {
    let main = main_line_indices(lines);
    let mut gaps: Vec<(usize, f64, f64)> = Vec::new();

    if let Some(&first) = main.first()
        && lines[first].start_time >= LYRIC_INTERLUDE_MIN_SECONDS
    {
        gaps.push((first, 0.0, lines[first].start_time));
    }

    for pair in main.windows(2) {
        let (current, next) = (pair[0], pair[1]);
        let next_start = lines[next].start_time;
        // Background lines sit after their parent in the list and can outlast
        // it, so the gap starts once every line in the run has finished.
        let gap_start = lines[current..next]
            .iter()
            .map(line_end_estimate)
            .fold(f64::NEG_INFINITY, f64::max)
            .clamp(lines[current].start_time, next_start);
        if next_start - gap_start >= LYRIC_INTERLUDE_MIN_SECONDS {
            gaps.push((next, gap_start, next_start));
        }
    }

    if gaps.is_empty() {
        return (lines.to_vec(), vec![false; lines.len()]);
    }

    let mut display = Vec::with_capacity(lines.len() + gaps.len());
    let mut interludes = Vec::with_capacity(lines.len() + gaps.len());
    let mut remap = vec![0usize; lines.len()];
    let mut gaps = gaps.into_iter().peekable();

    for (index, line) in lines.iter().enumerate() {
        while let Some(&(at, start, end)) = gaps.peek() {
            if at != index {
                break;
            }
            gaps.next();
            display.push(utils::lyrics::LyricLine {
                start_time: start,
                end_time: Some(end),
                text: String::new(),
                chunks: Vec::new(),
                parent_line_index: None,
                background: false,
                opposite_turn: false,
            });
            interludes.push(true);
        }
        remap[index] = display.len();
        display.push(line.clone());
        interludes.push(false);
    }

    for line in &mut display {
        if let Some(parent) = line.parent_line_index {
            line.parent_line_index = remap.get(parent).copied();
        }
    }

    (display, interludes)
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
            "for (const [key, type] of [['lineOver', 'mouseover'], ['lineOut', 'mouseout']]) {{ const fn = window[`__{layout}_${{key}}`]; if (fn) document.removeEventListener(type, fn); }} for (const key of ['updateLyrics', 'resetLyrics', 'setAutoSync', 'autoSync', 'lineOver', 'lineOut']) delete window[`__{layout}_${{key}}`];"
        ));
    });

    // Take over on real input, not on scroll events: line growth and the browser's
    // own scroll anchoring move scrollTop on their own. The sync button re-arms.
    use_future(move || async move {
        let mut listener = eval(&format!(
            r#"
                const attach = () => {{
                    const container = document.getElementById('{layout}-lyrics-content');
                    if (!container) {{ requestAnimationFrame(attach); return; }}
                    const scrollKeys = new Set(
                        ['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End']
                    );
                    const takeOver = () => {{
                        if (window.__{layout}_autoSync === false) return;
                        window.__{layout}_autoSync = false;
                        dioxus.send('user_scroll');
                    }};
                    container.addEventListener('wheel', takeOver, {{ passive: true }});
                    container.addEventListener('touchmove', takeOver, {{ passive: true }});
                    container.addEventListener('keydown', (e) => {{
                        if (scrollKeys.has(e.key)) takeOver();
                    }});
                    // Scrollbar gutter only; a press on a line is a seek.
                    container.addEventListener('pointerdown', (e) => {{
                        if (e.target === container && e.offsetX >= container.clientWidth) {{
                            takeOver();
                        }}
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
        let (depth_blur_step_px, depth_blur_max_px) = lyric_depth_blur_ramp(layout);

        let _update_func = eval(&format!(
            r#"
                let currEl;
                let activeSecondaryEls = new Set();
                let scrollAnimationFrame;
                let activeClass = "{active_class}";
                let inactiveClass = "{inactive_class}";
                window.__{layout}_autoSync = true;

                // Depth-of-field state: only re-swept when the set of lit lines or
                // the setting itself changes, not on every clock tick.
                let lastBlurLit = null;
                let lastBlurEnabled = null;
                let lastBlurStrength = null;
                // An intro before the first line, and any gap between rows, report
                // no active line. Holding the last anchor keeps the ramp in place
                // instead of flattening the whole list until the next line lands.
                let lastAnchorIndex = 0;
                let hoveredLine = null;
                const BLUR_STEP_PX = {depth_blur_step_px};
                const BLUR_MAX_PX = {depth_blur_max_px};

                const UNSUNG_ALPHA = 0.45;
                const GLOW_DECAY_SECONDS = 0.6;
                // A chunk runs until the next one starts, which over a pause or a line
                // tail is far longer than the syllable itself. Cap the wipe so it lands
                // on the beat and holds instead of creeping through the silence.
                const MAX_WIPE_SECONDS = 1.2;
                const reduceMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches === true;

                // Playback time only arrives every ~16-50ms; extrapolate between
                // updates so the wipe runs at frame rate, capped so a stalled feed
                // can't run away.
                const clock = {{ time: 0, at: 0, playing: false }};
                const MAX_EXTRAPOLATION_SECONDS = 0.1;
                const nowSeconds = () => clock.playing
                    ? clock.time + Math.min((performance.now() - clock.at) / 1000, MAX_EXTRAPOLATION_SECONDS)
                    : clock.time;

                const chunkAlpha = (lineEl) => lineEl.dataset.backgroundLine === 'true' ? 0.7 : 1;

                // The gradient is 2.2 chunk-widths with a soft band in the middle, so
                // sliding it from 99% to 1% wipes the fill across the glyphs and still
                // parks the band clear of both edges without leaving the chunk box
                // (a position outside 0-100% would expose an unpainted sliver).
                const primeChunks = (lineEl, chunks) => {{
                    if (lineEl.dataset.lyricPrimedFor === lineEl.className) return;
                    lineEl.dataset.lyricPrimedFor = lineEl.className;
                    const alpha = chunkAlpha(lineEl);
                    const sung = `rgba(255,255,255,${{alpha}})`;
                    const unsung = `rgba(255,255,255,${{alpha * UNSUNG_ALPHA}})`;
                    const image = `linear-gradient(to right, ${{sung}} 0%, ${{sung}} 46%, ${{unsung}} 54%, ${{unsung}} 100%)`;
                    for (const chunk of chunks) {{
                        chunk.style.backgroundImage = image;
                        chunk.style.backgroundSize = '220% 100%';
                        chunk.style.backgroundRepeat = 'no-repeat';
                        chunk.style.webkitBackgroundClip = 'text';
                        chunk.style.backgroundClip = 'text';
                        chunk.style.color = 'transparent';
                        chunk.style.webkitTextFillColor = 'transparent';
                    }}
                }};

                // An instrumental stretch has no words to wipe, so the note itself
                // fills left to right to show how much of the gap is left.
                const paintInterlude = (lineEl, time) => {{
                    const fillEl = lineEl.querySelector('[data-interlude-fill]');
                    if (!fillEl) return false;
                    const start = Number(lineEl.dataset.interludeStart);
                    const end = Number(lineEl.dataset.interludeEnd);
                    const span = end - start;
                    let progress = span > 0 ? (time - start) / span : (time >= start ? 1 : 0);
                    progress = Math.min(1, Math.max(0, progress));
                    if (reduceMotion) progress = time >= start ? 1 : 0;

                    const nextFill = Math.round(progress * 200) / 200;
                    if (lineEl.__interludeFill !== nextFill) {{
                        lineEl.__interludeFill = nextFill;
                        fillEl.style.clipPath = `inset(0 ${{(100 - nextFill * 100).toFixed(2)}}% 0 0)`;
                    }}

                    return true;
                }};

                const paintChunks = (lineEl, time) => {{
                    if (!lineEl?.isConnected) return false;
                    if (lineEl.dataset.lyricInterlude === 'true') return paintInterlude(lineEl, time);
                    const chunks = lineEl.querySelectorAll('[data-lyric-chunk]');
                    if (!chunks.length) return false;
                    primeChunks(lineEl, chunks);
                    const alpha = chunkAlpha(lineEl);

                    for (const chunk of chunks) {{
                        const start = Number(chunk.dataset.chunkStart);
                        const end = Number(chunk.dataset.chunkEnd);
                        const span = Math.min(end - start, MAX_WIPE_SECONDS);
                        let fill = span > 0 ? (time - start) / span : (time >= start ? 1 : 0);
                        fill = Math.min(1, Math.max(0, fill));
                        if (reduceMotion) fill = time >= start ? 1 : 0;

                        const nextFill = Math.round(fill * 200) / 200;
                        if (chunk.__lyricFill !== nextFill) {{
                            chunk.__lyricFill = nextFill;
                            chunk.style.backgroundPositionX = `${{(99 - nextFill * 98).toFixed(2)}}%`;
                        }}

                        let glow = 0;
                        if (!reduceMotion) {{
                            // Lit while the chunk is the one being sung, then settles.
                            glow = time < start
                                ? 0
                                : (time <= end ? 1 : 1 - (time - end) / GLOW_DECAY_SECONDS);
                            glow = Math.min(1, Math.max(0, glow));
                        }}

                        const nextGlow = Math.round(glow * 20) / 20;
                        if (chunk.__lyricGlow !== nextGlow) {{
                            chunk.__lyricGlow = nextGlow;
                            chunk.style.textShadow = nextGlow > 0
                                ? `0 0 ${{(4 + nextGlow * 6).toFixed(1)}}px rgba(255,255,255,${{(nextGlow * 0.3 * alpha).toFixed(3)}})`
                                : '';
                        }}
                    }}

                    return true;
                }};

                let paintFrame = null;
                const paintTick = () => {{
                    paintFrame = null;
                    const time = nowSeconds();
                    let painted = currEl ? paintChunks(currEl, time) : false;
                    for (const lineEl of activeSecondaryEls) {{
                        painted = paintChunks(lineEl, time) || painted;
                    }}
                    if (painted) {{
                        paintFrame = requestAnimationFrame(paintTick);
                    }}
                }};

                const schedulePaint = () => {{
                    if (paintFrame === null) {{
                        paintFrame = requestAnimationFrame(paintTick);
                    }}
                }};

                const resetWords = (lineEl) => {{
                    if (!lineEl) return;
                    delete lineEl.dataset.lyricPrimedFor;
                    lineEl.querySelectorAll('[data-lyric-chunk]').forEach((chunk) => {{
                        chunk.style.backgroundImage = '';
                        chunk.style.backgroundSize = '';
                        chunk.style.backgroundRepeat = '';
                        chunk.style.backgroundPositionX = '';
                        chunk.style.webkitBackgroundClip = '';
                        chunk.style.backgroundClip = '';
                        chunk.style.color = '';
                        chunk.style.webkitTextFillColor = '';
                        chunk.style.textShadow = '';
                        chunk.__lyricFill = undefined;
                        chunk.__lyricGlow = undefined;
                    }});
                    const interludeFill = lineEl.querySelector('[data-interlude-fill]');
                    if (interludeFill) {{
                        interludeFill.style.clipPath = 'inset(0 100% 0 0)';
                        lineEl.__interludeFill = undefined;
                    }}
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

                const comfortScrollTop = (container, lineEl) => {{
                    const currentOffset = lineEl.getBoundingClientRect().top
                        - container.getBoundingClientRect().top;
                    const targetOffset = container.clientHeight * {LYRIC_COMFORT_OFFSET_PERCENT} / 100;
                    const furthest = Math.max(0, container.scrollHeight - container.clientHeight);
                    const top = container.scrollTop + currentOffset - targetOffset;
                    return Math.min(furthest, Math.max(0, top));
                }};

                const scrollLineIntoComfortView = (lineEl) => {{
                    if (!window.__{layout}_autoSync) return;
                    const container = document.getElementById('{layout}-lyrics-content');
                    if (!container || !lineEl) return;

                    const nextTop = comfortScrollTop(container, lineEl);

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

                // A remount measures the list before layout settles and a resize moves
                // it under us; re-park rather than wait for the next line.
                const realignIfDrifted = (lineEl) => {{
                    if (!window.__{layout}_autoSync || scrollAnimationFrame) return;
                    const container = document.getElementById('{layout}-lyrics-content');
                    if (!container || !lineEl) return;
                    if (Math.abs(comfortScrollTop(container, lineEl) - container.scrollTop) > 24) {{
                        scrollLineIntoComfortView(lineEl);
                    }}
                }};

                const fadeLineIn = (lineEl) => {{
                    if (!lineEl?.animate) return;
                    lineEl.animate(
                        [{{ opacity: 0.68 }}, {{ opacity: 1 }}],
                        {{ duration: 260, easing: 'ease-out' }}
                    );
                }};

                // Apple Music style depth-of-field: every rendered line blurs a
                // little more per line of distance (data-lyric-index, not seconds)
                // from the active one, clamped so far lines stay legible.
                const depthBlurPx = (distance, scale) =>
                    Math.min(distance * BLUR_STEP_PX * scale, BLUR_MAX_PX * scale);

                // A filter hands the line its own compositing layer and backing
                // store. Half pixels land on a device pixel at 2x and a blur under
                // one is not visible anyway, so quantise and let the __lyricBlur
                // guard drop the write once a line has settled on the clamp.
                // macOS 27 betas paint unpainted backing store as magenta
                // (WebKit 303157), so the write count is worth keeping down.
                const BLUR_QUANTUM_PX = 0.5;

                // Lit lines (the active one plus any background or overlapping
                // line) stay sharp. Everything else rides distance alone, which
                // depthBlurPx clamps, so a long list converges on one value at the
                // far end instead of hitting a cutoff and snapping back to sharp
                // partway down.
                const applyDepthBlur = (mainIndex, litIndices, enabled, strengthPercent) => {{
                    const anchorIndex = mainIndex >= 0
                        ? mainIndex
                        : (litIndices.size ? Math.max(...litIndices) : lastAnchorIndex);
                    lastAnchorIndex = anchorIndex;
                    const litKey = `${{anchorIndex}}:${{[...litIndices].sort((a, b) => a - b).join(',')}}`;
                    if (litKey === lastBlurLit
                        && enabled === lastBlurEnabled
                        && strengthPercent === lastBlurStrength) return;
                    lastBlurLit = litKey;
                    lastBlurEnabled = enabled;
                    lastBlurStrength = strengthPercent;
                    const scale = strengthPercent / 100;
                    const container = document.getElementById('{layout}-lyrics-content');
                    if (!container) return;
                    container.querySelectorAll('[data-lyric-line]').forEach((lineEl) => {{
                        const index = Number(lineEl.dataset.lyricIndex);
                        const distance = enabled && !litIndices.has(index)
                            ? Math.abs(index - anchorIndex)
                            : 0;
                        const rawBlurPx = distance > 0 ? depthBlurPx(distance, scale) : 0;
                        const blurPx = Math.round(rawBlurPx / BLUR_QUANTUM_PX) * BLUR_QUANTUM_PX;
                        // Always an explicit length, never ''. Clearing the
                        // declaration drops the line back to the computed `none`,
                        // and `transition: filter` has to interpolate a blur list
                        // against a keyword; WebKit does that badly and the whole
                        // ramp reads as unblurred for the length of the switch.
                        const nextFilter = `blur(${{blurPx.toFixed(2)}}px)`;
                        if (lineEl.__lyricBlur === nextFilter) return;
                        lineEl.__lyricBlur = nextFilter;
                        if (lineEl !== hoveredLine) lineEl.style.filter = nextFilter;
                    }});
                }};

                // Hover lifts a line clear of the depth of field so it can be read
                // before it is clicked, and the ramp's own value comes back on
                // leave. The sweep keeps writing __lyricBlur underneath, so a line
                // whose distance moved under the cursor leaves on the new value.
                const setHovered = (lineEl) => {{
                    if (hoveredLine === lineEl) return;
                    if (hoveredLine) {{
                        hoveredLine.style.filter = hoveredLine.__lyricBlur || 'blur(0px)';
                    }}
                    hoveredLine = lineEl;
                    if (lineEl) lineEl.style.filter = 'blur(0px)';
                }};

                const onLineOver = (event) => {{
                    const lineEl = event.target?.closest?.('[data-lyric-line]');
                    if (!lineEl) {{
                        setHovered(null);
                        return;
                    }}
                    const container = document.getElementById('{layout}-lyrics-content');
                    setHovered(container?.contains(lineEl) ? lineEl : null);
                }};

                const onLineOut = (event) => {{
                    if (!hoveredLine) return;
                    const to = event.relatedTarget;
                    if (to && hoveredLine.contains(to)) return;
                    setHovered(null);
                }};

                for (const [key, type] of [['lineOver', 'mouseover'], ['lineOut', 'mouseout']]) {{
                    const prev = window[`__{layout}_${{key}}`];
                    if (prev) document.removeEventListener(type, prev);
                }}
                window.__{layout}_lineOver = onLineOver;
                window.__{layout}_lineOut = onLineOut;
                document.addEventListener('mouseover', onLineOver);
                document.addEventListener('mouseout', onLineOut);

                const deactivateLine = (lineEl) => {{
                    if (!lineEl) return;
                    lineEl.className = inactiveFor(lineEl);
                    lineEl.style.transformOrigin = lineEl.dataset.transformOrigin || 'center';
                    applyLineLayout(lineEl);
                    lineEl.style.transform = 'scale(1)';
                    resetWords(lineEl);
                }};

                const activateLine = (lineEl, scale = null) => {{
                    if (!lineEl) return;
                    const scaleValue = scale || activeScaleFor(lineEl);
                    const origin = lineEl.dataset.transformOrigin || 'center';
                    lineEl.className = activeFor(lineEl);
                    lineEl.style.transformOrigin = origin;
                    applyLineLayout(lineEl);
                    lineEl.style.transform = `scale(${{scaleValue}})`;
                    paintChunks(lineEl, nowSeconds());
                }};

                window.__{layout}_updateLyrics = (nextIndex, currentTime, playing, activeLinesJson = '[]', depthBlurEnabled = true, depthBlurStrength = 100) => {{
                    clock.time = currentTime;
                    clock.at = performance.now();
                    clock.playing = playing;

                    let nextEl = document.getElementById(`{layout}-lyrics-${{nextIndex}}`)
                    let nextSecondary = new Set(JSON.parse(activeLinesJson));
                    const lit = new Set(nextSecondary);
                    if (nextIndex >= 0) lit.add(nextIndex);
                    applyDepthBlur(nextIndex, lit, depthBlurEnabled, depthBlurStrength);
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
                            activateLine(nextEl);
                            fadeLineIn(nextEl);
                            scrollLineIntoComfortView(nextEl);
                        }}

                        currEl = nextEl;
                    }}

                    if (nextEl) {{
                        activateLine(nextEl);
                        realignIfDrifted(nextEl);
                    }}

                    for (const idx of nextSecondary) {{
                        const lineEl = document.getElementById(`{layout}-lyrics-${{idx}}`);
                        if (!lineEl || lineEl === nextEl) continue;
                        activateLine(lineEl);
                        activeSecondaryEls.add(lineEl);
                    }}

                    schedulePaint();
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
                    if (paintFrame !== null) {{
                        cancelAnimationFrame(paintFrame);
                        paintFrame = null;
                    }}
                    const container = document.getElementById('{layout}-lyrics-content');
                    container
                        ?.querySelectorAll('[data-lyric-line]')
                        .forEach((lineEl) => deactivateLine(lineEl));
                    currEl = null;
                    activeSecondaryEls = new Set();
                    lastBlurLit = null;
                    lastBlurEnabled = null;
                    lastBlurStrength = null;
                    lastAnchorIndex = 0;
                    hoveredLine = null;
                    container?.scrollTo({{ top: 0, left: 0 }});
                }}
            "#,
        ));
    });

    use_resource(move || {
        let lyrics = lyrics.read().clone();

        // a fresh track re-arms auto-scroll
        auto_sync.set(true);

        let _reset = eval(&format!(
            "if (window.__{layout}_autoSync !== undefined) window.__{layout}_autoSync = true; window.__{layout}_resetLyrics?.();"
        ));

        async move {
            if let Some(Some(utils::lyrics::Lyrics::Synced(lines))) = lyrics {
                let mut sleep_duration_ms: u64;

                let (lines, _) = build_display_lines(&lines);
                let main_line_indices = main_line_indices(&lines);

                loop {
                    // The clock runs ahead of the speakers; hold the lyrics back.
                    let (offset_secs, depth_blur_enabled, depth_blur_strength) = {
                        let cfg = config.peek();
                        let offset_secs = if cfg.lyrics_offset_auto {
                            ctrl.output_latency_secs()
                        } else {
                            f64::from(cfg.lyrics_offset_ms) / 1000.0
                        };
                        (
                            offset_secs,
                            cfg.lyrics_depth_blur,
                            cfg.lyrics_depth_blur_strength,
                        )
                    };
                    let current_time = ctrl.displayed_progress_secs_f64() - offset_secs;
                    let playing = *ctrl.is_playing.peek();
                    if let Some(current_line_index) =
                        active_main_line_index(&lines, &main_line_indices, current_time)
                    {
                        let active_secondary_lines = active_secondary_lines(
                            &lines,
                            &main_line_indices,
                            current_time,
                            current_line_index,
                        );
                        let _ = eval(&format!(
                            "window.__{layout}_updateLyrics({current_line_index}, {current_time}, {playing}, '{}', {depth_blur_enabled}, {depth_blur_strength})",
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
                            "window.__{layout}_updateLyrics(-1, {current_time}, {playing}, '{}', {depth_blur_enabled}, {depth_blur_strength})",
                            active_secondary_lines
                        ));
                        sleep_duration_ms = 50;
                    }

                    utils::sleep(std::time::Duration::from_millis(sleep_duration_ms)).await;
                }
            }
        }
    });

    let has_synced_lyrics = matches!(
        &*lyrics.read(),
        Some(Some(utils::lyrics::Lyrics::Synced(_)))
    );
    let show_sync_button = !auto_sync() && has_synced_lyrics;

    rsx! {
        div { class: "relative flex flex-col flex-1 min-h-0",
        div {
            id: "{layout}-lyrics-content",
            tabindex: "0",
            class: match layout {
                LayoutMode::Fullscreen => "flex-1 overflow-y-auto overflow-x-hidden px-4 py-2 space-y-1",
                LayoutMode::Rightbar => "flex-1 overflow-y-auto overflow-x-hidden px-2 py-2 space-y-1",
            },

            if has_synced_lyrics {
                div { "aria-hidden": "true", style: "height: {LYRIC_COMFORT_OFFSET_PERCENT}%" }
            }

            div {
                class: match layout {
                    LayoutMode::Fullscreen => "text-white/70 text-center py-4 px-8 leading-relaxed font-medium text-lg w-full max-w-2xl mx-auto flex flex-col gap-4 overflow-x-hidden",
                    LayoutMode::Rightbar =>
                    "text-white/70 text-center py-4 px-4 leading-relaxed font-medium text-sm flex flex-col gap-4 overflow-x-hidden"
                },
                match &*lyrics.read() {
                    Some(Some(utils::lyrics::Lyrics::Synced(lines))) => {
                        let (lines, interludes) = build_display_lines(lines);
                        let has_opposite_turn = lines.iter().any(|line| line.opposite_turn);
                        let note_class = match layout {
                            LayoutMode::Fullscreen => "w-7 h-7",
                            LayoutMode::Rightbar => "w-5 h-5",
                        };
                        rsx! {
                            for (i, line) in lines.iter().enumerate() {
                                if interludes[i] {
                                    div {
                                        key: "{i}-interlude-{line.start_time}",
                                        id: "{layout}-lyrics-{i}",
                                        "data-lyric-line": "true",
                                        "data-lyric-index": "{i}",
                                        "data-lyric-interlude": "true",
                                        "data-interlude-start": "{line.start_time}",
                                        "data-interlude-end": "{line.end_time.unwrap_or(line.start_time)}",
                                        "data-background-line": "false",
                                        "data-max-line-width": "{lyric_line_max_width(layout, line, has_opposite_turn)}",
                                        "data-inactive-class": "{interlude_line_class(has_opposite_turn, false)}",
                                        "data-active-class": "{interlude_line_class(has_opposite_turn, true)}",
                                        "data-active-scale": "1.06",
                                        "data-transform-origin": "{lyric_line_transform_origin(line, has_opposite_turn)}",
                                        "aria-label": "{i18n::t(\"instrumental_break\")}",
                                        class: "{interlude_line_class(has_opposite_turn, false)}",
                                        style: lyric_line_style(layout, line, has_opposite_turn),
                                        onclick: {
                                            let st = line.start_time;
                                            move |_| {
                                                ctrl.seek(std::time::Duration::from_secs_f64(st));
                                            }
                                        },
                                        span { class: "relative inline-flex text-white",
                                            svg {
                                                class: "{note_class}",
                                                "aria-hidden": "true",
                                                view_box: "0 0 24 24",
                                                fill: "none",
                                                stroke: "currentColor",
                                                stroke_width: "2",
                                                stroke_linecap: "round",
                                                stroke_linejoin: "round",
                                                style: "opacity: 0.35;",
                                                path { d: "M9 18V5l12-2v13" }
                                                circle { cx: "6", cy: "18", r: "3" }
                                                circle { cx: "18", cy: "16", r: "3" }
                                            }
                                            svg {
                                                class: "{note_class} absolute left-0 top-0",
                                                "aria-hidden": "true",
                                                "data-interlude-fill": "true",
                                                view_box: "0 0 24 24",
                                                fill: "none",
                                                stroke: "currentColor",
                                                stroke_width: "2",
                                                stroke_linecap: "round",
                                                stroke_linejoin: "round",
                                                style: "clip-path: inset(0 100% 0 0);",
                                                path { d: "M9 18V5l12-2v13" }
                                                circle { cx: "6", cy: "18", r: "3" }
                                                circle { cx: "18", cy: "16", r: "3" }
                                            }
                                        }
                                    }
                                } else {
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
                                                    "data-chunk-start": "{word.start_time}",
                                                    "data-chunk-end": "{chunk_end_time(line, chunk_i)}",
                                                    "{word.text}"
                                                }
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

            if has_synced_lyrics {
                div { "aria-hidden": "true", style: "height: {LYRIC_TAIL_SPACER_PERCENT}%" }
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

#[cfg(test)]
mod tests {
    use super::*;
    use utils::lyrics::LyricLine;

    fn line(start_time: f64, end_time: Option<f64>) -> LyricLine {
        LyricLine {
            start_time,
            end_time,
            text: "la".into(),
            chunks: Vec::new(),
            parent_line_index: None,
            background: false,
            opposite_turn: false,
        }
    }

    #[test]
    fn marks_intro_and_instrumental_gaps() {
        let lines = vec![
            line(12.0, Some(15.0)),
            line(40.0, Some(43.0)),
            line(45.0, Some(48.0)),
        ];

        let (display, interludes) = build_display_lines(&lines);

        assert_eq!(interludes, vec![true, false, true, false, false]);
        assert_eq!(display[0].start_time, 0.0);
        assert_eq!(display[0].end_time, Some(12.0));
        assert_eq!(display[2].start_time, 15.0);
        assert_eq!(display[2].end_time, Some(40.0));
    }

    #[test]
    fn gap_starts_after_a_background_line_outlasts_its_parent() {
        let mut background = line(3.0, Some(9.0));
        background.background = true;
        background.parent_line_index = Some(0);
        let lines = vec![line(1.0, Some(4.0)), background, line(30.0, Some(33.0))];

        let (display, interludes) = build_display_lines(&lines);

        assert_eq!(interludes, vec![false, false, true, false]);
        assert_eq!(display[2].start_time, 9.0);
        assert_eq!(display[3].parent_line_index, None);
        assert_eq!(display[1].parent_line_index, Some(0));
    }

    fn background_line(start_time: f64, end_time: Option<f64>, parent: usize) -> LyricLine {
        let mut line = line(start_time, end_time);
        line.background = true;
        line.parent_line_index = Some(parent);
        line
    }

    #[test]
    fn background_line_stays_lit_when_the_next_main_line_starts() {
        // Apple's rows for The Chain: the next main line begins while the
        // backing vocal of the previous one is still running.
        let lines = vec![
            line(63.167, Some(67.299)),
            background_line(65.48, Some(67.299), 0),
            line(66.236, Some(70.567)),
        ];
        let main = main_line_indices(&lines);

        assert_eq!(active_main_line_index(&lines, &main, 66.5), Some(2));
        assert_eq!(active_secondary_lines(&lines, &main, 66.5, 2), "[0,1]");
        assert_eq!(active_secondary_lines(&lines, &main, 67.5, 2), "[]");
    }

    #[test]
    fn background_line_stays_lit_after_its_parent_ends_with_no_main_line_active() {
        let lines = vec![
            line(1.0, Some(2.0)),
            background_line(1.5, Some(5.0), 0),
            line(10.0, Some(12.0)),
        ];
        let main = main_line_indices(&lines);

        assert_eq!(active_main_line_index(&lines, &main, 3.0), None);
        assert_eq!(
            active_secondary_lines(&lines, &main, 3.0, usize::MAX),
            "[1]"
        );
        assert_eq!(active_secondary_lines(&lines, &main, 5.5, usize::MAX), "[]");
    }

    #[test]
    fn untimed_background_line_runs_until_the_next_main_line() {
        let lines = vec![
            line(1.0, Some(2.0)),
            background_line(1.5, None, 0),
            line(4.0, Some(6.0)),
        ];
        let main = main_line_indices(&lines);

        assert_eq!(
            active_secondary_lines(&lines, &main, 3.0, usize::MAX),
            "[1]"
        );
        assert_eq!(active_secondary_lines(&lines, &main, 4.5, 2), "[]");
    }

    #[test]
    fn leaves_lyrics_untouched_without_a_long_gap() {
        let lines = vec![line(1.0, Some(4.0)), line(5.0, Some(8.0))];

        let (display, interludes) = build_display_lines(&lines);

        assert_eq!(display, lines);
        assert_eq!(interludes, vec![false, false]);
    }

    #[test]
    fn depth_blur_ramp_scales_down_for_the_smaller_rightbar_type() {
        let (fullscreen_step, fullscreen_max) = lyric_depth_blur_ramp(LayoutMode::Fullscreen);
        let (rightbar_step, rightbar_max) = lyric_depth_blur_ramp(LayoutMode::Rightbar);

        assert!(rightbar_step < fullscreen_step);
        assert!(rightbar_max < fullscreen_max);
        // At least a couple of lines of headroom before the clamp kicks in.
        assert!(fullscreen_max > fullscreen_step * 2.0);
        assert!(rightbar_max > rightbar_step * 2.0);
    }

    #[test]
    fn untimed_lines_fall_back_to_an_assumed_tail() {
        let lines = vec![line(0.0, None), line(60.0, None)];

        let (display, interludes) = build_display_lines(&lines);

        assert_eq!(interludes, vec![false, true, false]);
        assert_eq!(display[1].start_time, LYRIC_LINE_ASSUMED_MAX_SECONDS);
        assert_eq!(display[1].end_time, Some(60.0));
    }
}
