use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::use_playlists;

#[component]
pub fn FolderPickerModal(playlist_id: String, on_close: EventHandler<()>) -> Element {
    let mut new_folder_name = use_signal(String::new);
    let mut show_create = use_signal(|| false);
    let gens = hooks::db_reactivity::use_generations();
    let playlists_res = use_playlists();

    let folders = playlists_res
        .read()
        .as_ref()
        .map(|s| s.folders.clone())
        .unwrap_or_default();

    let pid = playlist_id.clone();
    let pid_keydown = pid.clone();
    let pid_btn = pid.clone();

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-center justify-center bg-black/60",
            onclick: move |_| on_close.call(()),

            div {
                class: "bg-neutral-900 border border-white/10 rounded-2xl p-6 w-80 shadow-2xl",
                onclick: move |evt| evt.stop_propagation(),

                h2 { class: "text-lg font-bold text-white mb-4", "{i18n::t(\"move_to_folder\")}" }

                if folders.is_empty() && !*show_create.read() {
                    p { class: "text-sm text-slate-500 mb-4", "{i18n::t(\"no_folders_yet\")}" }
                } else {
                    div { class: "space-y-1 mb-3 max-h-48 overflow-y-auto",
                        for folder in &folders {
                            {
                                let fid = folder.id.clone();
                                let fname = folder.name.clone();
                                let pid2 = pid.clone();
                                rsx! {
                                    button {
                                        key: "{fid}",
                                        class: "w-full text-left px-3 py-2 rounded-lg text-sm text-white hover:bg-white/10 flex items-center gap-2 transition-colors",
                                        onclick: move |_| {
                                            let db = consume_context::<db::Db>();
                                            let pid = pid2.clone();
                                            let fid = fid.clone();
                                            spawn(async move {
                                                if db.set_playlist_folder(&pid, Some(&fid)).await.is_ok() {
                                                    gens.bump(Table::Folders);
                                                }
                                            });
                                            on_close.call(());
                                        },
                                        i { class: "fa-solid fa-folder text-amber-400 text-xs" }
                                        "{fname}"
                                    }
                                }
                            }
                        }
                    }
                }

                if *show_create.read() {
                    div { class: "flex gap-2 mb-3",
                        input {
                            class: "flex-1 bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-sm text-white placeholder-slate-500 focus:outline-none focus:border-indigo-500",
                            placeholder: i18n::t("folder_name"),
                            value: "{new_folder_name}",
                            oninput: move |evt| new_folder_name.set(evt.value()),
                            onkeydown: move |evt| {
                                if evt.key() == Key::Enter {
                                    let name = new_folder_name.read().trim().to_string();
                                    if !name.is_empty() {
                                        let new_id = uuid::Uuid::new_v4().to_string();
                                        let pid = pid_keydown.clone();
                                        let db = consume_context::<db::Db>();
                                        spawn(async move {
                                            if db.create_folder(&new_id, &name).await.is_ok()
                                                && db
                                                    .set_playlist_folder(&pid, Some(&new_id))
                                                    .await
                                                    .is_ok()
                                            {
                                                gens.bump(Table::Folders);
                                            }
                                        });
                                        on_close.call(());
                                    }
                                }
                            },
                        }
                        button {
                            class: "px-3 py-2 bg-indigo-500 hover:bg-indigo-400 text-white rounded-lg text-sm transition-colors",
                            onclick: {
                                let pid4 = pid_btn.clone();
                                move |_| {
                                    let name = new_folder_name.read().trim().to_string();
                                    if !name.is_empty() {
                                        let new_id = uuid::Uuid::new_v4().to_string();
                                        let pid = pid4.clone();
                                        let db = consume_context::<db::Db>();
                                        spawn(async move {
                                            if db.create_folder(&new_id, &name).await.is_ok()
                                                && db
                                                    .set_playlist_folder(&pid, Some(&new_id))
                                                    .await
                                                    .is_ok()
                                            {
                                                gens.bump(Table::Folders);
                                            }
                                        });
                                        on_close.call(());
                                    }
                                }
                            },
                            "{i18n::t(\"create\")}"
                        }
                    }
                }

                div { class: "flex gap-2",
                    button {
                        class: "flex-1 py-2 text-sm text-slate-400 hover:text-white border border-white/10 rounded-lg transition-colors",
                        onclick: move |_| {
                            let next = !*show_create.read();
                            show_create.set(next);
                            new_folder_name.set(String::new());
                        },
                        i { class: "fa-solid fa-folder-plus mr-2 text-xs" }
                        "{i18n::t(\"new_folder\")}"
                    }
                    button {
                        class: "px-4 py-2 text-sm text-slate-400 hover:text-white transition-colors",
                        onclick: move |_| on_close.call(()),
                        "{i18n::t(\"cancel\")}"
                    }
                }
            }
        }
    }
}
