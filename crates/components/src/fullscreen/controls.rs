use crate::shared::fmt_time;
use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_player_controller::{LoopMode, PlayerController};
use player::player::Player;

#[component]
pub(crate) fn ProgressBarControl(
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
) -> Element {
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

    rsx! {
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
                        onchange: move |evt| {
                            if let Ok(val) = evt.value().parse::<f64>().map(|v| v as u64) {
                                ctrl.seek(std::time::Duration::from_secs(val));
                                drag_progress.set(val);
                                is_dragging.set(false);
                            }
                        },
                        oninput: move |evt| {
                            if let Ok(val) = evt.value().parse::<f64>().map(|v| v as u64) {
                                is_dragging.set(true);
                                drag_progress.set(val);
                            }
                        }
                    }
                }
                span { class: "text-xs text-white/70 font-mono", style: "width: 50px; text-align: right;", "{fmt_time(*current_song_duration.read())}" }
            }
        }
    }
}

#[component]
pub(crate) fn VolumeControl(
    mut player: Signal<Player>,
    config: Signal<AppConfig>,
    persisted_volume: Signal<f32>,
    volume: Signal<f32>,
) -> Element {
    let volume_percent = *volume.read() * 100.0;

    rsx! {
        div {
            class: "flex items-center gap-5 w-full",
            style: "max-width: 640px;",
            i { class: "fa-solid fa-volume-low text-white/40" }
            div {
                class: "flex-1 cursor-pointer relative group",
                style: "height: 20px;",
                onwheel: move |evt| {
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
                },
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
                    onchange: move |evt| {
                        if let Ok(val) = evt.value().parse::<f32>() {
                            persisted_volume.set(val);
                        }
                    },
                    oninput: move |evt| {
                        if let Ok(val) = evt.value().parse::<f32>() {
                            player.peek().set_volume(val);
                            volume.set(val);
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn PlaybackControl(mut is_playing: Signal<bool>) -> Element {
    let mut ctrl = use_context::<PlayerController>();

    rsx! {
        div {
            class: "flex items-center justify-between w-full mb-3",
            style: "max-width: 640px;",
            button {
                class: "w-11 h-11 rounded-full flex items-center justify-center transition-colors active:scale-95 relative flex-shrink-0 hover:bg-white/10",
                style: if *ctrl.shuffle.read() { "color: var(--color-indigo-500);" } else { "color: rgba(255,255,255,0.6);" },
                onclick: move |_| ctrl.toggle_shuffle(),
                title: if *ctrl.shuffle.read() { i18n::t("shuffle_on").to_string() } else { i18n::t("shuffle_off").to_string() },
                i { class: "fa-solid fa-shuffle text-lg" }
            }
            div {
                class: "flex items-center gap-4",
                button {
                    class: "w-14 h-14 rounded-full flex items-center justify-center text-white/90 hover:text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
                    onclick: move |_| {
                        ctrl.play_prev();
                    },
                    i { class: "fa-solid fa-backward-step text-3xl" }
                }
                button {
                    class: "w-16 h-16 rounded-full flex items-center justify-center text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
                    onclick: move |_| {
                        ctrl.toggle();
                    },
                    i { class: if *is_playing.read() { "fa-solid fa-pause text-4xl" } else { "fa-solid fa-play text-4xl ml-1" } }
                }
                button {
                    class: "w-14 h-14 rounded-full flex items-center justify-center text-white/90 hover:text-white hover:bg-white/10 transition-colors active:scale-95 flex-shrink-0",
                    onclick: move |_| {
                        ctrl.play_next();
                    },
                    i { class: "fa-solid fa-forward-step text-3xl" }
                }
            }
            button {
                class: "w-11 h-11 rounded-full flex items-center justify-center transition-colors active:scale-95 relative flex-shrink-0 hover:bg-white/10",
                style: match *ctrl.loop_mode.read() {
                    LoopMode::None => "color: rgba(255,255,255,0.6);",
                    _ => "color: var(--color-indigo-500);",
                },
                onclick: move |_| ctrl.toggle_loop(),
                title: match *ctrl.loop_mode.read() {
                    LoopMode::None => i18n::t("repeat_off").to_string(),
                    LoopMode::Queue => i18n::t("repeat_queue").to_string(),
                    LoopMode::Track => i18n::t("repeat_track").to_string(),
                },
                i { class: "fa-solid fa-repeat text-lg" }
                match *ctrl.loop_mode.read() {
                     LoopMode::Track => rsx! {
                         span { class: "absolute bottom-1 left-1/2 -translate-x-1/2 text-[10px] font-bold leading-none", "1" }
                     },
                     _ => rsx! {
                         div {}
                     }
                }
            }
        }
    }
}
