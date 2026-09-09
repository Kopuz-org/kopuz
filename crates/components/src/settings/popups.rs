use crate::settings_items::MultiDirectoryPicker;
use dioxus::prelude::*;

#[component]
pub fn AddLocalSourcePopup(
    name: Signal<String>,
    directories: Signal<Vec<std::path::PathBuf>>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "overlay", onclick: move |_| on_close.call(()),
            div { class: "popup", onclick: |e| e.stop_propagation(),
                h2 { "{i18n::t(\"add_local_library\")}" }
                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }
                input {
                    placeholder: "{i18n::t(\"local_library_name\")}",
                    value: "{name()}",
                    oninput: move |e| name.set(e.value()),
                    onkeydown: move |e| e.stop_propagation(),
                }
                MultiDirectoryPicker {
                    current_paths: directories(),
                    on_add: move |path| {
                        if !directories.peek().contains(&path) {
                            directories.write().push(path);
                        }
                    },
                    on_remove: move |index| {
                        if index < directories.peek().len() {
                            directories.write().remove(index);
                        }
                    },
                }
                div { class: "actions",
                    button { onclick: move |_| on_close.call(()), "{i18n::t(\"cancel\")}" }
                    button { onclick: move |_| on_save.call(()), "{i18n::t(\"save\")}" }
                }
            }
        }
    }
}

/// Add a server: pick a service, fill in whatever form the daemon publishes
/// for it. Nothing here knows what any of those fields mean.
#[component]
pub fn AddServerPopup(
    services: Vec<api::ServiceInfo>,
    service: Signal<String>,
    name: Signal<String>,
    values: Signal<Vec<api::FieldValue>>,
    secrets: Signal<Vec<api::FieldValue>>,
    check: Option<api::DraftCheck>,
    host_access: bool,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    let chosen = services
        .iter()
        .find(|offered| offered.id == service())
        .or_else(|| services.first())
        .cloned()
        .unwrap_or_default();
    let fields = chosen.fields.clone();
    // A sandboxed daemon cannot open a browser, so a service whose sign-in is
    // one cannot be saved from here at all.
    let needs_browser = check
        .as_ref()
        .is_some_and(|check| check.sign_in == api::SignInKind::Browser);
    let blocked = needs_browser && !host_access;
    let problems = check.map(|check| check.problems).unwrap_or_default();
    let flatpak_access_command =
        "flatpak override --user --talk-name=org.freedesktop.Flatpak moe.kopuz.kopuz";

    let mut answered = values();
    answered.extend(secrets());

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

                if blocked {
                    div { class: "warning",
                        p { "{i18n::t(\"browser_sign_in_needs_host\")}" }
                        button {
                            class: "flatpak-command",
                            title: i18n::t("copy"),
                            aria_label: i18n::t("copy"),
                            onclick: move |_| {
                                let js = format!(
                                    "navigator.clipboard.writeText('{flatpak_access_command}').catch((e) => console.error('clipboard writeText failed', e));"
                                );
                                let _ = dioxus::document::eval(&js);
                            },
                            "{flatpak_access_command}"
                        }
                    }
                }

                input {
                    placeholder: "{i18n::t(\"server_name\")}",
                    value: "{name()}",
                    oninput: move |e| name.set(e.value()),
                    onkeydown: move |e| e.stop_propagation()
                }

                select {
                    onchange: move |e| {
                        service.set(e.value());
                        values.set(Vec::new());
                        secrets.set(Vec::new());
                    },
                    onkeydown: move |e| e.stop_propagation(),
                    for offered in services.iter() {
                        option {
                            key: "{offered.id}",
                            value: "{offered.id}",
                            selected: offered.id == chosen.id,
                            if offered.experimental {
                                "{crate::forms::text(&offered.name)} ({i18n::t(\"experimental\")})"
                            } else {
                                "{crate::forms::text(&offered.name)}"
                            }
                        }
                    }
                }

                crate::forms::schema_form::SchemaForm {
                    fields: fields.clone(),
                    values: answered,
                    problems,
                    on_change: move |answer: api::FieldValue| {
                        let secret = fields
                            .iter()
                            .any(|field| {
                                field.key == answer.key
                                    && matches!(field.kind, api::FieldKind::Secret)
                            });
                        let mut held = if secret { secrets } else { values };
                        let mut held = held.write();
                        match held.iter_mut().find(|value| value.key == answer.key) {
                            Some(existing) => existing.value = answer.value,
                            None => held.push(answer),
                        }
                    },
                }

                div { class: "actions",
                    button {
                        onclick: move |_| on_close.call(()),
                        "{i18n::t(\"cancel\")}"
                    }
                    button {
                        disabled: blocked,
                        onclick: move |_| on_save.call(()),
                        "{i18n::t(\"save\")}"
                    }
                }
            }
        }
    }
}
#[component]
pub fn LoginPopup(
    mut username: Signal<String>,
    mut password: Signal<String>,
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
