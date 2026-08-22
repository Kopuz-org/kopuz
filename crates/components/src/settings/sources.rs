//! Local-library and remote-server settings controls.

use config::{MusicService, SavedLocalSource, SavedServer};
use dioxus::prelude::*;
#[cfg(not(target_os = "android"))]
use rfd::AsyncFileDialog;

#[component]
pub fn MultiDirectoryPicker(
    current_paths: Vec<std::path::PathBuf>,
    on_add: EventHandler<std::path::PathBuf>,
    on_remove: EventHandler<usize>,
) -> Element {
    let add_text = i18n::t("add_folder");
    let remove_text = i18n::t("remove");
    let no_folders_text = i18n::t("no_music_folders");

    rsx! {
        div { class: "flex flex-col gap-2 w-full",
            if current_paths.is_empty() {
                p { class: "text-xs text-slate-500 italic", "{no_folders_text}" }
            }
            for (i, path) in current_paths.iter().enumerate() {
                {
                    let display = path.display().to_string();
                    let row_key = format!("{i}-{display}");
                    rsx! {
                        div { key: "{row_key}",
                            class: "flex items-center justify-between gap-3 bg-white/5 p-2 rounded w-full",
                            span {
                                class: "text-xs text-slate-400 font-mono truncate flex-1",
                                "{display}"
                            }
                            button {
                                onclick: move |_| {
                                    on_remove.call(i);
                                },
                                class: "text-red-400 hover:text-red-300 text-xs px-2 py-0.5 rounded transition-colors shrink-0",
                                "{remove_text}"
                            }
                        }
                    }
                }
            }
            AddFolderButton { on_add, add_text }
        }
    }
}

