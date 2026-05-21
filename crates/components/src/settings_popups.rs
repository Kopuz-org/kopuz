use config::MusicService;
use dioxus::prelude::*;

#[component]
pub fn AddServerPopup(
    server_name: Signal<String>,
    server_url: Signal<String>,
    server_service: Signal<MusicService>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    let service_value = match server_service() {
        MusicService::Jellyfin => "jellyfin",
        MusicService::Subsonic => "subsonic",
        MusicService::Custom => "custom",
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

                input {
                    placeholder: "{server_url_placeholder}",
                    value: "{server_url()}",
                    oninput: move |e| server_url.set(e.value()),
                    onkeydown: move |e| e.stop_propagation()
                }

                select {
                    onchange: move |e| {
                        let service = match e.value().as_str() {
                            "subsonic" => MusicService::Subsonic,
                            "custom" => MusicService::Custom,
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
