//! One popup that renders any plugin-authored sign-in step.
//!
//! Every label, field and message comes from the [`AuthPrompt`] the plugin
//! sent, so this component contains no provider vocabulary and needs no change
//! when a new kind of plugin appears. The host loop feeds it prompts and posts
//! back whatever the user entered until the plugin says `Done` or `Failed`.

use std::collections::HashMap;

use dioxus::prelude::*;
use server::plugin::AuthPrompt;

#[component]
pub fn PluginAuthPopup(
    /// The plugin's display name, for the popup heading.
    plugin_name: String,
    prompt: AuthPrompt,
    /// True while the plugin is working on the last submission.
    busy: bool,
    /// Field values for a `Form` prompt; empty for every other kind.
    on_submit: EventHandler<HashMap<String, String>>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut values = use_signal(HashMap::<String, String>::new);
    let cancel_text = i18n::t("cancel").to_string();
    let continue_text = i18n::t("continue_action").to_string();

    rsx! {
        div { class: "overlay", onclick: move |_| on_cancel.call(()),
            div { class: "popup", onclick: |e| e.stop_propagation(),
                h2 { "{plugin_name}" }

                match &prompt {
                    AuthPrompt::OpenUrl { url, message } => {
                        let url = url.clone();
                        rsx! {
                            p { class: "text-sm text-white/80", "{message}" }
                            button {
                                onclick: move |_| {
                                    if let Err(e) = webbrowser::open(&url) {
                                        tracing::warn!(error = %e, "cannot open the plugin sign-in URL");
                                    }
                                },
                                "{i18n::t(\"plugin_open_sign_in\")}"
                            }
                        }
                    }
                    AuthPrompt::Form { title, fields } => rsx! {
                        p { class: "text-sm text-white/80", "{title}" }
                        for field in fields.iter().cloned() {
                            {
                                let key = field.key.clone();
                                rsx! {
                                    input {
                                        key: "{field.key}",
                                        r#type: if field.secret { "password" } else { "text" },
                                        placeholder: "{field.label}",
                                        value: "{values().get(&field.key).cloned().unwrap_or_default()}",
                                        oninput: move |e| {
                                            values.write().insert(key.clone(), e.value());
                                        },
                                        onkeydown: move |e| e.stop_propagation(),
                                    }
                                }
                            }
                        }
                    },
                    AuthPrompt::Message { text } => rsx! {
                        p { class: "text-sm text-white/80", "{text}" }
                    },
                    AuthPrompt::Failed { message } => rsx! {
                        p { class: "error", "{message}" }
                    },
                    // The host closes the popup on `Done`; rendering it at all
                    // only happens for the frame in between.
                    AuthPrompt::Done => rsx! {
                        p { class: "text-sm text-white/80", "{i18n::t(\"plugin_signed_in\")}" }
                    },
                }

                div { class: "actions",
                    button { onclick: move |_| on_cancel.call(()), "{cancel_text}" }
                    button {
                        disabled: busy,
                        onclick: move |_| on_submit.call(values.peek().clone()),
                        if busy {
                            "{i18n::t(\"plugin_working\")}"
                        } else if matches!(prompt, AuthPrompt::Failed { .. }) {
                            "{i18n::t(\"plugin_retry\")}"
                        } else {
                            "{continue_text}"
                        }
                    }
                }
            }
        }
    }
}
