//! Spotify Connect device picker: routes kopuz's Spotify playback to any of
//! the account's devices (phone, desktop app, speakers) or back to the in-app
//! player. Renders nothing unless Spotify is the signed-in active server.

use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

#[component]
pub fn SpotifyDevicesButton(#[props(default = false)] compact: bool) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut open = use_signal(|| false);
    let mut devices = use_signal(Vec::<::server::spotify::api::ConnectDevice>::new);

    let is_spotify = {
        let cfg = ctrl.config.read();
        cfg.server
            .as_ref()
            .is_some_and(|s| s.service == config::MusicService::Spotify && s.access_token.is_some())
    };
    if !is_spotify {
        return rsx! {};
    }

    let refresh = move || {
        let Some(access) = ctrl.spotify_access_token() else {
            return;
        };
        spawn(async move {
            if let Ok(list) = ::server::spotify::api::devices(&access).await {
                devices.set(list);
            }
        });
    };

    let override_active = ctrl.spotify_device_override.read().is_some();
    let sdk_device = ctrl.spotify_device.read().clone();
    let selected = ctrl.spotify_device_override.read().clone();

    rsx! {
        div { class: "relative",
            button {
                class: match (compact, override_active) {
                    (true, true) => "w-7 h-7 flex items-center justify-center text-indigo-400 hover:text-white transition-colors",
                    (true, false) => "w-7 h-7 flex items-center justify-center text-slate-500 hover:text-white transition-colors",
                    (false, true) => "text-indigo-400 hover:text-white",
                    (false, false) => "text-slate-400 hover:text-white",
                },
                title: i18n::t("spotify_play_on").to_string(),
                onclick: move |_| {
                    let now = !*open.peek();
                    open.set(now);
                    if now {
                        refresh();
                    }
                },
                i { class: if compact { "fa-solid fa-display text-[10px]" } else { "fa-solid fa-display text-xs" } }
            }
            if *open.read() {
                div {
                    class: "absolute bottom-8 right-0 z-50 min-w-52 bg-neutral-900 border border-white/10 rounded-lg shadow-xl p-2 flex flex-col gap-1",
                    p { class: "text-[10px] uppercase tracking-wide text-white/40 px-2 pt-1", "{i18n::t(\"spotify_play_on\")}" }
                    button {
                        class: if selected.is_none() { "text-left text-xs text-indigo-400 px-2 py-1.5 rounded hover:bg-white/10" } else { "text-left text-xs text-white px-2 py-1.5 rounded hover:bg-white/10" },
                        onclick: move |_| {
                            ctrl.spotify_select_device(None);
                            open.set(false);
                        },
                        i { class: "fa-solid fa-music text-[10px] mr-2" }
                        "{i18n::t(\"spotify_this_app\")}"
                    }
                    for d in devices.read().iter().filter(|d| Some(&d.id) != sdk_device.as_ref()).cloned() {
                        {
                            let id = d.id.clone();
                            let chosen = selected.as_deref() == Some(d.id.as_str());
                            let icon = match d.kind.as_str() {
                                "Smartphone" => "fa-solid fa-mobile-screen text-[10px] mr-2",
                                "Speaker" => "fa-solid fa-volume-high text-[10px] mr-2",
                                _ => "fa-solid fa-computer text-[10px] mr-2",
                            };
                            rsx! {
                                button {
                                    key: "{d.id}",
                                    class: if chosen { "text-left text-xs text-indigo-400 px-2 py-1.5 rounded hover:bg-white/10" } else { "text-left text-xs text-white px-2 py-1.5 rounded hover:bg-white/10" },
                                    onclick: move |_| {
                                        ctrl.spotify_select_device(Some(id.clone()));
                                        open.set(false);
                                    },
                                    i { class: icon }
                                    "{d.name}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
