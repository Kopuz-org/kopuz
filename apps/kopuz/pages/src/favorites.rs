use components::search_bar::SearchBar;
use config::{Settings, UiStyle};
use dioxus::prelude::*;

use crate::favorites_body::FavoritesBody;

#[component]
pub fn FavoritesPage(
    settings: Signal<Settings>,
    mut is_playing: Signal<bool>,
    mut current_playing: Signal<u64>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    mut queue: Signal<Vec<api::TrackInfo>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let is_vaxry = settings.read().ui.ui_style == UiStyle::Vaxry;
    let search_query = use_signal(String::new);

    rsx! {
        div {
            // Height-constrained column so the server list can window its rows
            // behind its own scroller (837 favorites in the DOM at once was
            // the page's frame-rate problem).
            class: crate::layout::page_container_class(&settings.read().ui.ui_style),

            if is_vaxry {
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

            SearchBar { search_query }
            FavoritesBody {
                settings,
                queue,
                search_query,
            }
        }
    }
}