#[component]
pub fn LocalSourceSettings(
    active_source: config::Source,
    default_directories: Vec<std::path::PathBuf>,
    sources: Vec<SavedLocalSource>,
    on_add: EventHandler<()>,
    on_delete: EventHandler<String>,
    on_switch: EventHandler<config::Source>,
    on_add_folder: EventHandler<(config::Source, std::path::PathBuf)>,
    on_remove_folder: EventHandler<(config::Source, usize)>,
) -> Element {
    let default_active = active_source == config::Source::Local;
    rsx! {
        div { class: "flex flex-col gap-3 w-full",
            div { class: "bg-white/5 p-3 rounded w-full space-y-2",
                div { class: "flex items-center justify-between gap-3",
                    div { class: "min-w-0 flex items-center gap-2",
                        p { class: "text-sm font-medium text-white truncate", "{i18n::t(\"local\")}" }
                        if default_active {
                            span { class: "text-[10px] px-2 py-0.5 rounded bg-indigo-500/30 text-indigo-200",
                                "{i18n::t(\"active_local_library\")}"
                            }
                        }
                    }
                    if !default_active {
                        button {
                            onclick: move |_| on_switch.call(config::Source::Local),
                            class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-1 rounded text-white transition-colors",
                            "{i18n::t(\"switch_to_local_library\")}"
                        }
                    }
                }
                MultiDirectoryPicker {
                    current_paths: default_directories,
                    on_add: move |path| on_add_folder.call((config::Source::Local, path)),
                    on_remove: move |index| on_remove_folder.call((config::Source::Local, index)),
                }
            }
            for source in sources.iter().cloned() {
                {
                    let id = source.id.clone();
                    let id_delete = id.clone();
                    let source_key = config::Source::LocalLibrary(id.clone());
                    let switch_key = source_key.clone();
                    let add_folder_key = source_key.clone();
                    let remove_folder_key = source_key.clone();
                    let is_active = active_source.local_library_id() == Some(source.id.as_str());
                    rsx! {
                        div { key: "{source.id}", class: "bg-white/5 p-3 rounded w-full space-y-2",
                            div { class: "flex items-center justify-between gap-3",
                                div { class: "min-w-0 flex items-center gap-2",
                                    p { class: "text-sm font-medium text-white truncate", "{source.name}" }
                                    if is_active {
                                        span { class: "text-[10px] px-2 py-0.5 rounded bg-indigo-500/30 text-indigo-200",
                                            "{i18n::t(\"active_local_library\")}"
                                        }
                                    }
                                }
                                div { class: "flex items-center gap-2 shrink-0",
                                    if !is_active {
                                        button {
                                            onclick: move |_| on_switch.call(switch_key.clone()),
                                            class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-1 rounded text-white transition-colors",
                                            "{i18n::t(\"switch_to_local_library\")}"
                                        }
                                    }
                                    button {
                                        onclick: move |_| on_delete.call(id_delete.clone()),
                                        class: "text-red-400 hover:text-red-300 text-sm px-2 py-1 transition-colors",
                                        "{i18n::t(\"delete\")}"
                                    }
                                }
                            }
                            MultiDirectoryPicker {
                                current_paths: source.directories.clone(),
                                on_add: move |path| on_add_folder.call((add_folder_key.clone(), path)),
                                on_remove: move |index| on_remove_folder.call((remove_folder_key.clone(), index)),
                            }
                        }
                    }
                }
            }
            button {
                onclick: move |_| on_add.call(()),
                class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
                "{i18n::t(\"add_local_library\")}"
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
#[component]
fn AddFolderButton(on_add: EventHandler<std::path::PathBuf>, add_text: String) -> Element {
    rsx! {
        button {
            onclick: move |_| {
                spawn(async move {
                    if let Some(handle) = AsyncFileDialog::new().pick_folder().await {
                        on_add.call(handle.path().to_path_buf());
                    }
                });
            },
            class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
            "{add_text}"
        }
    }
}

// Android has no native folder dialog (rfd doesn't work), so request storage permission
// and auto-detect the system Music directory via JNI, falling back to common paths.
#[cfg(target_os = "android")]
#[component]
fn AddFolderButton(on_add: EventHandler<std::path::PathBuf>, add_text: String) -> Element {
    rsx! {
        button {
            onclick: move |_| {
                player::systemint::request_permissions();
                spawn(async move {
                    if !player::systemint::await_media_permission().await {
                        return;
                    }
                    let mut paths = Vec::new();
                    if let Some(android_music) = player::systemint::get_android_music_dir() {
                        paths.push(std::path::PathBuf::from(android_music));
                    }
                    paths.push(std::path::PathBuf::from("/storage/emulated/0/Music"));
                    paths.push(std::path::PathBuf::from("/sdcard/Music"));
                    if let Ok(home) = std::env::var("HOME") {
                        paths.push(std::path::PathBuf::from(home).join("Music"));
                    }
                    for path in paths {
                        if path.exists() {
                            on_add.call(path);
                            break;
                        }
                    }
                });
            },
            class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
            "{add_text}"
        }
    }
}

#[component]
pub fn ServerSettings(
    /// The active source's server id (`None` ⇒ Local) — the authoritative "which
    /// server is active", reactive to the sidebar source switcher too.
    active_source_id: Option<String>,
    servers: Vec<SavedServer>,
    on_add: EventHandler<()>,
    on_delete: EventHandler<String>,
    on_switch: EventHandler<String>,
    on_login: EventHandler<()>,
    /// `(id, label)` of installed browsers that can host Spotify playback,
    /// shown as a picker under a Spotify server's card.
    spotify_browsers: Vec<(String, String)>,
    /// Persisted browser choice; `None` = automatic.
    spotify_browser: Option<String>,
    on_spotify_browser: EventHandler<Option<String>>,
    /// When another Connect device is already playing, adopt it (`true`) rather
    /// than starting on this app's in-app device.
    spotify_prefer_active_device: bool,
    on_spotify_prefer_active_device: EventHandler<bool>,
    /// Folder picker for the active server, when it browses a folder tree. Only
    /// the active server has its creds hydrated, so only it can be browsed.
    remote_folders: Option<crate::settings_remote_folders::RemoteFolderSettings>,
) -> Element {
    let login_text = i18n::t("login");
    let delete_text = i18n::t("delete");
    let switch_text = i18n::t("switch_to_server");
    let active_text = i18n::t("active_server");
    let conn = hooks::source_switch::use_connection_status();

    rsx! {
        div { class: "flex flex-col gap-2 w-full",
            if servers.is_empty() {
                p { class: "text-xs text-white/50 italic", "{i18n::t(\"no_saved_servers\")}" }
            }
            for srv in servers.iter().cloned() {
                {
                    let id = srv.id.clone();
                    let is_active = active_source_id.as_deref() == Some(srv.id.as_str());
                    let id_switch = id.clone();
                    let id_delete = id.clone();
                    let is_spotify = srv.service == MusicService::Spotify;
                    // Folders are the whole library definition here, so the
                    // picker sits on the card the way it does for a local
                    // library, not behind a separate dialog.
                    let picker = is_active.then(|| remote_folders.clone()).flatten();
                    let browsers = spotify_browsers.clone();
                    let chosen = spotify_browser.clone();
                    let prefer_active = spotify_prefer_active_device;
                    rsx! {
                        div { key: "{srv.id}",
                            class: "flex flex-col gap-2 bg-white/5 p-2 rounded w-full",
                            div { class: "flex items-center justify-between gap-4 w-full",
                            div { class: "min-w-0 flex-1",
                                div { class: "flex items-center gap-2",
                                    p { class: "text-sm font-medium text-white truncate", "{srv.name}" }
                                    if is_active {
                                        span { class: "text-[10px] px-2 py-0.5 rounded bg-indigo-500/30 text-indigo-200",
                                            "{active_text}"
                                        }
                                    }
                                }
                                p { class: "text-xs text-white/60", "{i18n::t_with(\"service\", &[(\"name\", srv.service.display_name().to_string())])}" }
                                p { class: "text-xs text-white/60 truncate", "{srv.url}" }
                                if is_active {
                                    match conn() {
                                        hooks::source_switch::ConnStatus::Online => rsx! {
                                            p { class: "text-xs mt-1", style: "color:#3fb950", "{i18n::t(\"connected\")}" }
                                        },
                                        hooks::source_switch::ConnStatus::Connecting => rsx! {
                                            p { class: "text-xs mt-1", style: "color:#d8a23a", "{i18n::t(\"connecting\")}" }
                                        },
                                        hooks::source_switch::ConnStatus::Offline => rsx! {
                                            div { class: "flex items-center gap-2 mt-1",
                                                p { class: "text-xs", style: "color:#e5534b", "{i18n::t(\"disconnected\")}" }
                                                button {
                                                    onclick: move |_| on_login.call(()),
                                                    class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-0.5 rounded text-white transition-colors",
                                                    "{login_text}"
                                                }
                                            }
                                        },
                                    }
                                }
                            }
                            div { class: "flex items-center gap-2 shrink-0",
                                if !is_active {
                                    button {
                                        onclick: move |_| on_switch.call(id_switch.clone()),
                                        class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-1 rounded text-white transition-colors",
                                        "{switch_text}"
                                    }
                                }
                                button {
                                    onclick: move |_| on_delete.call(id_delete.clone()),
                                    class: "text-red-400 hover:text-red-300 text-sm px-2 py-1 transition-colors",
                                    "{delete_text}"
                                }
                            }
                            }
                            if let Some(picker) = picker {
                                div { class: "flex flex-col gap-2 border-t border-white/10 pt-2",
                                    p { class: "text-xs text-white/60", "{i18n::t(\"remote_music_folders\")}" }
                                    crate::settings_remote_folders::RemoteFolderPicker { settings: picker }
                                }
                            }
                            if is_spotify {
                                div { class: "flex items-center justify-between gap-4 border-t border-white/10 pt-2",
                                    p { class: "text-xs text-white/60", "{i18n::t(\"spotify_browser\")}" }
                                    select {
                                        class: "bg-stone-800 text-white rounded px-2 py-1 text-xs border border-white/10 focus:outline-none focus:border-indigo-500",
                                        onchange: move |evt| {
                                            let v = evt.value();
                                            on_spotify_browser.call((v != "auto").then_some(v));
                                        },
                                        option {
                                            value: "auto",
                                            selected: chosen.is_none(),
                                            "{i18n::t(\"spotify_browser_auto\")}"
                                        }
                                        for (bid, label) in browsers.iter() {
                                            option {
                                                value: "{bid}",
                                                selected: chosen.as_deref() == Some(bid.as_str()),
                                                "{label}"
                                            }
                                        }
                                    }
                                }
                                div { class: "flex items-center justify-between gap-4 border-t border-white/10 pt-2",
                                    p { class: "text-xs text-white/60", "{i18n::t(\"spotify_connect_device\")}" }
                                    select {
                                        class: "bg-stone-800 text-white rounded px-2 py-1 text-xs border border-white/10 focus:outline-none focus:border-indigo-500",
                                        onchange: move |evt| {
                                            on_spotify_prefer_active_device.call(evt.value() == "other");
                                        },
                                        option {
                                            value: "other",
                                            selected: prefer_active,
                                            "{i18n::t(\"spotify_connect_device_other\")}"
                                        }
                                        option {
                                            value: "this",
                                            selected: !prefer_active,
                                            "{i18n::t(\"spotify_connect_device_this\")}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            button {
                onclick: move |_| on_add.call(()),
                class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
                "{i18n::t(\"add_server\")}"
            }
        }
    }
}
