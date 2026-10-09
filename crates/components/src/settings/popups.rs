use dioxus::prelude::*;

/// Flathub does not allow `org.freedesktop.Flatpak` in the manifest, so the
/// user has to grant it before the daemon can reach a browser on the host.
#[component]
pub fn HostAccessWarning(message: String) -> Element {
    let command = "flatpak override --user --talk-name=org.freedesktop.Flatpak moe.kopuz.kopuz";
    rsx! {
        div { class: "host-access-warning",
            p { "{message}" }
            button {
                class: "flatpak-command",
                title: i18n::t("copy"),
                aria_label: i18n::t("copy"),
                onclick: move |_| {
                    let js = format!(
                        "navigator.clipboard.writeText('{command}').catch((e) => console.error('clipboard writeText failed', e));"
                    );
                    let _ = dioxus::document::eval(&js);
                },
                "{command}"
            }
        }
    }
}

/// Add a server: pick a service, fill in whatever form the daemon publishes
/// for it. Nothing here knows what any of those fields mean.
#[component]
pub fn AddSourcePopup(
    services: Vec<api::ServiceInfo>,
    service: Signal<String>,
    name: Signal<String>,
    values: Signal<Vec<api::FieldValue>>,
    secrets: Signal<Vec<api::FieldValue>>,
    check: Option<api::DraftCheck>,
    host_access: bool,
    error: Signal<Option<String>>,
    /// Browser profiles already signed in to the chosen service; `None`
    /// while they are being looked for.
    sessions: Option<Vec<api::BrowserSession>>,
    /// The session being taken, while it is.
    importing: Option<String>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
    on_import: EventHandler<api::BrowserSession>,
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

    let mut answered = values();
    answered.extend(secrets());

    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                h2 { "{i18n::t(\"add_source\")}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                if blocked {
                    HostAccessWarning { message: i18n::t("browser_sign_in_needs_host").to_string() }
                }

                input {
                    placeholder: "{i18n::t(\"source_name\")}",
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

                // Only a service signed in through a browser can have any, so
                // nothing is held open for the others while they are asked.
                if sessions.as_ref().map_or(needs_browser, |found| !found.is_empty()) {
                    BrowserSessions {
                        sessions,
                        importing,
                        disabled: blocked,
                        on_import,
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
/// Profiles in browsers on this machine that already hold a session, each
/// one click from being used instead of signing in again. `None` is still
/// looking: every profile is checked with the service first, which takes a
/// few seconds.
#[component]
pub fn BrowserSessions(
    sessions: Option<Vec<api::BrowserSession>>,
    importing: Option<String>,
    disabled: bool,
    on_import: EventHandler<api::BrowserSession>,
) -> Element {
    let busy = importing.is_some();
    let looking = sessions.is_none();
    rsx! {
        section {
            class: "flex flex-col gap-2",
            aria_labelledby: "browser-sessions-heading",
            aria_busy: looking,
            h3 {
                id: "browser-sessions-heading",
                class: "app-section-label text-xs font-semibold uppercase tracking-wider text-white/50",
                if looking {
                    "{i18n::t(\"browser_sessions_checking\")}"
                } else {
                    "{i18n::t(\"browser_sessions_heading\")}"
                }
            }
            match sessions {
                None => rsx! {
                    div {
                        class: "app-list-item flex items-center gap-3 w-full rounded-lg border border-white/10 bg-white/5 px-3 py-2.5 animate-pulse",
                        aria_hidden: "true",
                        i { class: "fa-solid fa-user text-white/20 text-sm shrink-0" }
                        span { class: "h-3 w-40 rounded bg-white/10" }
                    }
                },
                Some(found) => rsx! {
                    for session in found.into_iter() {
                        button {
                            key: "{session.id}",
                            r#type: "button",
                            class: "app-list-item flex items-center gap-3 w-full text-left rounded-lg border border-white/10 bg-white/5 px-3 py-2.5 hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer",
                            disabled: disabled || busy,
                            onclick: {
                                let session = session.clone();
                                move |_| on_import.call(session.clone())
                            },
                            i { class: "fa-solid fa-user text-white/40 text-sm shrink-0", aria_hidden: "true" }
                            span { class: "flex flex-col min-w-0 flex-1",
                                span { class: "text-sm text-white truncate", "{session_label(&session)}" }
                                if let Some(account) = session.account.as_ref() {
                                    span { class: "text-xs text-white/50 truncate", "{account}" }
                                }
                            }
                            span { class: "text-sm font-medium text-indigo-400 shrink-0",
                                if importing.as_deref() == Some(session.id.as_str()) {
                                    "{i18n::t(\"browser_session_importing\")}"
                                } else {
                                    "{i18n::t(\"browser_session_use\")}"
                                }
                            }
                        }
                    }
                    p { class: "text-xs text-white/50", "{i18n::t(\"browser_session_help\")}" }
                },
            }
        }
    }
}

/// How a browser profile is named to the user: its browser, then its own name.
pub fn session_label(session: &api::BrowserSession) -> String {
    i18n::t_with(
        "browser_session_name",
        &[
            ("browser", session.browser.clone()),
            ("profile", session.profile.clone()),
        ],
    )
}

/// Signing a source in again: the browser profiles that already hold a
/// session for it, or a fresh browser sign-in.
#[component]
pub fn BrowserSignInPopup(
    service_name: String,
    sessions: Option<Vec<api::BrowserSession>>,
    importing: Option<String>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_browser: EventHandler<()>,
    on_import: EventHandler<api::BrowserSession>,
) -> Element {
    let busy = importing.is_some();
    rsx! {
        div {
            class: "overlay",
            onclick: move |_| if !busy { on_close.call(()) },

            div {
                class: "popup",
                role: "dialog",
                aria_modal: "true",
                onclick: |e| e.stop_propagation(),

                h2 { "{i18n::t_with(\"login_to_service\", &[(\"service\", service_name.clone())])}" }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                BrowserSessions {
                    sessions,
                    importing,
                    disabled: false,
                    on_import,
                }

                div { class: "actions",
                    button {
                        disabled: busy,
                        onclick: move |_| on_close.call(()),
                        "{i18n::t(\"cancel\")}"
                    }
                    button {
                        disabled: busy,
                        onclick: move |_| on_browser.call(()),
                        "{i18n::t(\"sign_in_with_browser\")}"
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
