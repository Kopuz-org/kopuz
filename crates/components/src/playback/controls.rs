use crate::shared::fmt_time;
use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_player_controller::{LoopMode, PlayerController};
use player::player::Player;

pub struct SeekDrag {
    pub display_progress: u64,
    pub progress_percent: f64,
    pub is_radio: bool,
    pub on_commit: Callback<FormEvent>,
    pub on_input: Callback<FormEvent>,
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
    pub volume_percent: f32,
    pub is_muted: bool,
    pub toggle_mute: Callback<()>,
    pub on_wheel: Callback<WheelEvent>,
    pub on_commit: Callback<FormEvent>,
    pub on_input: Callback<FormEvent>,
}

pub fn use_volume_mute(
    player: Signal<Player>,
    config: Signal<AppConfig>,
    volume: Signal<f32>,
    persisted_volume: Signal<f32>,
) -> VolumeMute {
    let initial_volume = *volume.read();
    let mut is_muted = use_signal(move || initial_volume <= f32::EPSILON);
    let mut volume_before_mute = use_signal(move || {
        if initial_volume > f32::EPSILON {
            initial_volume
        } else {
            0.5f32
        }
    });

    let mut volume = volume;
    let mut persisted_volume = persisted_volume;

    let toggle_mute = use_callback(move |_: ()| {
        let muted = *is_muted.read();
        if muted {
            let vol = *volume_before_mute.read();
            player.peek().set_volume(vol);
            volume.set(vol);
            persisted_volume.set(vol);
            is_muted.set(false);
        } else {
            volume_before_mute.set(*volume.read());
            player.peek().set_volume(0.0);
            volume.set(0.0);
            persisted_volume.set(0.0);
            is_muted.set(true);
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
        let current = *volume.read();
        let new_val = (current + dir * step).clamp(0.0, 1.0);
        player.peek().set_volume(new_val);
        volume.set(new_val);
        persisted_volume.set(new_val);
        is_muted.set(new_val <= f32::EPSILON);
        if new_val > f32::EPSILON {
            volume_before_mute.set(new_val);
        }
    });

    let on_commit = use_callback(move |evt: FormEvent| {
        if let Ok(val) = evt.value().parse::<f32>() {
            persisted_volume.set(val);
            is_muted.set(val == 0.0);
        }
    });

    let on_input = use_callback(move |evt: FormEvent| {
        if let Ok(val) = evt.value().parse::<f32>() {
            player.peek().set_volume(val);
            volume.set(val);
            is_muted.set(val == 0.0);
            if val > f32::EPSILON {
                volume_before_mute.set(val);
            }
        }
    });

    VolumeMute {
        volume_percent: *volume.read() * 100.0,
        is_muted: *is_muted.read(),
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
    side_idle: &'static str,
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
            side_idle: "color: rgba(255,255,255,0.6);",
            side_icon: "text-lg",
            step: "w-14 h-14 rounded-full flex items-center justify-center text-white/90 hover:text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
            step_icon: "text-3xl",
            play: "w-16 h-16 rounded-full flex items-center justify-center text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
            play_icon_size: "text-4xl",
            badge: "absolute bottom-1 left-1/2 -translate-x-1/2 text-[10px] font-bold leading-none",
        },
        ControlsVariant::Bar => TransportClasses {
            wrapper: "flex items-center gap-2",
            side: "w-9 h-9 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95 relative flex-shrink-0",
            side_idle: "",
            side_icon: "text-sm",
            step: "w-10 h-10 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
            step_icon: "text-xl",
            play: "w-10 h-10 rounded-full flex items-center justify-center text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
            play_icon_size: "text-lg",
            badge: "absolute bottom-0.5 left-1/2 -translate-x-1/2 text-[9px] font-bold leading-none",
        },
    }
}

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
            class: classes.wrapper,
            style: if variant == ControlsVariant::Fullscreen { "max-width: 640px;" } else { "" },
            button {
                class: classes.side,
                style: if *ctrl.shuffle.read() { "color: var(--color-indigo-500);" } else { classes.side_idle },
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
                    class: classes.play,
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
                class: classes.side,
                style: match *ctrl.loop_mode.read() {
                    LoopMode::None => classes.side_idle,
                    _ => "color: var(--color-indigo-500);",
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
    let seek = use_seek_drag(current_song_duration, current_song_progress);
    let display_progress = seek.display_progress;
    let progress_percent = seek.progress_percent;
    let is_radio = seek.is_radio;
    let on_commit = seek.on_commit;
    let on_input = seek.on_input;

    match variant {
        ControlsVariant::Fullscreen => rsx! {
            div {
                class: "w-full mb-3",
                style: "max-width: 640px;",
                div {
                    class: "flex items-center gap-3",
                    span { class: "text-xs text-white/70 font-mono", style: "width: 50px; text-align: left;", "{fmt_time(display_progress)}" }
                    div {
                        class: format!("flex-1 {} relative group", if is_radio { "" } else { "cursor-pointer" }),
                        style: "height: 20px;",
                        div {
                            class: "absolute bg-white/20 rounded-full",
                            style: "height: 4px; top: 8px; left: 0; right: 0;"
                        }
                        div {
                            class: "absolute rounded-full pointer-events-none bg-white/90",
                            style: "height: 4px; top: 8px; left: 0; width: {progress_percent}%;"
                        }
                        div {
                            class: if cfg!(target_os = "android") {
                                "absolute bg-white rounded-full pointer-events-none"
                            } else {
                                "absolute bg-white rounded-full pointer-events-none opacity-0 group-hover:opacity-100 transition-opacity"
                            },
                            style: "width: 12px; height: 12px; top: 4px; left: calc({progress_percent}% - 6px);"
                        }
                        input {
                            r#type: "range",
                            min: "0",
                            max: "{*current_song_duration.read()}",
                            value: "{display_progress}",
                            class: format!("slider-hit absolute top-0 left-0 w-full h-full opacity-0 {}", if is_radio { "" } else { "cursor-pointer" }),
                            disabled: is_radio,
                            onchange: move |evt| on_commit.call(evt),
                            oninput: move |evt| on_input.call(evt),
                        }
                    }
                    span { class: "text-xs text-white/70 font-mono", style: "width: 50px; text-align: right;", "{fmt_time(*current_song_duration.read())}" }
                }
            }
        },
        ControlsVariant::Bar => rsx! {
            div {
                class: "flex items-center gap-2 w-full",
                span { class: "text-[10px] text-slate-500 w-8 text-right font-mono", "{fmt_time(display_progress)}" }
                div {
                    class: format!("flex-1 h-1 bg-white/10 rounded-full relative {}", if is_radio { "" } else { "group cursor-pointer" }),
                    div {
                        class: "absolute top-0 left-0 h-full bg-white/90 rounded-full pointer-events-none",
                        style: "width: {progress_percent}%",
                        div { class: "absolute -right-1.5 -top-1 w-3 h-3 bg-white rounded-full opacity-0 group-hover:opacity-100 transition-opacity" }
                    }
                    input {
                        r#type: "range",
                        min: "0",
                        max: "{*current_song_duration.read()}",
                        value: "{display_progress}",
                        class: format!("slider-hit absolute top-0 left-0 w-full h-full opacity-0 z-10 {}", if is_radio { "pointer-events-none" } else { "cursor-pointer" }),
                        disabled: is_radio,
                        onchange: move |evt| on_commit.call(evt),
                        oninput: move |evt| on_input.call(evt),
                    }
                }
                span { class: "text-[10px] text-slate-500 w-8 font-mono", "{fmt_time(*current_song_duration.read())}" }
            }
        },
    }
}

#[component]
pub fn VolumeSlider(
    player: Signal<Player>,
    config: Signal<AppConfig>,
    volume: Signal<f32>,
    persisted_volume: Signal<f32>,
    variant: ControlsVariant,
) -> Element {
    let vol = use_volume_mute(player, config, volume, persisted_volume);
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
                style: "max-width: 640px;",
                i { class: "fa-solid fa-volume-low text-white/40" }
                div {
                    class: "flex-1 cursor-pointer relative group",
                    style: "height: 20px;",
                    onwheel: move |evt| on_wheel.call(evt),
                    div {
                        class: "absolute bg-white/20 rounded-full",
                        style: "height: 4px; top: 8px; left: 0; right: 0;"
                    }
                    div {
                        class: "absolute bg-white/90 rounded-full pointer-events-none",
                        style: "height: 4px; top: 8px; left: 0; width: {volume_percent}%;"
                    }
                    div {
                        class: if cfg!(target_os = "android") {
                            "absolute bg-white rounded-full pointer-events-none"
                        } else {
                            "absolute bg-white rounded-full pointer-events-none opacity-0 group-hover:opacity-100 transition-opacity"
                        },
                        style: "width: 12px; height: 12px; top: 4px; left: calc({volume_percent}% - 6px);"
                    }
                    input {
                        r#type: "range",
                        min: "0",
                        max: "1",
                        step: "0.01",
                        value: "{*volume.read()}",
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
                button {
                    class: "w-9 h-9 rounded-full flex items-center justify-center text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
                    onclick: move |_| toggle_mute.call(()),
                    i { class: if is_muted { "fa-solid fa-volume-xmark text-xs" } else { "fa-solid fa-volume-high text-xs" } }
                }
                div {
                    class: "w-24 h-1 bg-white/10 rounded-full group/vol cursor-pointer relative",
                    onwheel: move |evt| on_wheel.call(evt),
                    div {
                        class: "absolute top-0 left-0 h-full bg-white/90 rounded-full pointer-events-none",
                        style: "width: {volume_percent}%",
                        div { class: "absolute -right-1.5 -top-1 w-3 h-3 bg-white rounded-full opacity-0 group-hover/vol:opacity-100 transition-opacity" }
                    }
                    input {
                        r#type: "range",
                        min: "0",
                        max: "1",
                        step: "0.01",
                        value: "{*volume.read()}",
                        class: "slider-hit absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer z-10",
                        onchange: move |evt| on_commit.call(evt),
                        oninput: move |evt| on_input.call(evt),
                    }
                }
            }
        },
    }
}
