use config::{Browser, MusicService};
use dioxus::prelude::*;

#[component]
pub fn AddServerPopup(
    server_name: Signal<String>,
    server_url: Signal<String>,
    server_service: Signal<MusicService>,
    /// Selected Chromium-family browser when service is YouTube Music.
    yt_browser: Signal<Browser>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    let _service_value = match server_service() {
        MusicService::Jellyfin => "jellyfin",
        MusicService::Subsonic => "subsonic",
        MusicService::Custom => "custom",
        MusicService::YtMusic => "ytmusic",
    };

    let server_name_optional = i18n::t("server_name_optional").to_string();
    let server_url_placeholder = i18n::t("server_url_placeholder").to_string();
    let custom_manual = i18n::t("custom_manual").to_string();
    let cancel_text = i18n::t("cancel").to_string();
    let save_text = i18n::t("save").to_string();

    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "{i18n::t(\"add_media_server\")}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "{server_name_optional}",
                    value: "{server_name()}",
                    oninput: move |e| server_name.set(e.value()),
                    onkeydown: move |e| e.stop_propagation()
                }

                ServerServiceFields {
                    server_service,
                    server_url,
                    yt_browser,
                    server_url_placeholder: server_url_placeholder.clone(),
                }

                select {
                    onchange: move |e| {
                        let service = match e.value().as_str() {
                            "subsonic" => MusicService::Subsonic,
                            "custom" => MusicService::Custom,
                            "ytmusic" => MusicService::YtMusic,
                            _ => MusicService::Jellyfin,
                        };
                        server_service.set(service);
                    },
                    onkeydown: move |e| e.stop_propagation(),
                    option {
                        value: "jellyfin",
                        selected: server_service() == MusicService::Jellyfin,
                        "{i18n::t(\"jellyfin\")}"
                    }
                    option {
                        value: "subsonic",
                        selected: server_service() == MusicService::Subsonic,
                        "{i18n::t(\"subsonic\")}"
                    }
                    option {
                        value: "custom",
                        selected: server_service() == MusicService::Custom,
                        "{custom_manual}"
                    }
                    option {
                        value: "ytmusic",
                        selected: server_service() == MusicService::YtMusic,
                        "YouTube Music"
                    }
                }

                div { class: "actions",
                    button {
                        onclick: move |_| on_close.call(()),
                        "{cancel_text}"
                    }
                    button {
                        onclick: move |_| on_save.call(()),
                        "{save_text}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn LoginPopup(
    username: Signal<String>,
    password: Signal<String>,
    service_name: String,
    error: Signal<Option<String>>,
    loading: Signal<bool>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    let cancel_text = i18n::t("cancel").to_string();
    let login_text = i18n::t("login").to_string();
    let username_placeholder = i18n::t("username").to_string();
    let password_placeholder = i18n::t("password").to_string();
    let login_to_service_text =
        i18n::t_with("login_to_service", &[("service", service_name.clone())]);

    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "{login_to_service_text}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "{username_placeholder}",
                    value: "{username()}",
                    oninput: move |e| username.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                    disabled: loading()
                }

                input {
                    r#type: "password",
                    placeholder: "{password_placeholder}",
                    value: "{password()}",
                    oninput: move |e| password.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                    disabled: loading()
                }

                div { class: "actions",
                    button {
                        onclick: move |_| if !loading() { on_close.call(()) },
                        disabled: loading(),
                        "{cancel_text}"
                    }
                    button {
                        onclick: move |_| if !loading() { on_save.call(()) },
                        disabled: loading(),
                        if loading() { "{i18n::t(\"logging_in\")}" } else { "{login_text}" }
                    }
                }
            }
        }
    }
}

#[component]
pub fn AddRegistryPopup(
    registry_url: Signal<String>,
    error: Signal<Option<String>>,
    loading: Signal<bool>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    let url_placeholder = i18n::t("radio_registry_url_placeholder").to_string();
    let cancel_text = i18n::t("cancel").to_string();
    let save_text = i18n::t("save").to_string();

    rsx! {
        div {
            class: "overlay",
            onclick: move |_| { if !loading() { on_close.call(()) } },

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "{i18n::t(\"add_radio_registry\")}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "{url_placeholder}",
                    value: "{registry_url()}",
                    oninput: move |e| registry_url.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                    disabled: loading()
                }

                div { class: "actions",
                    button {
                        onclick: move |_| if !loading() { on_close.call(()) },
                        disabled: loading(),
                        "{cancel_text}"
                    }
                    button {
                        onclick: move |_| if !loading() { on_save.call(()) },
                        disabled: loading(),
                        if loading() { "{i18n::t(\"saving\")}" } else { "{save_text}" }
                    }
                }
            }
        }
    }
}

#[component]
fn ServerServiceFields(
    server_service: Signal<MusicService>,
    server_url: Signal<String>,
    yt_browser: Signal<Browser>,
    server_url_placeholder: String,
) -> Element {
    let mut botguard_status: Signal<Option<Result<(), String>>> = use_signal(|| None);

    match server_service() {
        MusicService::YtMusic => rsx! {
            p { class: "text-xs text-white/60",
                "Pick which browser kopuz should use for the YouTube Music sign-in window. It opens in an isolated profile (a fresh, separate session) — your normal browsing is untouched. Make sure the browser is installed."
            }
            select {
                onchange: move |e| {
                    if let Some(b) = Browser::from_id(&e.value()) {
                        yt_browser.set(b);
                    }
                },
                onkeydown: move |e| e.stop_propagation(),
                for browser in Browser::ALL.iter().copied() {
                    option {
                        value: "{browser.id()}",
                        selected: yt_browser() == browser,
                        "{browser.label()}"
                    }
                }
            }
            div { class: "flex items-center gap-2 mt-2",
                button {
                    class: "text-xs px-2 py-1 rounded bg-white/10 hover:bg-white/20 transition-colors",
                    onclick: move |_| {
                        spawn(async move {
                            let res = ::server::ytmusic::botguard::check_available().await;
                            botguard_status.set(Some(res));
                        });
                    },
                    "Check rustypipe-botguard"
                }
                {match botguard_status.read().as_ref() {
                    Some(Ok(())) => rsx! {
                        span { class: "text-xs text-emerald-400",
                            i { class: "fa-solid fa-check mr-1" }
                            "Installed"
                        }
                    },
                    Some(Err(msg)) => rsx! {
                        span { class: "text-xs text-rose-400 whitespace-pre-line",
                            i { class: "fa-solid fa-xmark mr-1" }
                            "{msg}"
                        }
                    },
                    None => rsx! { span {} },
                }}
            }
        },
        _ => rsx! {
            input {
                placeholder: "{server_url_placeholder}",
                value: "{server_url()}",
                oninput: move |e| server_url.set(e.value()),
                onkeydown: move |e| e.stop_propagation()
            }
        },
    }
}
