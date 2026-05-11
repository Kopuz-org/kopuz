use crate::dots_menu::{DotsMenu, MenuAction};
use dioxus::prelude::*;
use reader::models::Track;

#[component]
pub fn TrackRow(
    track: Track,
    cover_url: Option<utils::CoverUrl>,
    on_click_menu: EventHandler<()>,
    is_menu_open: bool,
    on_add_to_playlist: EventHandler<()>,
    on_close_menu: EventHandler<()>,
    on_play: EventHandler<()>,
    on_delete: EventHandler<()>,
    on_remove_from_playlist: Option<EventHandler<()>>,
    #[props(default = false)] is_selection_mode: bool,
    #[props(default = false)] is_selected: bool,
    #[props(default = false)] hide_delete: bool,
    on_select: Option<EventHandler<bool>>,
    on_long_press: Option<EventHandler<()>>,
    on_download: Option<EventHandler<()>>,
    #[props(default = false)] is_downloaded: bool,
    #[props(default = false)] is_downloading: bool,
) -> Element {
    let add_to_playlist_text = i18n::t("add_to_playlist").to_string();
    let remove_from_playlist_text = i18n::t("remove_from_playlist").to_string();
    let delete_song_text = i18n::t("delete").to_string();

    let mut actions = vec![MenuAction::new(
        add_to_playlist_text.as_str(),
        "fa-solid fa-plus",
    )];

    let has_remove = on_remove_from_playlist.is_some();
    if has_remove {
        actions.push(MenuAction::new(
            remove_from_playlist_text.as_str(),
            "fa-solid fa-minus",
        ));
    }

    let has_download = on_download.is_some();
    if has_download {
        let (dl_label, dl_icon) = if is_downloading {
            ("Downloading...", "fa-solid fa-spinner fa-spin")
        } else if is_downloaded {
            ("Remove Download", "fa-solid fa-trash-can")
        } else {
            ("Download Offline", "fa-solid fa-download")
        };
        let mut action = MenuAction::new(dl_label, dl_icon);
        if is_downloaded {
            action = action.destructive();
        }
        actions.push(action);
    }

    if !hide_delete {
        actions.push(MenuAction::new(delete_song_text.as_str(), "fa-solid fa-trash").destructive());
    }

    let download_action_idx = if has_remove { 2 } else { 1 };
    let delete_action_idx = if has_download {
        download_action_idx + 1
    } else {
        download_action_idx
    };

    let mut long_press_task = use_signal(|| None);
    let mut long_press_occurred = use_signal(|| false);

    let mut start_long_press = move || {
        if is_selection_mode {
            return;
        }
        long_press_occurred.set(false);
        if let Some(handler) = on_long_press {
            let mut occurred = long_press_occurred;
            let task = spawn(async move {
                utils::sleep(std::time::Duration::from_millis(600)).await;
                occurred.set(true);
                handler.call(());
            });
            long_press_task.set(Some(task));
        }
    };

    let mut cancel_long_press = move || {
        if let Some(task) = long_press_task.write().take() {
            task.cancel();
        }
    };

    rsx! {
        div {
            class: format!(
                "flex items-center p-2 rounded-lg hover:bg-white/5 group transition-colors relative select-none {}",
                if is_selected { "bg-white/10" } else { "" }
            ),
            onclick: move |evt| {
                evt.stop_propagation();
                if *long_press_occurred.read() {
                    long_press_occurred.set(false);
                    return;
                }
                if !is_selected && !is_selection_mode {
                    if let Some(handler) = on_long_press {
                        handler.call(());
                    } else if let Some(handler) = on_select {
                        handler.call(true);
                    }
                } else if let Some(handler) = on_select {
                    handler.call(!is_selected);
                }
            },
            ondoubleclick: move |evt| {
                evt.stop_propagation();
                if !is_selection_mode {
                    on_play.call(());
                }
            },
            onmousedown: move |_| start_long_press(),
            onmouseup: move |_| cancel_long_press(),
            onmouseleave: move |_| cancel_long_press(),
            ontouchstart: move |_| start_long_press(),
            ontouchend: move |_| cancel_long_press(),
            oncontextmenu: move |evt| {
                evt.prevent_default();
                if !is_selection_mode {
                    on_click_menu.call(());
                }
            },

            if on_select.is_some() {
                div { class: "mr-4 flex items-center justify-center w-6 h-6 shrink-0",
                    button {
                        class: if is_selected {
                            "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                        } else {
                            "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                        },
                        aria_label: if is_selected { "Deselect track" } else { "Select track" },
                        onclick: move |evt| {
                            evt.stop_propagation();
                            if !is_selected && !is_selection_mode {
                                if let Some(handler) = on_long_press {
                                    handler.call(());
                                } else if let Some(handler) = on_select {
                                    handler.call(true);
                                }
                            } else if let Some(handler) = on_select {
                                handler.call(!is_selected);
                            }
                        },
                        if is_selected {
                            i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                        }
                    }
                }
            }

            div { class: "relative w-10 h-10 bg-white/5 rounded overflow-hidden flex items-center justify-center mr-4 shrink-0",
                if let Some(url) = cover_url {
                    img {
                        src: "{url.as_ref()}",
                        class: "w-full h-full object-cover",
                        loading: "lazy",
                        decoding: "async",
                    }
                } else {
                    i { class: "fa-solid fa-music text-white/20" }
                }
                if is_downloaded {
                    div { class: "absolute bottom-0 right-0 w-3 h-3 bg-indigo-500 rounded-tl flex items-center justify-center",
                        i { class: "fa-solid fa-check text-white", style: "font-size: 6px;" }
                    }
                }
            }

            div { class: "flex-1 min-w-0 pr-4",
                p { class: "text-sm font-medium text-white/90 truncate", "{track.title}" }
                p { class: "text-xs text-slate-500 truncate", "{track.artist}" }
            }

            if !is_selection_mode {
                DotsMenu {
                    actions,
                    is_open: is_menu_open,
                    on_open: move |_| on_click_menu.call(()),
                    on_close: move |_| on_close_menu.call(()),
                    button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                    anchor: "right".to_string(),
                    on_action: move |idx: usize| {
                        if idx == 0 {
                            on_add_to_playlist.call(());
                        } else if has_remove && idx == 1 {
                            if let Some(handler) = on_remove_from_playlist {
                                handler.call(());
                            }
                        } else if has_download && idx == download_action_idx {
                            if let Some(handler) = on_download {
                                handler.call(());
                            }
                        } else if idx == delete_action_idx {
                            on_delete.call(());
                        }
                    },
                }
            }
        }
    }
}
