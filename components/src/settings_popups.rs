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

    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "Add Media Server" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "Server name (optional)",
                    value: "{server_name()}",
                    oninput: move |e| server_name.set(e.value()),
                    onkeydown: move |e| e.stop_propagation()
                }

                input {
                    placeholder: "http://localhost:8096",
                    value: "{server_url()}",
                    oninput: move |e| server_url.set(e.value()),
                    onkeydown: move |e| e.stop_propagation()
                }

                select {
                    value: "{service_value}",
                    onchange: move |e| {
                        let service = match e.value().as_str() {
                            "subsonic" => MusicService::Subsonic,
                            "custom" => MusicService::Custom,
                            _ => MusicService::Jellyfin,
                        };
                        server_service.set(service);
                    },
                    onkeydown: move |e| e.stop_propagation(),
                    option { value: "jellyfin", "Jellyfin" }
                    option { value: "subsonic", "Subsonic" }
                    option { value: "custom", "Custom (manual API)" }
                }

                div { class: "actions",
                    button {
                        onclick: move |_| on_close.call(()),
                        "Cancel"
                    }
                    button {
                        onclick: move |_| on_save.call(()),
                        "Save"
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
    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "Login to {service_name}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "Username",
                    value: "{username()}",
                    oninput: move |e| username.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                    disabled: loading()
                }

                input {
                    r#type: "password",
                    placeholder: "Password",
                    value: "{password()}",
                    oninput: move |e| password.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                    disabled: loading()
                }

                div { class: "actions",
                    button {
                        onclick: move |_| if !loading() { on_close.call(()) },
                        disabled: loading(),
                        "Cancel"
                    }
                    button {
                        onclick: move |_| if !loading() { on_save.call(()) },
                        disabled: loading(),
                        if loading() { "Logging in..." } else { "Login" }
                    }
                }
            }
        }
    }
}
