//! Spotify Connect device picker: routes kopuz's Spotify playback to any of
//! the account's devices (phone, desktop app, speakers) or back to the in-app
//! player. The `SpotifyDevicesButton` in the bottombar toggles a docked
//! `SpotifyDevicesPanel` that slides in on the right like the queue rightbar,
//! mirroring Spotify's own "Connect to a device" panel. Both render nothing
//! unless Spotify is the signed-in active server.

use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

/// True when Spotify is the active server and we hold an access token — the
/// only state in which either device widget should appear.
fn spotify_active(ctrl: &PlayerController) -> bool {
    let cfg = ctrl.config.read();
    cfg.server
        .as_ref()
        .is_some_and(|s| s.service == config::MusicService::Spotify && s.access_token.is_some())
}

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
/// Spotify is the active server; opening the panel closes the queue rightbar so
/// the two never fight over the right edge.
#[component]
pub fn SpotifyDevicesButton(
    #[props(default = false)] compact: bool,
    mut is_rightbar_open: Signal<bool>,
    mut is_devices_open: Signal<bool>,
) -> Element {
    let ctrl = use_context::<PlayerController>();

    if !spotify_active(&ctrl) {
        return rsx! {};
    }

    let override_active = ctrl.spotify_device_override.read().is_some();

    rsx! {
        button {
            class: match (compact, override_active) {
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
/// styled to match it. Fetches the account's Connect devices each time it opens.
#[component]
pub fn SpotifyDevicesPanel(
    mut is_devices_open: Signal<bool>,
    is_rightbar_open: Signal<bool>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut devices = use_signal(Vec::<::server::spotify::api::ConnectDevice>::new);

    // The rightbar and this panel are mutually exclusive; opening the rightbar
    // dismisses us.
    use_effect(move || {
        if *is_rightbar_open.read() {
            is_devices_open.set(false);
        }
    });

    // Refresh the device list every time the panel is opened.
    use_effect(move || {
        if !*is_devices_open.read() {
            return;
        }
        let Some(access) = ctrl.spotify_access_token() else {
            return;
        };
        spawn(async move {
            if let Ok(list) = ::server::spotify::api::devices(&access).await {
                devices.set(list);
            }
        });
    });

    if !*is_devices_open.read() || !spotify_active(&ctrl) {
        return rsx! {};
    }

    let sdk_device = ctrl.spotify_device.read().clone();
    let selected = ctrl.spotify_device_override.read().clone();

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
                    onclick: move |_| {
                        ctrl.spotify_select_device(None);
                    },
                }
                for d in devices.read().iter().filter(|d| Some(&d.id) != sdk_device.as_ref()).cloned() {
                    {
                        let id = d.id.clone();
                        let chosen = selected.as_deref() == Some(d.id.as_str());
                        let icon = match d.kind.as_str() {
                            "Smartphone" => "fa-solid fa-mobile-screen",
                            "Speaker" => "fa-solid fa-volume-high",
                            _ => "fa-solid fa-computer",
                        };
                        rsx! {
                            DeviceRow {
                                key: "{d.id}",
                                icon,
                                name: d.name.clone(),
                                subtitle: (!d.kind.is_empty()).then(|| d.kind.clone()),
                                chosen,
                                onclick: move |_| {
                                    ctrl.spotify_select_device(Some(id.clone()));
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}
