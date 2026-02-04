use dioxus::prelude::*;

#[component]
pub fn AddServerPopup(
    server_name: Signal<String>,
    server_url: Signal<String>,
    error: Signal<Option<String>>,
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

                h2 { "Add Jellyfin Server" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "Server name (optional)",
                    value: "{server_name()}",
                    oninput: move |e| server_name.set(e.value())
                }

                input {
                    placeholder: "http://localhost:8096",
                    value: "{server_url()}",
                    oninput: move |e| server_url.set(e.value())
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

                h2 { "Login to Jellyfin" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "Username",
                    value: "{username()}",
                    oninput: move |e| username.set(e.value()),
                    disabled: loading()
                }

                input {
                    r#type: "password",
                    placeholder: "Password",
                    value: "{password()}",
                    oninput: move |e| password.set(e.value()),
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
