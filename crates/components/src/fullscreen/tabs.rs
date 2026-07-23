use crate::lyrics_view::LyricsView;
use crate::queue_list_view::QueueListView;
use config::AppConfig;
use dioxus::prelude::*;

#[component]
pub(crate) fn Tabs(
    mut config: Signal<AppConfig>,
    items: Vec<reader::Track>,
    current_queue_index: Signal<usize>,
    lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>>,
    current_song_progress: Signal<u64>,
) -> Element {
    let mut active_tab = use_signal(|| 0usize);

    rsx! {
        div {
            class: "flex-1 flex flex-col h-full min-w-0 bg-black/45 border-l border-white/5",

            div {
                class: "flex items-center px-6 pt-4 pb-2",
                div {
                    class: "flex items-center gap-1 p-1 rounded-lg bg-white/10",
                    button {
                        class: if *active_tab.read() == 0 {
                            "px-4 py-1.5 text-xs font-medium rounded-md bg-white/20 text-white transition-colors"
                        } else {
                            "px-4 py-1.5 text-xs font-medium rounded-md text-white/50 hover:text-white/80 transition-colors"
                        },
                        onclick: move |_| active_tab.set(0),
                        "{i18n::t(\"up_next\")}"
                    }

                    button {
                        class: if *active_tab.read() == 1 {
                            "px-4 py-1.5 text-xs font-medium rounded-md bg-white/20 text-white transition-colors"
                        } else {
                            "px-4 py-1.5 text-xs font-medium rounded-md text-white/50 hover:text-white/80 transition-colors"
                        },
                        onclick: move |_| active_tab.set(1),
                        "{i18n::t(\"lyrics\")}"
                    }
                }

                button {
                    class: "ml-auto w-8 h-8 flex items-center justify-center text-white/40 hover:text-white transition-colors",
                    "aria-label": i18n::t("hide_side_panel").to_string(),
                    title: i18n::t("hide_side_panel").to_string(),
                    onclick: move |_| config.write().fullscreen_tabs_collapsed = true,
                    i { class: "fa-solid fa-chevron-right text-sm" }
                }
            }

            if *active_tab.read() == 0 {
                QueueListView {
                    items,
                    config,
                    current_queue_index,
                    layout: crate::queue_list_view::LayoutMode::Fullscreen,
                }
            } else if *active_tab.read() == 1 {
                LyricsView {
                    lyrics,
                    current_song_progress,
                    config,
                    layout: crate::lyrics_view::LayoutMode::Fullscreen,
                }
            }
        }
    }
}
