use crate::shared::fmt_time;
use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_player_controller::{BufferedRange, LoopMode, PlayerController};

pub struct SeekDrag {
    pub display_progress: u64,
    pub progress_percent: f64,
    pub is_radio: bool,
    pub on_commit: Callback<FormEvent>,
    pub on_input: Callback<FormEvent>,
}

/// Network bytes already fetched, drawn behind the played position. Before
/// byte totals arrive, an indeterminate scan confirms that loading is active.
#[component]
pub(crate) fn PlaybackBufferIndicator(
    ranges: Vec<BufferedRange>,
    played_percent: f64,
    loading: bool,
) -> Element {
    let visual_ranges = visual_buffer_ranges(&ranges, played_percent);
    rsx! {
        div {
            class: "absolute inset-0 overflow-hidden rounded-full pointer-events-none",
            "aria-hidden": "true",
            for (left, right) in visual_ranges {
                div {
                    class: "absolute top-0 h-full bg-white/30 rounded-full",
                    style: "left: {left}%; width: {right - left}%;"
                }
            }
            if loading && ranges.is_empty() {
                div {
                    class: "h-full w-1/4 bg-[var(--color-primary,#6366f1)] animate-scan"
                }
            }
        }
    }
}

fn visual_buffer_ranges(ranges: &[BufferedRange], played_percent: f64) -> Vec<(f64, f64)> {
    const MERGE_GAP_PERCENT: f64 = 0.75;
    const PLAYHEAD_SNAP_PERCENT: f64 = 5.0;

    let mut visual: Vec<(f64, f64)> = Vec::with_capacity(ranges.len());
    for range in ranges {
        let start = (range.start as f64 / range.total as f64 * 100.0).clamp(0.0, 100.0);
        let end = (range.end as f64 / range.total as f64 * 100.0).clamp(start, 100.0);
        if let Some(previous) = visual.last_mut()
            && start - previous.1 <= MERGE_GAP_PERCENT
        {
            previous.1 = previous.1.max(end);
        } else {
            visual.push((start, end));
        }
    }

    let played_percent = played_percent.clamp(0.0, 100.0);
    if let Some(active) = visual.iter_mut().find(|(start, end)| {
        *end >= played_percent && *start <= played_percent + PLAYHEAD_SNAP_PERCENT
    }) && active.0 > played_percent
    {
        active.0 = played_percent;
    }
    visual
}

pub fn use_seek_drag(
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
) -> SeekDrag {
    let mut ctrl = use_context::<PlayerController>();
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

    let on_commit = use_callback(move |evt: FormEvent| {
        if let Ok(val) = evt.value().parse::<f64>().map(|v| v as u64) {
            ctrl.seek(std::time::Duration::from_secs(val));
            drag_progress.set(val);
            is_dragging.set(false);
        }
    });

    let on_input = use_callback(move |evt: FormEvent| {
        if let Ok(val) = evt.value().parse::<f64>().map(|v| v as u64) {
            is_dragging.set(true);
            drag_progress.set(val);
        }
    });

    SeekDrag {
        display_progress,
        progress_percent,
        is_radio,
        on_commit,
        on_input,
    }
}

pub struct VolumeMute {
    /// What the slider shows: zero while muted.
    pub level: f32,
    pub volume_percent: f32,
    pub is_muted: bool,
    pub toggle_mute: Callback<()>,
    pub on_wheel: Callback<WheelEvent>,
    pub on_commit: Callback<FormEvent>,
    pub on_input: Callback<FormEvent>,
}

