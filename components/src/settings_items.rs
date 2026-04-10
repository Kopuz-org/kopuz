use config::MusicServer;
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
            optgroup { label: "── Dynamic ──",
                option { value: "album-art", "Album Art Gradient" }
            }
            optgroup { label: "── Dark ──",
                option { value: "default", "Default" }
                option { value: "gruvbox", "Gruvbox Material" }
                option { value: "gruvbox-classic", "Gruvbox Classic" }
                option { value: "gruvbox-dark-soft", "Gruvbox Dark Soft" }
                option { value: "dracula", "Dracula" }
                option { value: "nord", "Nord" }
                option { value: "catppuccin", "Catppuccin Mocha" }
                option { value: "ef-night", "Ef Night" }
                option { value: "ayu-dark", "Ayu Dark" }
                option { value: "ayu-mirage", "Ayu Mirage" }
                option { value: "vague", "Vague" }
                option { value: "onedarkpro", "One Dark Pro" }
                option { value: "osmium", "Osmium" }
                option { value: "kanagawa-dragon", "Kanagawa Dragon" }
                option { value: "everforest", "Everforest" }
                option { value: "rosepine", "Rosé Pine" }
                option { value: "kettek16", "kettek16" }
            }
            optgroup { label: "── Light ──",
                option { value: "default-light", "Default Light" }
                option { value: "catppuccin-latte", "Catppuccin Latte" }
                option { value: "rosepine-dawn", "Rosé Pine Dawn" }
                option { value: "everforest-light", "Everforest Light" }
                option { value: "ayu-light", "Ayu Light" }
                option { value: "one-light", "One Light" }
                option { value: "gruvbox-light", "Gruvbox Light Soft" }
            }
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
    server: Option<MusicServer>,
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
                        p { class: "text-xs text-white/60", "Service: {server.service.display_name()}" }
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

#[component]
pub fn DiscordPresenceSettings(enabled: bool, on_change: EventHandler<bool>) -> Element {
    let slider_style = if enabled {
        "left: 4px; width: calc(50% - 4px);"
    } else {
        "left: calc(50% + 2px); width: calc(50% - 4px);"
    };

    let enable_class = if enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    let disable_class = if !enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    rsx! {
        div {
            class: "bg-white/5 p-1 rounded-xl flex relative h-10 items-center border border-white/5 w-48",
            div {
                class: "absolute h-8 bg-white/10 rounded-lg transition-all duration-300 ease-out",
                style: "{slider_style}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {enable_class}",
                onclick: move |_| on_change.call(true),
                "ENABLED"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {disable_class}",
                onclick: move |_| on_change.call(false),
                "DISABLED"
            }
        }
    }
}

#[component]
pub fn ToggleSetting(enabled: bool, on_change: EventHandler<bool>) -> Element {
    let slider_style = if enabled {
        "left: 4px; width: calc(50% - 4px);"
    } else {
        "left: calc(50% + 2px); width: calc(50% - 4px);"
    };

    let enable_class = if enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    let disable_class = if !enabled {
        "text-white"
    } else {
        "text-slate-500 hover:text-slate-300"
    };

    rsx! {
        div {
            class: "bg-white/5 p-1 rounded-xl flex relative h-10 items-center border border-white/5 w-48",
            div {
                class: "absolute h-8 bg-white/10 rounded-lg transition-all duration-300 ease-out",
                style: "{slider_style}"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {enable_class}",
                onclick: move |_| on_change.call(true),
                "ENABLED"
            }
            button {
                class: "flex-1 text-[11px] font-bold z-10 transition-colors duration-300 cursor-pointer {disable_class}",
                onclick: move |_| on_change.call(false),
                "DISABLED"
            }
        }
    }
}

#[component]
pub fn MusicBrainzSettings(current: String, on_save: EventHandler<String>) -> Element {
    let mut input = use_signal(move || current.clone());

    rsx! {
        div {
            class: "flex items-center gap-2 w-full max-w-xl",
            div {
                class: "flex-1 bg-white/5 p-1 rounded-xl border border-white/5",
                input {
                    class: "bg-transparent w-full px-3 py-2 text-sm text-white placeholder:text-white/50 outline-none",
                    placeholder: "Enter your ListenBrainz token",
                    value: "{input()}",
                    oninput: move |evt| {
                        input.set(evt.value());
                        on_save.call(evt.value());
                    },
                    r#type: "password",
                }
            }
        }
    }
}

// #[component]
// pub fn LastFmSettings(current: String, on_save: EventHandler<String>) -> Element {
//     let mut input = use_signal(move || current.clone());

//     rsx! {
//         div { class: "flex items-center gap-2 w-full max-w-xl",
//             div { class: "flex-1 bg-white/5 p-1 rounded-xl border border-white/5",
//                 input {
//                     class: "bg-transparent w-full px-3 py-2 text-sm text-white placeholder:text-white/50 outline-none",
//                     placeholder: "Enter your last.fm token",
//                     value: "{input()}",
//                     oninput: move |evt| {
//                         input.set(evt.value());
//                         on_save.call(evt.value());
//                     },
//                     r#type: "text",
//                 }
//             }
//         }
//     }
// }
