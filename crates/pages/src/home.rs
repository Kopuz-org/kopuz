use config::{AppConfig, MusicSource, UiStyle};
use dioxus::prelude::*;

use crate::local::home::LocalHome;
use crate::server::home::ServerHome;

#[component]
pub fn Home(
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let mut config = use_context::<Signal<AppConfig>>();
    let is_server = config.read().active_source == MusicSource::Server;
    let is_modern = config.read().ui_style == UiStyle::Modern;
    let mut edit_mode = use_signal(|| false);

    rsx! {
        div {
            class: if cfg!(target_os = "android") {
                "px-4 pt-2 pb-28 space-y-8 w-full"
            } else if is_modern {
                "px-6 pt-4 pb-24 w-full"
            } else {
                "p-8 space-y-12 pb-32 animate-fade-in w-full"
            },

            div { class: "flex items-center justify-between mb-4",
                if !is_modern {
                    h1 { class: "text-4xl font-black text-white tracking-tight", "{i18n::t(\"home\")}" }
                } else {
                    div {}
                }
                div { class: "flex items-center gap-2",
                    if *edit_mode.read() {
                        button {
                            class: "px-3 h-8 rounded-full bg-white/5 hover:bg-white/10 border border-white/10 text-white/70 hover:text-white text-xs font-semibold transition-colors",
                            onclick: move |_| {
                                config.write().home_sections = config::default_home_sections();
                            },
                            i { class: "fa-solid fa-rotate-left mr-1 text-[10px]" }
                            "{i18n::t(\"reset_home\")}"
                        }
                        button {
                            class: "px-4 h-8 rounded-full bg-indigo-500 hover:bg-indigo-400 text-white text-xs font-bold transition-colors",
                            onclick: move |_| edit_mode.set(false),
                            i { class: "fa-solid fa-check mr-1 text-[10px]" }
                            "{i18n::t(\"done_editing\")}"
                        }
                    } else {
                        button {
                            class: "px-3 h-8 rounded-full bg-white/5 hover:bg-white/10 border border-white/10 text-white/70 hover:text-white text-xs font-semibold transition-colors flex items-center gap-2",
                            onclick: move |_| edit_mode.set(true),
                            i { class: "fa-solid fa-sliders text-[10px]" }
                            "{i18n::t(\"customize_home\")}"
                        }
                    }
                }
            }

            if is_server {
                ServerHome {
                    edit_mode,
                    on_select_album,
                    on_play_album,
                    on_select_playlist,
                    on_search_artist,
                }
            } else {
                LocalHome {
                    edit_mode,
                    on_select_album,
                    on_play_album,
                    on_select_playlist,
                    on_search_artist,
                }
            }
        }
    }
}