pub fn use_volume_mute(
    config: Signal<AppConfig>,
    volume: Signal<f32>,
    persisted_volume: Signal<f32>,
) -> VolumeMute {
    let mut ctrl = use_context::<PlayerController>();
    let muted = ctrl.muted;
    let initial_volume = *volume.read();
    // A slider dragged to zero reads as muted too, and the daemon has no
    // level to restore for it, so the last audible one is kept here.
    let mut last_audible = use_signal(move || {
        if initial_volume > f32::EPSILON {
            initial_volume
        } else {
            0.5f32
        }
    });

    let mut persisted_volume = persisted_volume;

    let toggle_mute = use_callback(move |_: ()| {
        let current = *volume.read();
        if *muted.read() {
            ctrl.set_muted(false);
            persisted_volume.set(current);
        } else if current <= f32::EPSILON {
            let vol = *last_audible.read();
            ctrl.set_volume(vol);
            persisted_volume.set(vol);
        } else {
            ctrl.set_muted(true);
            persisted_volume.set(0.0);
        }
    });

    let on_wheel = use_callback(move |evt: WheelEvent| {
        evt.stop_propagation();
        let dy = evt.delta().strip_units().y;
        if dy.abs() < f64::EPSILON {
            return;
        }
        let step = config.read().volume_scroll_step.max(0.0);
        let dir = if dy < 0.0 { 1.0 } else { -1.0 };
        let current = if *muted.read() { 0.0 } else { *volume.read() };
        let new_val = (current + dir * step).clamp(0.0, 1.0);
        ctrl.set_volume(new_val);
        persisted_volume.set(new_val);
        if new_val > f32::EPSILON {
            last_audible.set(new_val);
        }
    });

    let on_commit = use_callback(move |evt: FormEvent| {
        if let Ok(val) = evt.value().parse::<f32>() {
            persisted_volume.set(val);
        }
    });

    let on_input = use_callback(move |evt: FormEvent| {
        if let Ok(val) = evt.value().parse::<f32>() {
            ctrl.set_volume(val);
            if val > f32::EPSILON {
                last_audible.set(val);
            }
        }
    });

    let is_muted = *muted.read();
    let level = if is_muted { 0.0 } else { *volume.read() };
    VolumeMute {
        level,
        volume_percent: level * 100.0,
        is_muted: is_muted || level <= f32::EPSILON,
        toggle_mute,
        on_wheel,
        on_commit,
        on_input,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ControlsVariant {
    Fullscreen,
    Bar,
}

struct TransportClasses {
    wrapper: &'static str,
    side: &'static str,
    /// The on/off colour of a side toggle (shuffle, repeat) as classes, never as
    /// an inline `style`. Dioxus merges a style attribute with what the element
    /// already has instead of replacing it, so a style that turns the colour on
    /// can only ever be turned off by another style naming the same property:
    /// an empty one leaves the button lit for the rest of the session (#690).
    side_idle: &'static str,
    side_active: &'static str,
    side_icon: &'static str,
    step: &'static str,
    step_icon: &'static str,
    play: &'static str,
    play_icon_size: &'static str,
    badge: &'static str,
}

fn transport_classes(variant: ControlsVariant) -> TransportClasses {
    match variant {
        ControlsVariant::Fullscreen => TransportClasses {
            wrapper: "flex items-center justify-between w-full mb-3",
            side: "w-11 h-11 rounded-full flex items-center justify-center transition-colors active:scale-95 relative flex-shrink-0 hover:bg-white/10",
            side_idle: "text-white/60 hover:text-white",
            side_active: "text-indigo-500",
            side_icon: "text-lg",
            step: "w-14 h-14 rounded-full flex items-center justify-center text-white/90 hover:text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
            step_icon: "text-3xl",
            play: "w-16 h-16 rounded-full flex items-center justify-center text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
            play_icon_size: "text-4xl",
            badge: "absolute bottom-1 left-1/2 -translate-x-1/2 text-[10px] font-bold leading-none",
        },
        ControlsVariant::Bar => TransportClasses {
            wrapper: "flex items-center gap-2",
            side: "w-9 h-9 rounded-full flex items-center justify-center hover:bg-white/10 transition-colors active:scale-95 relative flex-shrink-0",
            side_idle: "text-slate-400 hover:text-white",
            side_active: "text-indigo-500",
            side_icon: "text-sm",
            step: "w-10 h-10 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
            step_icon: "text-xl",
            play: "w-10 h-10 rounded-full flex items-center justify-center text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
            play_icon_size: "text-lg",
            badge: "absolute bottom-0.5 left-1/2 -translate-x-1/2 text-[9px] font-bold leading-none",
        },
    }
}

/// Transport and both sliders force `dir="ltr"`. Time runs left-to-right in
/// every locale, and the native `input[type=range]` flips its value mapping
/// under RTL while the fills below it are drawn from the physical left — so
/// inheriting the app direction would make the sliders track the pointer
/// backwards and put elapsed time on the wrong end.
#[component]
pub fn TransportButtons(is_playing: Signal<bool>, variant: ControlsVariant) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let classes = transport_classes(variant);
    let inner_gap = match variant {
        ControlsVariant::Fullscreen => "flex items-center gap-4",
        ControlsVariant::Bar => "contents",
    };

    rsx! {
        div {
            class: "playback-controls {classes.wrapper}",
            "data-fullscreen": variant == ControlsVariant::Fullscreen,
            dir: "ltr",
            style: if variant == ControlsVariant::Fullscreen { "max-width: 640px;" } else { "" },
            button {
                class: if *ctrl.shuffle.read() { format!("{} {}", classes.side, classes.side_active) } else { format!("{} {}", classes.side, classes.side_idle) },
                title: if *ctrl.shuffle.read() { i18n::t("shuffle_on").to_string() } else { i18n::t("shuffle_off").to_string() },
                onclick: move |_| ctrl.toggle_shuffle(),
                i { class: "fa-solid fa-shuffle {classes.side_icon}" }
            }
            div {
                class: inner_gap,
                button {
                    class: classes.step,
                    onclick: move |_| ctrl.play_prev(),
                    i { class: "fa-solid fa-backward-step {classes.step_icon}" }
                }
                button {
                    class: "playback-play {classes.play}",
                    aria_label: if *is_playing.read() { i18n::t("pause") } else { i18n::t("play") },
                    onclick: move |_| ctrl.toggle(),
                    i { class: if *is_playing.read() { format!("fa-solid fa-pause {}", classes.play_icon_size) } else { format!("fa-solid fa-play {} ml-1", classes.play_icon_size) } }
                }
                button {
                    class: classes.step,
                    onclick: move |_| ctrl.play_next(),
                    i { class: "fa-solid fa-forward-step {classes.step_icon}" }
                }
            }
            button {
                class: match *ctrl.loop_mode.read() {
                    LoopMode::None => format!("{} {}", classes.side, classes.side_idle),
                    _ => format!("{} {}", classes.side, classes.side_active),
                },
                title: match *ctrl.loop_mode.read() {
                    LoopMode::None => i18n::t("repeat_off").to_string(),
                    LoopMode::Queue => i18n::t("repeat_queue").to_string(),
                    LoopMode::Track => i18n::t("repeat_track").to_string(),
                },
                onclick: move |_| ctrl.toggle_loop(),
                i { class: "fa-solid fa-repeat {classes.side_icon}" }
                if let LoopMode::Track = *ctrl.loop_mode.read() {
                    span { class: classes.badge, "1" }
                }
            }
        }
    }
}

#[component]
pub fn SeekSlider(
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
    variant: ControlsVariant,
) -> Element {
    let ctrl = use_context::<PlayerController>();
    let seek = use_seek_drag(current_song_duration, current_song_progress);
    let display_progress = seek.display_progress;
    let progress_percent = seek.progress_percent;
    let is_radio = seek.is_radio;
    let on_commit = seek.on_commit;
    let on_input = seek.on_input;
    let is_loading = *ctrl.is_loading.read();
    let buffered_ranges = ctrl.buffered_ranges.read().clone();
    let can_seek = !is_radio && !is_loading;

    match variant {
        ControlsVariant::Fullscreen => rsx! {
            div {
                class: "w-full mb-3",
                dir: "ltr",
                style: "max-width: 640px;",
                div {
                    class: "flex items-center gap-3",
                    span { class: "playback-time text-xs text-white/70 font-mono", style: "width: 50px; text-align: left;", "{fmt_time(display_progress)}" }
                    div {
                        class: format!("playback-seek flex-1 h-5 {} relative group", if can_seek { "cursor-pointer" } else { "" }),
                        style: "--seek-progress: {progress_percent}%;",
                        div { class: "playback-seek-track absolute inset-x-0 top-2 h-1 bg-white/20 rounded-full" }
                        div { class: "playback-seek-track absolute inset-x-0 top-2 h-1 overflow-hidden rounded-full",
                            PlaybackBufferIndicator {
                                ranges: buffered_ranges.clone(),
                                played_percent: progress_percent,
                                loading: is_loading,
                            }
                        }
                        div {
                            class: "playback-seek-fill absolute left-0 top-2 h-1 rounded-full pointer-events-none bg-white/90",
                            style: "width: {progress_percent}%;"
                        }
                        if !is_loading {
                            div {
                                class: if cfg!(target_os = "android") {
                                    "playback-seek-handle absolute top-1 w-3 h-3 bg-white rounded-full pointer-events-none"
                                } else {
                                    "playback-seek-handle absolute top-1 w-3 h-3 bg-white rounded-full pointer-events-none opacity-0 group-hover:opacity-100 transition-opacity"
                                },
                                style: "left: calc({progress_percent}% - 6px);"
                            }
                        }
                        input {
                            r#type: "range",
                            min: "0",
                            max: "{*current_song_duration.read()}",
                            value: "{display_progress}",
                            class: format!("slider-hit absolute top-0 left-0 w-full h-full opacity-0 {}", if can_seek { "cursor-pointer" } else { "" }),
                            disabled: !can_seek,
                            onchange: move |evt| on_commit.call(evt),
                            oninput: move |evt| on_input.call(evt),
                        }
                    }
                    span { class: "playback-time text-xs text-white/70 font-mono", style: "width: 50px; text-align: right;", "{fmt_time(*current_song_duration.read())}" }
                }
            }
        },
        ControlsVariant::Bar => rsx! {
            div {
                class: "flex items-center gap-2 w-full",
                dir: "ltr",
                span { class: "playback-time text-[10px] text-slate-500 w-8 text-right font-mono", "{fmt_time(display_progress)}" }
                div {
                    class: format!("playback-slider flex-1 h-1 bg-white/10 rounded-full relative {}", if can_seek { "group cursor-pointer" } else { "" }),
                    PlaybackBufferIndicator {
                        ranges: buffered_ranges.clone(),
                        played_percent: progress_percent,
                        loading: is_loading,
                    }
                    div {
                        class: "playback-slider-fill absolute top-0 left-0 h-full bg-white/90 rounded-full pointer-events-none",
                        style: "width: {progress_percent}%",
                            div { class: "absolute -right-1.5 -top-1 w-3 h-3 bg-white rounded-full opacity-0 group-hover:opacity-100 transition-opacity" }
                    }
                    input {
                        r#type: "range",
                        min: "0",
                        max: "{*current_song_duration.read()}",
                        value: "{display_progress}",
                        class: format!("slider-hit absolute top-0 left-0 w-full h-full opacity-0 z-10 {}", if can_seek { "cursor-pointer" } else { "pointer-events-none" }),
                        disabled: !can_seek,
                        onchange: move |evt| on_commit.call(evt),
                        oninput: move |evt| on_input.call(evt),
                    }
                }
                span { class: "playback-time text-[10px] text-slate-500 w-8 font-mono", "{fmt_time(*current_song_duration.read())}" }
            }
        },
    }
}

#[component]
pub fn VolumeSlider(
    config: Signal<AppConfig>,
    volume: Signal<f32>,
    persisted_volume: Signal<f32>,
    variant: ControlsVariant,
) -> Element {
    let vol = use_volume_mute(config, volume, persisted_volume);
    let level = vol.level;
    let volume_percent = vol.volume_percent;
    let is_muted = vol.is_muted;
    let toggle_mute = vol.toggle_mute;
    let on_wheel = vol.on_wheel;
    let on_commit = vol.on_commit;
    let on_input = vol.on_input;

    match variant {
        ControlsVariant::Fullscreen => rsx! {
            div {
                class: "flex items-center gap-5 w-full",
                dir: "ltr",
                style: "max-width: 640px;",
                i { class: "fa-solid fa-volume-low text-white/40" }
                div {
                    class: "playback-seek flex-1 h-5 cursor-pointer relative group",
                    style: "--seek-progress: {volume_percent}%;",
                    onwheel: move |evt| on_wheel.call(evt),
                    div { class: "playback-seek-track absolute inset-x-0 top-2 h-1 bg-white/20 rounded-full" }
                    div {
                        class: "playback-seek-fill absolute left-0 top-2 h-1 bg-white/90 rounded-full pointer-events-none",
                        style: "width: {volume_percent}%;"
                    }
                    div {
                        class: if cfg!(target_os = "android") {
                            "playback-seek-handle absolute top-1 w-3 h-3 bg-white rounded-full pointer-events-none"
                        } else {
                            "playback-seek-handle absolute top-1 w-3 h-3 bg-white rounded-full pointer-events-none opacity-0 group-hover:opacity-100 transition-opacity"
                        },
                        style: "left: calc({volume_percent}% - 6px);"
                    }
                    input {
                        r#type: "range",
                        min: "0",
                        max: "1",
                        step: "0.01",
                        value: "{level}",
                        class: "slider-hit absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer",
                        onchange: move |evt| on_commit.call(evt),
                        oninput: move |evt| on_input.call(evt),
                    }
                }
            }
        },
        ControlsVariant::Bar => rsx! {
            div {
                class: "flex items-center gap-2 group",
                dir: "ltr",
                button {
                    class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                    onclick: move |_| toggle_mute.call(()),
                    i { class: if is_muted { "fa-solid fa-volume-xmark text-xs" } else { "fa-solid fa-volume-high text-xs" } }
                }
                div {
                    class: "playback-slider w-24 h-1 bg-white/10 rounded-full group/vol cursor-pointer relative",
                    onwheel: move |evt| on_wheel.call(evt),
                    div {
                        class: "playback-slider-fill absolute top-0 left-0 h-full bg-white/90 rounded-full pointer-events-none",
                        style: "width: {volume_percent}%",
                        div { class: "absolute -right-1.5 -top-1 w-3 h-3 bg-white rounded-full opacity-0 group-hover/vol:opacity-100 transition-opacity" }
                    }
                    input {
                        r#type: "range",
                        min: "0",
                        max: "1",
                        step: "0.01",
                        value: "{level}",
                        class: "slider-hit absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer z-10",
                        onchange: move |evt| on_commit.call(evt),
                        oninput: move |evt| on_input.call(evt),
                    }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferedRange, visual_buffer_ranges};

    #[test]
    fn buffer_display_coalesces_small_gaps_and_attaches_to_playhead() {
        let ranges = vec![
            BufferedRange {
                start: 680,
                end: 700,
                total: 1_000,
            },
            BufferedRange {
                start: 704,
                end: 720,
                total: 1_000,
            },
        ];

        assert_eq!(visual_buffer_ranges(&ranges, 65.0), vec![(65.0, 72.0)]);
    }

    #[test]
    fn buffer_display_keeps_distant_seek_ranges_separate() {
        let ranges = vec![
            BufferedRange {
                start: 100,
                end: 200,
                total: 1_000,
            },
            BufferedRange {
                start: 800,
                end: 900,
                total: 1_000,
            },
        ];

        assert_eq!(
            visual_buffer_ranges(&ranges, 10.0),
            vec![(10.0, 20.0), (80.0, 90.0)]
        );
    }
}
