use config::{AppConfig, UiStyle};
use dioxus::prelude::*;

use crate::favorites_body::FavoritesBody;

#[component]
pub fn FavoritesPage(
    config: Signal<AppConfig>,
    player: Signal<player::player::Player>,
    mut is_playing: Signal<bool>,
    mut current_playing: Signal<u64>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let is_modern = config.read().ui_style == UiStyle::Modern;

    rsx! {
        div {
            // Height-constrained column so the server list can window its rows
            // behind its own scroller (837 favorites in the DOM at once was
            // the page's frame-rate problem).
            class: if cfg!(target_os = "android") { "px-4 pt-2 absolute inset-0 flex flex-col overflow-x-hidden" } else if is_modern { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },

            if is_modern {
                div { class: "mb-6",
                    p {
                        class: "text-[10px] font-bold mb-1",
                        style: "color: rgba(255,255,255,0.35);",
                        "{i18n::t(\"library\")}"
                    }
                    h1 {
                        class: "text-3xl font-bold text-white",
                        "{i18n::t(\"favorites\")}"
                    }
                }
            } else {
                div {
                    class: "flex items-center gap-3 mb-8",
                    i { class: "fa-solid fa-heart text-red-400 text-2xl" }
                    h1 { class: "text-3xl font-bold text-white", "{i18n::t(\"favorites\")}" }
                }
            }

            FavoritesBody { config, queue }
        }
    }
}
