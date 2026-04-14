use config::{AppConfig, CustomTheme};
use dioxus::prelude::*;
use std::collections::HashMap;

const VAR_LABELS: &[(&str, &str)] = &[
    ("bg", "Background"),
    ("raised", "Raised Surface"),
    ("surface", "Surface"),
    ("text", "Text"),
    ("text-muted", "Text Muted"),
    ("accent", "Accent"),
    ("accent-soft", "Accent Soft"),
    ("accent-alt", "Accent Alt"),
    ("accent-deep", "Accent Deep"),
    ("highlight", "Highlight"),
    ("highlight-dark", "Highlight Dark"),
    ("progress", "Progress"),
    ("danger", "Danger"),
];

const DEFAULT_VARS: &[(&str, &str)] = &[
    ("bg", "#0f0f17"),
    ("raised", "#1a1a2a"),
    ("surface", "#282838"),
    ("text", "#e2e2f0"),
    ("text-muted", "#7878a0"),
    ("accent", "#5f8aff"),
    ("accent-soft", "#8faeff"),
    ("accent-alt", "#3a5fd9"),
    ("accent-deep", "#0a0a1a"),
    ("highlight", "#c77dff"),
    ("highlight-dark", "#9d4edd"),
    ("progress", "#5f8aff"),
    ("danger", "#ff6b6b"),
];

