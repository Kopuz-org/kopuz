//! One renderer for every published field list.
//!
//! A source's sign-in form, an integration's credentials and the downloader's
//! options all arrive as [`api::FieldSpec`]s and go back as
//! [`api::FieldValue`]s, so this is the only place that decides what a control
//! looks like -- and nothing here knows what any of the fields are for.

use dioxus::prelude::*;

use crate::settings::items::{AppSelect, SettingItem, ToggleSetting};

/// The value a field currently holds: what the caller has typed, falling back
/// to what the daemon published.
fn current(values: &[api::FieldValue], field: &api::FieldSpec) -> String {
    api::value_of(values, &field.key)
        .map(str::to_string)
        .or_else(|| field.value.clone())
        .unwrap_or_default()
}

/// Whether a row applies, given what the other fields hold.
fn applies(values: &[api::FieldValue], fields: &[api::FieldSpec], field: &api::FieldSpec) -> bool {
    let Some(condition) = field.show_when.as_ref() else {
        return true;
    };
    let holder = fields.iter().find(|other| other.key == condition.key);
    let held = holder
        .map(|holder| current(values, holder))
        .unwrap_or_default();
    held == condition.value
}

/// The option list for a choice, plus a "custom" entry when the daemon allows
/// a value outside it -- an Apple Music storefront it does not list, say.
fn options(field: &api::FieldSpec, chosen: &str) -> (Vec<(String, String)>, bool) {
    let api::FieldKind::Choice { options, custom } = &field.kind else {
        return (Vec::new(), false);
    };
    let mut listed: Vec<(String, String)> = options
        .iter()
        .map(|option| (option.value.clone(), super::text(&option.label)))
        .collect();
    let known = options.iter().any(|option| option.value == chosen);
    if *custom {
        listed.push((CUSTOM.to_string(), i18n::t("custom_manual")));
    }
    (listed, *custom && !known)
}

/// The sentinel a choice takes when the answer is typed rather than picked.
const CUSTOM: &str = "__custom";

const TEXT_INPUT: &str =
    "bg-transparent w-full px-3 py-2 text-sm text-white placeholder:text-white/50 outline-none";
const TEXT_WRAP: &str = "flex-1 bg-white/5 p-1 rounded-xl border border-white/5";
/// Render a published field list. `values` holds every answer so far, and
/// `on_change` is called with each one as it is edited.
#[component]
pub fn SchemaForm(
    fields: Vec<api::FieldSpec>,
    values: Vec<api::FieldValue>,
    #[props(default)] problems: Vec<api::Problem>,
    on_change: EventHandler<api::FieldValue>,
) -> Element {
    let shown: Vec<api::FieldSpec> = fields
        .iter()
        .filter(|field| applies(&values, &fields, field))
        .cloned()
        .collect();
    // A heading belongs to the first row of its group, so it is only drawn
    // when the group actually changes -- a hidden row must not eat it.
    let mut section: Option<String> = None;
    let mut rows: Vec<(api::FieldSpec, Option<String>)> = Vec::with_capacity(shown.len());
    for field in shown {
        let heading = field
            .section
            .as_ref()
            .map(super::text)
            .filter(|title| Some(title) != section.as_ref());
        if let Some(title) = heading.clone() {
            section = Some(title);
        }
        rows.push((field, heading));
    }

    rsx! {
        div { class: "flex flex-col w-full",
            for problem in problems.iter().filter(|problem| problem.field.is_none()) {
                p { class: "text-xs text-red-400 px-5 py-1", "{super::text(&problem.label)}" }
            }
            for (field , heading) in rows.into_iter() {
                Field {
                    key: "{field.key}",
                    problem: problems
                        .iter()
                        .find(|problem| problem.field.as_deref() == Some(field.key.as_str()))
                        .map(|problem| super::text(&problem.label)),
                    value: current(&values, &field),
                    heading,
                    field,
                    on_change,
                }
            }
        }
    }
}

