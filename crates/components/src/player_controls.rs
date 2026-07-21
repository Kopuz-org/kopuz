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
pub enum TransportVariant {
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

fn transport_classes(variant: TransportVariant) -> TransportClasses {
    match variant {
        TransportVariant::Fullscreen => TransportClasses {
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
        TransportVariant::Bar => TransportClasses {
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
pub fn TransportButtons(is_playing: Signal<bool>, variant: TransportVariant) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let classes = transport_classes(variant);
    let inner_gap = match variant {
        TransportVariant::Fullscreen => "flex items-center gap-4",
        TransportVariant::Bar => "contents",
    };

    rsx! {
        div {
            class: classes.wrapper,
            style: if variant == TransportVariant::Fullscreen { "max-width: 640px;" } else { "" },
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
