//! Folder picker for backends whose library is a directory tree (Nextcloud over
//! WebDAV). Same list-plus-add shape as a local library, except the file dialog
//! is an in-place browser, since the folders live on the server.

use dioxus::prelude::*;
use server::nextcloud::{folder_name, parent_dir};

/// Credentials the browser needs. The picker runs from Settings, before the
/// active source is rebuilt, so it takes them rather than a `MediaSource`.
#[derive(Clone, PartialEq)]
pub struct RemoteCreds {
    pub url: String,
    pub user_id: String,
    pub token: String,
}

/// A server card's folder picker, absent unless the active server browses a
/// folder tree and is signed in. `folders` empty means the backend guesses a
/// root for itself.
#[derive(Clone, PartialEq)]
pub struct RemoteFolderSettings {
    pub creds: RemoteCreds,
    pub folders: Vec<String>,
    pub on_add: EventHandler<String>,
    pub on_remove: EventHandler<usize>,
}

#[component]
pub fn RemoteFolderPicker(settings: RemoteFolderSettings) -> Element {
    let mut browsing = use_signal(|| false);
    let mut path = use_signal(|| "/".to_string());

    let RemoteFolderSettings {
        creds,
        folders,
        on_add,
        on_remove,
    } = settings;

    let listing = use_resource(move || {
        let creds = creds.clone();
        let at = path();
        let open = browsing();
        // Closed means nothing to list, not an empty server.
        async move {
            if !open {
                return Ok(Vec::new());
            }
            server::nextcloud::browse_folders(&creds.url, &creds.user_id, &creds.token, &at).await
        }
    });

    rsx! {
        div { class: "flex flex-col gap-2 w-full",
            if folders.is_empty() {
                p { class: "text-xs text-slate-500 italic", "{i18n::t(\"no_music_folders\")}" }
            }
            for (i , folder) in folders.iter().enumerate() {
                div {
                    key: "{i}-{folder}",
                    class: "flex items-center justify-between gap-3 bg-white/5 p-2 rounded w-full",
                    span { class: "text-xs text-slate-400 font-mono truncate flex-1", "{folder}" }
                    button {
                        onclick: move |_| on_remove.call(i),
                        class: "text-red-400 hover:text-red-300 text-xs px-2 py-0.5 rounded transition-colors shrink-0",
                        "{i18n::t(\"remove\")}"
                    }
                }
            }

            if browsing() {
                div { class: "flex flex-col gap-2 bg-white/5 p-2 rounded w-full",
                    div { class: "flex items-center gap-2",
                        button {
                            onclick: move |_| path.set(parent_dir(&path())),
                            disabled: path() == "/",
                            class: "text-xs bg-white/10 hover:bg-white/20 disabled:opacity-40 px-2 py-1 rounded text-white transition-colors shrink-0",
                            "{i18n::t(\"parent_folder\")}"
                        }
                        span { class: "text-xs text-slate-400 font-mono truncate flex-1", "{path()}" }
                    }

                    div { class: "flex flex-col gap-1 max-h-56 overflow-y-auto",
                        match &*listing.read_unchecked() {
                            None => rsx! {
                                p { class: "text-xs text-slate-500 italic", "{i18n::t(\"loading_folders\")}" }
                            },
                            Some(Err(e)) => rsx! {
                                p { class: "text-xs text-red-400",
                                    "{i18n::t_with(\"folder_browse_failed\", &[(\"error\", e.clone())])}"
                                }
                            },
                            Some(Ok(dirs)) if dirs.is_empty() => rsx! {
                                p { class: "text-xs text-slate-500 italic", "{i18n::t(\"no_subfolders\")}" }
                            },
                            Some(Ok(dirs)) => rsx! {
                                for dir in dirs.clone() {
                                    button {
                                        key: "{dir}",
                                        onclick: move |_| path.set(dir.clone()),
                                        class: "flex items-center gap-2 text-left text-xs text-white/80 hover:bg-white/10 px-2 py-1 rounded transition-colors",
                                        i { class: "fa-solid fa-folder text-white/40" }
                                        span { class: "truncate", "{folder_name(&dir)}" }
                                    }
                                }
                            },
                        }
                    }

                    div { class: "flex items-center gap-2",
                        button {
                            onclick: move |_| {
                                on_add.call(path());
                                browsing.set(false);
                            },
                            class: "text-xs bg-indigo-500/70 hover:bg-indigo-500 px-2 py-1 rounded text-white transition-colors",
                            "{i18n::t(\"use_this_folder\")}"
                        }
                        button {
                            onclick: move |_| browsing.set(false),
                            class: "text-xs bg-white/10 hover:bg-white/20 px-2 py-1 rounded text-white transition-colors",
                            "{i18n::t(\"cancel\")}"
                        }
                    }
                }
            } else {
                button {
                    onclick: move |_| {
                        path.set("/".to_string());
                        browsing.set(true);
                    },
                    class: "bg-white/10 hover:bg-white/20 px-3 py-1 rounded text-sm text-white transition-colors self-start",
                    "{i18n::t(\"add_folder\")}"
                }
            }
        }
    }
}
