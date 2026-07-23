use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

use crate::shared::fmt_time;

/// Global flag toggling the compact mini-player overlay. Provided via context
/// from the app root so the bottom bar (and a keyboard shortcut) can flip it
/// while the overlay itself reads it to render/hide.
#[derive(Clone, Copy)]
pub struct CompactMode(pub Signal<bool>);

/// Style-dependent classes so the mini player visually relates to the active
/// UI style (Normal vs Vaxry), mirroring how the bottom bar splits.
struct CompactSkin {
    container: &'static str,
    title: &'static str,
    cover: &'static str,
    play_btn: &'static str,
    play_icon: &'static str,
    ctrl_btn: &'static str,
    progress_fill: &'static str,
}

fn skin_for(style: UiStyle) -> CompactSkin {
    match style {
        UiStyle::Normal => CompactSkin {
            container: "bg-[#0a0a0a]",
            title: "text-[14px] font-bold text-white/95 truncate leading-tight",
            cover: "rounded-lg ring-1 ring-white/10 shadow-lg shadow-black/50",
            play_btn: "w-10 h-10 flex items-center justify-center rounded-full text-white hover:bg-white/10 transition-colors active:scale-95",
            play_icon: "text-base",
            ctrl_btn: "w-9 h-9 flex items-center justify-center rounded-full text-slate-300 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
            progress_fill: "bg-white/90",
        },
        UiStyle::Vaxry => CompactSkin {
            container: "bg-black",
            title: "text-[13px] font-mono font-semibold uppercase tracking-wide text-white/90 truncate leading-tight",
            cover: "rounded-none ring-1 ring-white/10",
            play_btn: "w-9 h-9 flex items-center justify-center rounded-full bg-white/10 hover:bg-white/20 text-white transition-colors active:scale-95",
            play_icon: "text-sm",
            ctrl_btn: "w-9 h-9 flex items-center justify-center rounded-full text-slate-400 hover:text-white hover:bg-white/10 transition-colors active:scale-95",
            progress_fill: "bg-white/90",
        },
    }
}

/// Small always-on-top mini player. Rendered as a full-window overlay; the app
/// root shrinks the OS window to a compact size while this is active, so the
/// overlay fills it. Desktop only — the bottom bar hides the toggle elsewhere.
#[component]
pub fn CompactPlayer() -> Element {
    let CompactMode(mut compact_mode) = use_context::<CompactMode>();

    if !*compact_mode.read() {
        return rsx! {};
    }

    let mut ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();
    let skin = skin_for(config.read().ui_style);

    let mut is_dragging = use_signal(|| false);
    let mut drag_progress = use_signal(|| 0u64);

    let duration = *ctrl.current_song_duration.read();
    let display_progress = if *is_dragging.read() {
        *drag_progress.read()
    } else {
        *ctrl.current_song_progress.read()
    };
    let progress_percent = if duration > 0 {
        (display_progress as f64 / duration as f64) * 100.0
    } else {
        0.0
    };
    let is_radio = duration == u64::MAX;

    let cover = ctrl.current_song_cover_url.read().clone();
    let is_playing = *ctrl.is_playing.read();

    let macos_top = cfg!(target_os = "macos");

    rsx! {
        div {
            class: format!(
                "fixed inset-0 z-[200] flex flex-col select-none overflow-hidden rounded-xl ring-1 ring-inset ring-white/10 {} {}",
                skin.container,
                if macos_top { "pt-[22px]" } else { "" },
            ),
            onmousedown: move |_| {
                #[cfg(not(target_os = "android"))]
                dioxus::desktop::window().drag();
            },

            if !cover.is_empty() {
                img {
                    src: "{cover}",
                    class: "absolute inset-0 w-full h-full object-cover scale-125 blur-2xl opacity-40 pointer-events-none",
                }
            }
            div {
                class: "absolute inset-0 bg-gradient-to-r from-black/90 via-black/75 to-black/90 pointer-events-none",
            }

            div {
                class: "relative z-10 flex-1 min-h-0 flex items-center gap-3 px-3 pt-2 pb-1",

                div {
                    class: format!("h-full aspect-square overflow-hidden bg-white/5 shrink-0 flex items-center justify-center {}", skin.cover),
                    if cover.is_empty() {
                        i { class: "fa-solid fa-music text-white/20" }
                    } else {
                        img { src: "{cover}", class: "w-full h-full object-cover" }
                    }
                }

                div {
                    class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                    span { class: skin.title, "{ctrl.current_song_title}" }
                    span { class: "text-[11px] text-white/55 truncate leading-tight", "{ctrl.current_song_artist}" }
                    span { class: "text-[9px] text-white/35 font-mono leading-tight",
                        if is_radio { "LIVE" } else { "{fmt_time(display_progress)} / {fmt_time(duration)}" }
                    }
                }

                button {
                    class: "w-9 h-9 flex items-center justify-center rounded-full text-slate-500 hover:text-white hover:bg-white/10 transition-colors active:scale-95 shrink-0",
                    title: i18n::t("restore_full_player").to_string(),
                    onmousedown: move |evt| evt.stop_propagation(),
                    onclick: move |_| compact_mode.set(false),
                    i { class: "fa-solid fa-up-right-and-down-left-from-center text-xs" }
                }
            }

            div {
                class: "relative z-10 shrink-0 flex items-center justify-center gap-2 px-3 pb-2",
                onmousedown: move |evt| evt.stop_propagation(),
                button {
                    class: skin.ctrl_btn,
                    onclick: move |_| ctrl.play_prev(),
                    i { class: "fa-solid fa-backward-step text-sm" }
                }
                button {
                    class: skin.play_btn,
                    onclick: move |_| ctrl.toggle(),
                    i { class: if is_playing { format!("fa-solid fa-pause {}", skin.play_icon) } else { format!("fa-solid fa-play {} ml-0.5", skin.play_icon) } }
                }
                button {
                    class: skin.ctrl_btn,
                    onclick: move |_| ctrl.play_next(),
                    i { class: "fa-solid fa-forward-step text-sm" }
                }
            }

            div {
                class: format!("relative z-10 h-[3px] w-full bg-white/10 shrink-0 {}", if is_radio { "" } else { "group cursor-pointer" }),
                onmousedown: move |evt| evt.stop_propagation(),
                div {
                    class: format!("absolute top-0 left-0 h-full pointer-events-none {}", skin.progress_fill),
                    style: "width: {progress_percent}%",
                }
                input {
                    r#type: "range",
                    min: "0",
                    max: "{duration}",
                    value: "{display_progress}",
                    class: format!("slider-hit absolute top-0 left-0 w-full h-full opacity-0 z-10 {}", if is_radio { "pointer-events-none" } else { "cursor-pointer" }),
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
        }
    }
}
