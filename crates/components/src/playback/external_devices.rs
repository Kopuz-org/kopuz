//! Device picker for a source that plays somewhere other than this app: routes
//! playback to any of the account's devices (phone, desktop app, speakers) or
//! back to the in-app player. The `ExternalDevicesButton` in the bottombar
//! toggles a docked `ExternalDevicesPanel` that slides in on the right like the
//! queue rightbar. Both render nothing unless the active source says it has
//! devices and is signed in.
//!
//! The devices are the daemon's answer: it holds the token, it moves playback
//! between them, it is what notices one that started playing on its own, and it
//! picks the glyph for each, so no vocabulary of any one service reaches here.

use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

/// Whether the active source plays devices of its own, and which one to ask.
fn device_source(source: &Memo<Option<api::SourceInfo>>) -> Option<String> {
    let source = source.read();
    let source = source.as_ref()?;
    (source.capabilities.external_devices && source.authenticated).then(|| source.id.clone())
}

/// One selectable target in the device panel, styled like the rightbar's queue
/// rows: a thumbnail-sized icon square, a two-line name/subtitle stack, and —
/// like the active queue item — an accent tint plus a "listening here"
/// indicator when it's the current playback target.
#[component]
fn DeviceRow(
    icon: api::Icon,
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
                crate::forms::Glyph { icon, class: "text-sm".to_string() }
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

/// Bottombar toggle that opens the docked device panel. Renders only when the
/// active source has devices to offer; opening the panel closes the queue
/// rightbar so the two never fight over the right edge.
#[component]
pub fn ExternalDevicesButton(
    #[props(default = false)] compact: bool,
    mut is_rightbar_open: Signal<bool>,
    mut is_devices_open: Signal<bool>,
) -> Element {
    let ctrl = use_context::<PlayerController>();
    let source = hooks::sources::use_active_source_info();
    if device_source(&source).is_none() {
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
            title: i18n::t("external_play_on").to_string(),
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
pub fn ExternalDevicesPanel(
    mut is_devices_open: Signal<bool>,
    is_rightbar_open: Signal<bool>,
) -> Element {
    let api = hooks::use_api();
    let source = hooks::sources::use_active_source_info();
    let playing_source = device_source(&source);

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
        let asked = device_source(&source);
        async move {
            let Some(asked) = asked.filter(|_| open) else {
                return Vec::new();
            };
            api.external_devices(asked).await.unwrap_or_default()
        }
    });

    if playing_source.is_none() || !*is_devices_open.read() {
        return rsx! {};
    }

    let listed = devices.read().clone().unwrap_or_default();
    let selected = listed
        .iter()
        .find(|device| device.active)
        .map(|device| device.id.clone());

    let select = move |device_id: Option<String>| {
        let api = hooks::consume_api();
        let asked = device_source(&source);
        spawn(async move {
            let Some(asked) = asked else {
                return;
            };
            if let Err(error) = api.select_external_device(asked, device_id).await {
                tracing::warn!(%error, "moving playback to another device failed");
                hooks::toast::toast_error(&error.to_string());
            }
            devices.restart();
        });
    };

    rsx! {
        div {
            id: "external-devices-root",
            class: "bg-black/40 border-l border-white/5 flex flex-col h-full flex-shrink-0 z-10",
            style: "width: 320px; min-width: 320px;",

            div {
                class: "flex items-center justify-between px-4 py-4 border-b border-white/10",
                span {
                    class: "text-[10px] font-medium uppercase tracking-wider text-white",
                    "{i18n::t(\"external_play_on\")}"
                }
                button {
                    class: "text-white/40 hover:text-white",
                    onclick: move |_| is_devices_open.set(false),
                    i { class: "fa-solid fa-xmark text-sm" }
                }
            }

            div { class: "flex-1 overflow-y-auto px-2 py-2 flex flex-col gap-0.5",
                DeviceRow {
                    icon: api::Icon::Class("fa-solid fa-music".to_string()),
                    name: i18n::t("external_this_app").to_string(),
                    subtitle: None,
                    chosen: selected.is_none(),
                    onclick: move |_| select(None),
                }
                for device in listed.iter().cloned() {
                    {
                        let id = device.id.clone();
                        let chosen = selected.as_deref() == Some(device.id.as_str());
                        rsx! {
                            DeviceRow {
                                key: "{device.id}",
                                icon: device.icon.clone(),
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
