//! Spotify Connect device picker: routes kopuz's Spotify playback to any of
//! the account's devices (phone, desktop app, speakers) or back to the in-app
//! player. The `SpotifyDevicesButton` in the bottombar toggles a docked
//! `SpotifyDevicesPanel` that slides in on the right like the queue rightbar,
//! mirroring Spotify's own "Connect to a device" panel. Both render nothing
//! unless Spotify is the signed-in active source.
//!
//! The devices are the daemon's answer: it holds the token, it moves playback
//! between them, and it is what notices one that started playing on its own.

use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

/// The integration these devices belong to; the daemon takes it by name.
const KIND: &str = "spotify";

/// One selectable target in the device panel, styled like the rightbar's queue
/// rows: a thumbnail-sized icon square, a two-line name/subtitle stack, and —
/// like the active queue item — an accent tint plus a "listening here"
/// indicator when it's the current playback target.
#[component]
fn DeviceRow(
    icon: &'static str,
    name: String,
    subtitle: Option<String>,
    chosen: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            class: if chosen {
                "w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-left cursor-pointer transition-colors"
            } else {
                "w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-left cursor-pointer transition-colors hover:bg-white/5"
            },
            style: if chosen {
                "background: color-mix(in oklab, var(--color-indigo-500) 12%, transparent);"
            } else {
                ""
            },
            onclick: move |evt| onclick.call(evt),

            div {
                class: "w-10 h-10 rounded-md flex items-center justify-center shrink-0",
                style: if chosen {
                    "background: color-mix(in oklab, var(--color-indigo-500) 18%, transparent); color: var(--color-indigo-500);"
                } else {
                    "background: rgba(255,255,255,0.06); color: rgba(255,255,255,0.6);"
                },
                i { class: "{icon} text-sm" }
            }

            div { class: "flex-1 min-w-0 flex flex-col justify-center gap-0.5",
                span {
                    class: "text-sm truncate",
                    style: if chosen { "color: var(--color-indigo-500);" } else { "color: #ffffff;" },
                    "{name}"
                }
                if let Some(subtitle) = subtitle {
                    span {
                        class: "text-xs truncate",
                        style: if chosen {
                            "color: color-mix(in oklab, var(--color-indigo-500) 70%, transparent);"
                        } else {
                            "color: rgba(255,255,255,0.5);"
                        },
                        "{subtitle}"
                    }
                }
            }

            if chosen {
                i {
                    class: "fa-solid fa-volume-high text-xs shrink-0",
                    style: "color: var(--color-indigo-500);",
                }
            }
        }
    }
}

/// Bottombar toggle that opens the docked device panel. Renders only when
/// Spotify is the signed-in active source; opening the panel closes the queue
/// rightbar so the two never fight over the right edge.
#[component]
pub fn SpotifyDevicesButton(
    #[props(default = false)] compact: bool,
    mut is_rightbar_open: Signal<bool>,
    mut is_devices_open: Signal<bool>,
) -> Element {
    let ctrl = use_context::<PlayerController>();
    let source = hooks::sources::use_active_source_info();
    let is_spotify = source.read().as_ref().is_some_and(|source| {
        source.service == Some(config::MusicService::Spotify) && source.authenticated
    });
    if !is_spotify {
        return rsx! {};
    }

    let elsewhere = ctrl.external_device.read().is_some();

    rsx! {
        button {
            class: match (compact, elsewhere) {
                (true, true) => "w-7 h-7 flex items-center justify-center text-indigo-400 hover:text-white transition-colors",
                (true, false) => "w-7 h-7 flex items-center justify-center text-slate-500 hover:text-white transition-colors",
                (false, true) => "text-indigo-400 hover:text-white",
                (false, false) => "text-slate-400 hover:text-white",
            },
            title: i18n::t("spotify_play_on").to_string(),
            onclick: move |_| {
                let now = !*is_devices_open.peek();
                if now {
                    is_rightbar_open.set(false);
                }
                is_devices_open.set(now);
            },
            i { class: if compact { "fa-solid fa-display text-[10px]" } else { "fa-solid fa-display text-xs" } }
        }
    }
}

/// Full-height panel docked on the right edge, sibling to the queue rightbar and
/// styled to match it. Asks the daemon for the account's devices each time it
/// opens.
#[component]
pub fn SpotifyDevicesPanel(
    mut is_devices_open: Signal<bool>,
    is_rightbar_open: Signal<bool>,
) -> Element {
    let api = hooks::use_api();
    let source = hooks::sources::use_active_source_info();
    let is_spotify = source.read().as_ref().is_some_and(|source| {
        source.service == Some(config::MusicService::Spotify) && source.authenticated
    });

    // The rightbar and this panel are mutually exclusive; opening the rightbar
    // dismisses us.
    use_effect(move || {
        if *is_rightbar_open.read() {
            is_devices_open.set(false);
        }
    });

    // Refresh the device list every time the panel is opened.
    let mut devices = use_resource(move || {
        let api = api.clone();
        let open = is_devices_open();
        async move {
            if !open {
                return Vec::new();
            }
            api.external_devices(KIND.to_string())
                .await
                .unwrap_or_default()
        }
    });

    if !*is_devices_open.read() || !is_spotify {
        return rsx! {};
    }

    let listed = devices.read().clone().unwrap_or_default();
    let selected = listed
        .iter()
        .find(|device| device.active)
        .map(|device| device.id.clone());

    let select = move |device_id: Option<String>| {
        let api = hooks::consume_api();
        spawn(async move {
            if let Err(error) = api
                .select_external_device(KIND.to_string(), device_id)
                .await
            {
                tracing::warn!(%error, "moving Spotify playback failed");
                hooks::toast::toast_error(&error.to_string());
            }
            devices.restart();
        });
    };

    rsx! {
        div {
            id: "spotify-devices-root",
            class: "bg-black/40 border-l border-white/5 flex flex-col h-full flex-shrink-0 z-10",
            style: "width: 320px; min-width: 320px;",

            div {
                class: "flex items-center justify-between px-4 py-4 border-b border-white/10",
                span {
                    class: "text-[10px] font-medium uppercase tracking-wider text-white",
                    "{i18n::t(\"spotify_play_on\")}"
                }
                button {
                    class: "text-white/40 hover:text-white",
                    onclick: move |_| is_devices_open.set(false),
                    i { class: "fa-solid fa-xmark text-sm" }
                }
            }

            div { class: "flex-1 overflow-y-auto px-2 py-2 flex flex-col gap-0.5",
                DeviceRow {
                    icon: "fa-solid fa-music",
                    name: i18n::t("spotify_this_app").to_string(),
                    subtitle: None,
                    chosen: selected.is_none(),
                    onclick: move |_| select(None),
                }
                for device in listed.iter().cloned() {
                    {
                        let id = device.id.clone();
                        let chosen = selected.as_deref() == Some(device.id.as_str());
                        let icon = match device.kind.as_str() {
                            "Smartphone" => "fa-solid fa-mobile-screen",
                            "Speaker" => "fa-solid fa-volume-high",
                            _ => "fa-solid fa-computer",
                        };
                        rsx! {
                            DeviceRow {
                                key: "{device.id}",
                                icon,
                                name: device.name.clone(),
                                subtitle: (!device.kind.is_empty()).then(|| device.kind.clone()),
                                chosen,
                                onclick: move |_| select(Some(id.clone())),
                            }
                        }
                    }
                }
            }
        }
    }
}
