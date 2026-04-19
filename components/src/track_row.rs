use crate::dots_menu::{DotsMenu, MenuAction};
use dioxus::prelude::*;
use reader::models::Track;

#[component]
pub fn TrackRow(
    track: Track,
    cover_url: Option<String>,
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
) -> Element {
    let mut actions = vec![MenuAction::new("Add to Playlist", "fa-solid fa-plus")];

    let has_remove = on_remove_from_playlist.is_some();
    if has_remove {
        actions.push(MenuAction::new("Remove from Playlist", "fa-solid fa-minus"));
    }

    if !hide_delete {
        actions.push(MenuAction::new("Delete Song", "fa-solid fa-trash").destructive());
    }

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
                if is_selection_mode {
                    if let Some(handler) = on_select {
                        handler.call(!is_selected);
                    }
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

            if is_selection_mode {
                div { class: "mr-4 flex items-center justify-center w-6 h-6",
                    input {
                        r#type: "checkbox",
                        class: "w-4 h-4 rounded border-white/20 bg-transparent text-indigo-500 focus:ring-indigo-400",
                        checked: is_selected,
                        onchange: |_| {}, // Controlled component
                    }
                }
            }

            div { class: "w-10 h-10 bg-white/5 rounded overflow-hidden flex items-center justify-center mr-4 shrink-0",
                if let Some(url) = cover_url {
                    img {
                        src: "{url}",
                        class: "w-full h-full object-cover",
                        loading: "lazy",
                        decoding: "async",
                    }
                } else {
                    i { class: "fa-solid fa-music text-white/20" }
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
                        } else {
                            on_delete.call(());
                        }
                    },
                }
            }
        }
    }
}
