use crate::player_controls::{use_seek_drag, use_volume_mute};
use crate::shared::fmt_time;
use config::AppConfig;
use dioxus::prelude::*;
use player::player::Player;

#[component]
pub(crate) fn ProgressBarControl(
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
) -> Element {
    let seek = use_seek_drag(current_song_duration, current_song_progress);
    let display_progress = seek.display_progress;
    let progress_percent = seek.progress_percent;
    let is_radio = seek.is_radio;
    let on_commit = seek.on_commit;
    let on_input = seek.on_input;

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
                        onchange: move |evt| on_commit.call(evt),
                        oninput: move |evt| on_input.call(evt),
                    }
                }
                span { class: "text-xs text-white/70 font-mono", style: "width: 50px; text-align: right;", "{fmt_time(*current_song_duration.read())}" }
            }
        }
    }
}

#[component]
pub(crate) fn VolumeControl(
    player: Signal<Player>,
    config: Signal<AppConfig>,
    persisted_volume: Signal<f32>,
    volume: Signal<f32>,
) -> Element {
    let vol = use_volume_mute(player, config, volume, persisted_volume);
    let volume_percent = vol.volume_percent;
    let on_wheel = vol.on_wheel;
    let on_commit = vol.on_commit;
    let on_input = vol.on_input;

    rsx! {
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
    }
}
