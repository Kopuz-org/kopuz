use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::use_playlists;
use std::future::Future;
use std::sync::Arc;

async fn run_folder_mutation<F, E, S, C>(mutation: F, on_success: S, on_close: C)
where
    F: Future<Output = Result<(), E>>,
    S: FnOnce(),
    C: FnOnce(),
{
    if mutation.await.is_ok() {
        on_success();
    }
    on_close();
}

async fn create_and_move_folder(
    api: Arc<dyn api::KopuzApi>,
    name: String,
    playlist_id: String,
) -> Result<(), api::ApiError> {
    let move_api = api.clone();
    create_then_move(
        api.create_playlist_folder(name),
        move |folder_id| async move { move_api.move_playlist(playlist_id, Some(folder_id)).await },
    )
    .await
}

async fn create_then_move<C, M, MFut, T, E>(create: C, move_to: M) -> Result<(), E>
where
    C: Future<Output = Result<T, E>>,
    M: FnOnce(T) -> MFut,
    MFut: Future<Output = Result<(), E>>,
{
    let folder = create.await?;
    move_to(folder).await
}

#[component]
pub fn FolderPickerModal(playlist_id: String, on_close: EventHandler<()>) -> Element {
    let mut new_folder_name = use_signal(String::new);
    let mut show_create = use_signal(|| false);
    let mut is_submitting = use_signal(|| false);
    let gens = hooks::db_reactivity::use_generations();
    let api = use_context::<Arc<dyn api::KopuzApi>>();
    let playlists_res = use_playlists();

    let folders = playlists_res
        .read()
        .as_ref()
        .map(|s| s.folders.clone())
        .unwrap_or_default();

    let pid = playlist_id.clone();
    let pid_keydown = pid.clone();
    let pid_btn = pid.clone();
    let api_keydown = api.clone();
    let api_btn = api.clone();

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-center justify-center bg-black/60",
            onclick: move |_| {
                if !*is_submitting.peek() {
                    on_close.call(());
                }
            },

            div {
                class: "bg-neutral-900 border border-white/10 rounded-lg p-6 w-80 shadow-2xl",
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
                                let folder_api = api.clone();
                                rsx! {
                                    button {
                                        key: "{fid}",
                                        disabled: *is_submitting.read(),
                                        class: "w-full text-left px-3 py-2 rounded-lg text-sm text-white hover:bg-white/10 flex items-center gap-2 transition-colors",
                                        onclick: move |_| {
                                            if *is_submitting.peek() {
                                                return;
                                            }
                                            is_submitting.set(true);
                                            let api = folder_api.clone();
                                            let pid = pid2.clone();
                                            let fid = fid.clone();
                                            spawn(async move {
                                                run_folder_mutation(
                                                    api.move_playlist(pid, Some(fid)),
                                                    move || gens.bump(Table::Folders),
                                                    move || on_close.call(()),
                                                )
                                                .await;
                                            });
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
                            disabled: *is_submitting.read(),
                            value: "{new_folder_name}",
                            oninput: move |evt| new_folder_name.set(evt.value()),
                            onkeydown: move |evt| {
                                if evt.key() == Key::Enter {
                                    let name = new_folder_name.read().trim().to_string();
                                    if !name.is_empty() && !*is_submitting.peek() {
                                        is_submitting.set(true);
                                        let pid = pid_keydown.clone();
                                        let api = api_keydown.clone();
                                        spawn(async move {
                                            run_folder_mutation(
                                                create_and_move_folder(api, name, pid),
                                                move || gens.bump(Table::Folders),
                                                move || on_close.call(()),
                                            )
                                            .await;
                                        });
                                    }
                                }
                            },
                        }
                        button {
                            disabled: *is_submitting.read(),
                            class: "px-3 py-2 bg-indigo-500 hover:bg-indigo-400 text-white rounded-lg text-sm transition-colors",
                            onclick: {
                                let pid4 = pid_btn.clone();
                                move |_| {
                                    let name = new_folder_name.read().trim().to_string();
                                    if !name.is_empty() && !*is_submitting.peek() {
                                        is_submitting.set(true);
                                        let pid = pid4.clone();
                                        let api = api_btn.clone();
                                        spawn(async move {
                                            run_folder_mutation(
                                                create_and_move_folder(api, name, pid),
                                                move || gens.bump(Table::Folders),
                                                move || on_close.call(()),
                                            )
                                            .await;
                                        });
                                    }
                                }
                            },
                            "{i18n::t(\"create\")}"
                        }
                    }
                }

                div { class: "flex gap-2",
                    button {
                        disabled: *is_submitting.read(),
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
                        disabled: *is_submitting.read(),
                        class: "px-4 py-2 text-sm text-slate-400 hover:text-white transition-colors",
                        onclick: move |_| {
                            if !*is_submitting.peek() {
                                on_close.call(());
                            }
                        },
                        "{i18n::t(\"cancel\")}"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{create_then_move, run_folder_mutation};
    use std::sync::{Arc, Mutex};

    fn record(events: &Arc<Mutex<Vec<&'static str>>>, event: &'static str) {
        events.lock().expect("event log poisoned").push(event);
    }

    #[tokio::test]
    async fn selecting_folder_finishes_mutation_before_refresh_and_close() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let (complete, pending) = tokio::sync::oneshot::channel();
        let mutation_events = events.clone();
        let refresh_events = events.clone();
        let close_events = events.clone();

        let task = tokio::spawn(run_folder_mutation(
            async move {
                record(&mutation_events, "mutation-started");
                pending.await.map_err(|_| ())?;
                record(&mutation_events, "mutation-finished");
                Ok::<(), ()>(())
            },
            move || record(&refresh_events, "refreshed"),
            move || record(&close_events, "closed"),
        ));

        tokio::task::yield_now().await;
        assert_eq!(
            *events.lock().expect("event log poisoned"),
            ["mutation-started"]
        );
        complete.send(()).expect("mutation receiver dropped");
        task.await.expect("mutation task panicked");
        assert_eq!(
            *events.lock().expect("event log poisoned"),
            [
                "mutation-started",
                "mutation-finished",
                "refreshed",
                "closed"
            ]
        );
    }

    #[tokio::test]
    async fn creating_folder_finishes_both_mutations_before_refresh_and_close() {
        for action in ["enter", "button"] {
            let events = Arc::new(Mutex::new(Vec::new()));
            let mutation_events = events.clone();
            let refresh_events = events.clone();
            let close_events = events.clone();

            run_folder_mutation(
                create_then_move(
                    {
                        let events = mutation_events.clone();
                        async move {
                            record(&events, "folder-created");
                            Ok::<(), ()>(())
                        }
                    },
                    move |_| {
                        let events = mutation_events.clone();
                        async move {
                            record(&events, "playlist-moved");
                            Ok::<(), ()>(())
                        }
                    },
                ),
                move || record(&refresh_events, "refreshed"),
                move || record(&close_events, "closed"),
            )
            .await;

            assert_eq!(
                *events.lock().expect("event log poisoned"),
                ["folder-created", "playlist-moved", "refreshed", "closed"],
                "unexpected lifecycle for {action} action"
            );
        }
    }

    #[tokio::test]
    async fn failed_mutation_skips_refresh_but_still_closes_after_attempt() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mutation_events = events.clone();
        let refresh_events = events.clone();
        let close_events = events.clone();

        run_folder_mutation(
            async move {
                record(&mutation_events, "mutation-failed");
                Err::<(), ()>(())
            },
            move || record(&refresh_events, "refreshed"),
            move || record(&close_events, "closed"),
        )
        .await;

        assert_eq!(
            *events.lock().expect("event log poisoned"),
            ["mutation-failed", "closed"]
        );
    }

    #[tokio::test]
    async fn failed_folder_creation_does_not_attempt_playlist_move() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let create_events = events.clone();
        let move_events = events.clone();

        let result = create_then_move(
            async move {
                record(&create_events, "folder-create-failed");
                Err::<(), ()>(())
            },
            move |_| async move {
                record(&move_events, "playlist-moved");
                Ok::<(), ()>(())
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            *events.lock().expect("event log poisoned"),
            ["folder-create-failed"]
        );
    }

    #[tokio::test]
    async fn failed_playlist_move_skips_refresh_but_still_closes() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let create_events = events.clone();
        let move_events = events.clone();
        let refresh_events = events.clone();
        let close_events = events.clone();

        run_folder_mutation(
            create_then_move(
                async move {
                    record(&create_events, "folder-created");
                    Ok::<(), ()>(())
                },
                move |_| async move {
                    record(&move_events, "playlist-move-failed");
                    Err::<(), ()>(())
                },
            ),
            move || record(&refresh_events, "refreshed"),
            move || record(&close_events, "closed"),
        )
        .await;

        assert_eq!(
            *events.lock().expect("event log poisoned"),
            ["folder-created", "playlist-move-failed", "closed"]
        );
    }
}