#[component]
fn Field(
    field: api::FieldSpec,
    value: String,
    heading: Option<String>,
    problem: Option<String>,
    on_change: EventHandler<api::FieldValue>,
) -> Element {
    let title = super::text(&field.label);
    let help = field.help.as_ref().map(super::text);
    let placeholder = field
        .placeholder
        .as_ref()
        .map(super::text)
        .unwrap_or_default();

    // A note is only its help text, so it gets no row and no label.
    if matches!(field.kind, api::FieldKind::Note) {
        let text = help.unwrap_or(title);
        return rsx! {
            p { class: "text-xs text-white/60 px-5 py-2", "{text}" }
        };
    }

    let control = match field.kind.clone() {
        api::FieldKind::Toggle => {
            let key = field.key.clone();
            rsx! {
                ToggleSetting {
                    enabled: value == "true",
                    on_change: move |on: bool| {
                        on_change.call(api::FieldValue::new(key.clone(), on.to_string()));
                    },
                }
            }
        }
        api::FieldKind::Choice { .. } => {
            let (listed, typed) = options(&field, &value);
            let shown = if typed {
                CUSTOM.to_string()
            } else {
                value.clone()
            };
            let free = value.clone();
            let pick_key = field.key.clone();
            let type_key = field.key.clone();
            rsx! {
                div { class: "flex flex-col gap-2 flex-1 items-end",
                    AppSelect {
                        value: shown,
                        options: listed,
                        on_change: move |picked: String| {
                            let answer = if picked == CUSTOM { String::new() } else { picked };
                            on_change.call(api::FieldValue::new(pick_key.clone(), answer));
                        },
                    }
                    if typed {
                        div { class: "{TEXT_WRAP}",
                            input {
                                class: "{TEXT_INPUT}",
                                placeholder: "{placeholder}",
                                value: "{free}",
                                onkeydown: move |e| e.stop_propagation(),
                                oninput: move |e| {
                                    on_change.call(api::FieldValue::new(type_key.clone(), e.value()));
                                },
                            }
                        }
                    }
                }
            }
        }
        api::FieldKind::Radio { options } => rsx! {
            div { class: "flex flex-col gap-2",
                for option in options.into_iter() {
                    label {
                        key: "{option.value}",
                        class: "flex items-center gap-2 text-sm text-white cursor-pointer",
                        input {
                            r#type: "radio",
                            name: "{field.key}",
                            checked: value == option.value,
                            onchange: {
                                let key = field.key.clone();
                                let picked = option.value.clone();
                                move |_| on_change.call(api::FieldValue::new(key.clone(), picked.clone()))
                            },
                        }
                        span { "{super::text(&option.label)}" }
                    }
                }
            }
        },
        api::FieldKind::Directory => {
            let typed_key = field.key.clone();
            let picked_key = field.key.clone();
            rsx! {
                div { class: "flex items-center gap-2 flex-1",
                    div { class: "{TEXT_WRAP}",
                        input {
                            class: "{TEXT_INPUT}",
                            placeholder: "{placeholder}",
                            value: "{value}",
                            onkeydown: move |e| e.stop_propagation(),
                            oninput: move |e| {
                                on_change.call(api::FieldValue::new(typed_key.clone(), e.value()));
                            },
                        }
                    }
                    DirectoryButton {
                        on_pick: move |path: String| {
                            on_change.call(api::FieldValue::new(picked_key.clone(), path));
                        },
                    }
                }
            }
        }
        kind => {
            let secret = matches!(kind, api::FieldKind::Secret);
            let key = field.key.clone();
            rsx! {
                div { class: "{TEXT_WRAP}",
                    input {
                        class: "{TEXT_INPUT}",
                        r#type: if secret { "password" } else { "text" },
                        placeholder: "{placeholder}",
                        value: "{value}",
                        onkeydown: move |e| e.stop_propagation(),
                        oninput: move |e| {
                            on_change.call(api::FieldValue::new(key.clone(), e.value()));
                        },
                    }
                }
            }
        }
    };

    let stacked = matches!(field.kind, api::FieldKind::Radio { .. });
    let config_key = field.config_key.clone().unwrap_or_default();
    rsx! {
        if let Some(heading) = heading {
            h3 {
                class: "text-xs font-semibold uppercase tracking-wider text-white/50 px-5 pt-4 pb-1",
                "{heading}"
            }
        }
        SettingItem { title, control, config_key, stacked }
        if let Some(help) = help {
            p { class: "text-xs text-white/50 px-5 pb-2 -mt-1", "{help}" }
        }
        if let Some(problem) = problem {
            p { class: "text-xs text-red-400 px-5 pb-2 -mt-1", "{problem}" }
        }
    }
}

/// Pick a directory with whatever the platform offers. Android has no folder
/// dialog, so there the path is typed.
#[cfg(not(target_os = "android"))]
#[component]
fn DirectoryButton(on_pick: EventHandler<String>) -> Element {
    rsx! {
        button {
            class: "bg-white/10 hover:bg-white/20 px-3 py-2 rounded-xl text-sm text-white transition-colors shrink-0",
            onclick: move |_| {
                spawn(async move {
                    if let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await {
                        on_pick.call(handle.path().display().to_string());
                    }
                });
            },
            i { class: "fa-solid fa-folder-open text-sm" }
        }
    }
}

#[cfg(target_os = "android")]
#[component]
fn DirectoryButton(on_pick: EventHandler<String>) -> Element {
    let _ = on_pick;
    rsx! {}
}
