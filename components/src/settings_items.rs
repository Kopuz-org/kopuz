use config::JellyfinServer;
use dioxus::prelude::*;
use rfd::AsyncFileDialog;

#[component]
pub fn SettingItem(title: &'static str, description: String, control: Element) -> Element {
    rsx! {
        div { class: "flex items-center justify-between py-2",
            div {
                p { class: "text-white font-medium", "{title}" }
                p { class: "text-sm text-slate-500", "{description}" }
            }
            {control}
        }
    }
}

#[component]
pub fn ThemeSelector(current_theme: String, on_change: EventHandler<String>) -> Element {
    rsx! {
        select {
            class: "bg-white/5 border border-white/10 rounded px-3 py-1 text-sm text-white focus:outline-none focus:border-white/20",
            value: "{current_theme}",
            onchange: move |evt| on_change.call(evt.value()),
            option { value: "default", "Default" }
            option { value: "gruvbox", "Gruvbox Material" }
            option { value: "dracula", "Dracula" }
            option { value: "nord", "Nord" }
            option { value: "catppuccin", "Catppuccin Mocha" }
        }
    }
}

#[component]
pub fn DirectoryPicker(on_change: EventHandler<std::path::PathBuf>) -> Element {
    rsx! {
        button {
            onclick: move |_| {
                spawn(async move {
                    if let Some(handle) = AsyncFileDialog::new().pick_folder().await {
                        let path = handle.path().to_path_buf();
                        on_change.call(path);
                    }
                });
            },
            class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors",
            "Change"
        }
    }
}

#[component]
pub fn ServerSettings(
    server: Option<JellyfinServer>,
    on_add: EventHandler<()>,
    on_delete: EventHandler<()>,
    on_login: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "flex flex-col gap-2",
            if let Some(server) = server {
                div { class: "flex items-center justify-between gap-4 bg-white/5 p-2 rounded w-full",
                    div {
                        p { class: "text-sm font-medium text-white", "{server.name}" }
                        p { class: "text-xs text-white/60", "{server.url}" }
                        if server.access_token.is_some() {
                            p { class: "text-xs text-green-400 mt-1", "● Connected" }
                        } else {
                            div { class: "flex items-center gap-2 mt-1",
                                p { class: "text-xs text-red-400", "● Disconnected" }
                                button {
                                    onclick: move |_| on_login.call(()),
                                    class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-0.5 rounded text-white transition-colors",
                                    "Login"
                                }
                            }
                        }
                    }
                    button {
                        onclick: move |_| on_delete.call(()),
                        class: "text-red-400 hover:text-red-300 text-sm px-2 py-1 transition-colors",
                        "Delete"
                    }
                }
            } else {
                button {
                    onclick: move |_| on_add.call(()),
                    class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
                    "Add Server"
                }
            }
        }
    }
}