fn default_vars_map() -> HashMap<String, String> {
    DEFAULT_VARS
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[component]
pub fn ThemeEditorPage(config: Signal<AppConfig>) -> Element {
    let mut selected_id: Signal<Option<String>> = use_signal(|| None);
    let mut editing_name = use_signal(String::new);
    let mut editing_vars: Signal<HashMap<String, String>> = use_signal(default_vars_map);

    // When selection changes, load the theme into the editor
    use_effect(move || {
        let id = selected_id.read().clone();
        match id {
            Some(ref id) => {
                let cfg = config.read();
                if let Some(ct) = cfg.custom_themes.get(id) {
                    editing_name.set(ct.name.clone());
                    editing_vars.set(ct.vars.clone());
                }
            }
            None => {
                editing_name.set(String::new());
                editing_vars.set(default_vars_map());
            }
        }
    });

    let themes_list: Vec<(String, String)> = {
        let mut v: Vec<(String, String)> = config
            .read()
            .custom_themes
            .iter()
            .map(|(id, ct)| (id.clone(), ct.name.clone()))
            .collect();
        v.sort_by(|a, b| a.1.cmp(&b.1));
        v
    };

    // Live preview: build the preview style string from editing_vars
    let preview_style = {
        let vars = editing_vars.read();
        let bg = vars.get("bg").cloned().unwrap_or_default();
        let raised = vars.get("raised").cloned().unwrap_or_default();
        let surface = vars.get("surface").cloned().unwrap_or_default();
        let text = vars.get("text").cloned().unwrap_or_default();
        let text_muted = vars.get("text-muted").cloned().unwrap_or_default();
        let accent = vars.get("accent").cloned().unwrap_or_default();
        let highlight = vars.get("highlight").cloned().unwrap_or_default();
        let progress = vars.get("progress").cloned().unwrap_or_default();
        let danger = vars.get("danger").cloned().unwrap_or_default();
        format!(
            "--preview-bg:{bg};--preview-raised:{raised};--preview-surface:{surface};\
             --preview-text:{text};--preview-muted:{text_muted};--preview-accent:{accent};\
             --preview-highlight:{highlight};--preview-progress:{progress};--preview-danger:{danger};"
        )
    };

    rsx! {
        div { class: "p-8 max-w-5xl pb-32",
            h1 { class: "text-3xl font-bold text-white mb-6", "Theme Editor" }

            div { class: "flex gap-6",

                // ── Left: saved themes list ──────────────────────────────
                div { class: "w-52 shrink-0 flex flex-col gap-2",
                    button {
                        class: "w-full px-3 py-2 bg-white/10 hover:bg-white/15 rounded text-sm text-white transition-colors text-left",
                        onclick: move |_| selected_id.set(None),
                        "+ New Theme"
                    }
                    div { class: "space-y-1",
                        for (id, name) in &themes_list {
                            {
                                let id = id.clone();
                                let name = name.clone();
                                let is_active = *selected_id.read() == Some(id.clone());
                                rsx! {
                                    button {
                                        key: "{id}",
                                        class: if is_active {
                                            "w-full text-left px-3 py-2 rounded text-sm bg-white/15 text-white"
                                        } else {
                                            "w-full text-left px-3 py-2 rounded text-sm text-slate-400 hover:bg-white/5 hover:text-white transition-colors"
                                        },
                                        onclick: move |_| selected_id.set(Some(id.clone())),
                                        "{name}"
                                    }
                                }
                            }
                        }
                    }
                }

                // ── Right: editor ────────────────────────────────────────
                div { class: "flex-1 flex flex-col gap-5",

                    // Name input
                    div { class: "bg-white/5 rounded-xl p-5",
                        label { class: "block text-xs text-slate-400 mb-1 uppercase tracking-wider", "Theme Name" }
                        input {
                            class: "bg-white/5 border border-white/10 rounded px-3 py-1.5 text-sm text-white w-full focus:outline-none focus:border-white/30",
                            placeholder: "My Custom Theme",
                            value: "{editing_name}",
                            oninput: move |e| editing_name.set(e.value()),
                        }
                    }

                    // Color pickers grid
                    div { class: "bg-white/5 rounded-xl p-5",
                        p { class: "text-xs text-slate-400 uppercase tracking-wider mb-4", "Colors" }
                        div { class: "grid grid-cols-2 gap-x-10 gap-y-3",
                            for (key, label) in VAR_LABELS {
                                {
                                    let key_str = key.to_string();
                                    let current = editing_vars
                                        .read()
                                        .get(*key)
                                        .cloned()
                                        .unwrap_or_else(|| "#000000".to_string());
                                    rsx! {
                                        div { class: "flex items-center justify-between",
                                            span { class: "text-sm text-slate-300", "{label}" }
                                            div { class: "flex items-center gap-2",
                                                input {
                                                    r#type: "color",
                                                    class: "w-8 h-8 rounded cursor-pointer border border-white/10 bg-transparent",
                                                    value: "{current}",
                                                    oninput: move |e| {
                                                        editing_vars.write().insert(key_str.clone(), e.value());
                                                    }
                                                }
                                                span { class: "text-xs text-slate-500 font-mono w-[4.5rem]",
                                                    "{current}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Live preview
                    div { class: "bg-white/5 rounded-xl p-5",
                        p { class: "text-xs text-slate-400 uppercase tracking-wider mb-3", "Preview" }
                        div {
                            class: "rounded-lg p-4 flex flex-col gap-2",
                            style: "background: var(--preview-bg); {preview_style}",
                            div { class: "flex items-center justify-between",
                                span {
                                    class: "text-sm font-semibold",
                                    style: "color: var(--preview-text)",
                                    "Track Title"
                                }
                                span {
                                    class: "text-xs",
                                    style: "color: var(--preview-muted)",
                                    "3:42"
                                }
                            }
                            div {
                                class: "h-1 rounded-full w-full",
                                style: "background: var(--preview-surface)",
                                div {
                                    class: "h-1 rounded-full w-2/3",
                                    style: "background: var(--preview-progress)"
                                }
                            }
                            div { class: "flex gap-2 mt-1",
                                span {
                                    class: "text-xs px-2 py-0.5 rounded-full",
                                    style: "background: var(--preview-raised); color: var(--preview-accent)",
                                    "Accent"
                                }
                                span {
                                    class: "text-xs px-2 py-0.5 rounded-full",
                                    style: "background: var(--preview-raised); color: var(--preview-highlight)",
                                    "Highlight"
                                }
                                span {
                                    class: "text-xs px-2 py-0.5 rounded-full",
                                    style: "background: var(--preview-raised); color: var(--preview-danger)",
                                    "Danger"
                                }
                            }
                        }
                    }

                    // Actions
                    div { class: "flex gap-3",
                        button {
                            class: "px-4 py-2 bg-indigo-600 hover:bg-indigo-500 rounded text-sm text-white transition-colors",
                            onclick: move |_| {
                                let name = editing_name.read().trim().to_string();
                                if name.is_empty() { return; }
                                let vars = editing_vars.read().clone();
                                let id = selected_id
                                    .read()
                                    .clone()
                                    .unwrap_or_else(|| {
                                        format!("custom-{}", name.to_lowercase().replace(' ', "-"))
                                    });
                                config.write().custom_themes.insert(id.clone(), CustomTheme { name, vars });
                                selected_id.set(Some(id));
                            },
                            "Save Theme"
                        }
                        if selected_id.peek().is_some() {
                            button {
                                class: "px-4 py-2 bg-red-500/20 hover:bg-red-500/30 rounded text-sm text-red-400 transition-colors",
                                onclick: move |_| {
                                    if let Some(id) = selected_id.write().take() {
                                        config.write().custom_themes.remove(&id);
                                    }
                                },
                                "Delete"
                            }
                        }
                    }
                }
            }
        }
    }
}
