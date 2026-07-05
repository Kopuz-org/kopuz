use dioxus::prelude::*;

#[cfg(not(target_os = "android"))]
use rfd::AsyncFileDialog;

#[component]
pub fn AddPlaylistPopup(
    playlist_name: Signal<String>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
    #[props(default = false)] show_add_folder: bool,
    #[props(default)] on_add_folder: EventHandler<String>,
) -> Element {
    rsx! {
        div {
            class: "overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "popup",
                onclick: |e| e.stop_propagation(),

                div { class: "flex items-center justify-between gap-3 mb-4",
                    h2 { "{i18n::t(\"add_playlist\")}" }
                    if show_add_folder {
                        AddFolderFromFileManagerButton {
                            on_add_folder,
                            on_close,
                        }
                    }
                }

                if let Some(err) = error() {
                    p { class: "error", "{err}" }
                }

                input {
                    placeholder: "{i18n::t(\"playlist_name_placeholder\")}",
                    value: "{playlist_name()}",
                    oninput: move |e| playlist_name.set(e.value()),
                    onkeydown: move |e| e.stop_propagation()
                }

                div { class: "actions",
                    button {
                        onclick: move |_| on_close.call(()),
                        "{i18n::t(\"cancel\")}"
                    }
                    button {
                        onclick: move |_| on_save.call(()),
                        "{i18n::t(\"save\")}"
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
#[component]
fn AddFolderFromFileManagerButton(
    on_add_folder: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            class: "text-white/60 flex items-center hover:text-white transition-colors p-2 rounded-full hover:bg-white/10",
            onclick: move |evt| {
                evt.stop_propagation();
                spawn(async move {
                    if let Some(handle) = AsyncFileDialog::new().pick_folder().await {
                        on_add_folder.call(handle.path().to_string_lossy().to_string());
                        on_close.call(());
                    }
                });
            },
            i { class: "fa-solid fa-folder-open" }
        }
    }
}

// Android has no native folder dialog.
#[cfg(target_os = "android")]
#[component]
fn AddFolderFromFileManagerButton(
    on_add_folder: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let _ = on_add_folder;
    let _ = on_close;
    rsx! {}
}
