use config::{AppConfig, MusicSource, UiStyle};
use dioxus::prelude::*;
use reader::{FavoritesStore, Library, PlaylistStore};

use crate::local::favorites::LocalFavorites;
use crate::server::favorites::ServerFavorites;

#[component]
pub fn FavoritesPage(
    favorites_store: Signal<FavoritesStore>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    playlist_store: Signal<PlaylistStore>,
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
    let is_server = config.read().active_source == MusicSource::Server;
    let is_modern = config.read().ui_style == UiStyle::Modern;

    rsx! {
        div {
            class: if is_modern { "px-6 pt-6 pb-24 min-h-full" } else { "p-8 min-h-full" },

            if is_modern {
                div { class: "mb-6",
                    p {
                        class: "text-[10px] font-bold tracking-widest uppercase mb-1",
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

            if is_server {
                ServerFavorites {
                    favorites_store,
                    library,
                    config,
                    playlist_store,
                    queue,
                }
            } else {
                LocalFavorites {
                    favorites_store,
                    library,
                    config,
                    playlist_store,
                    queue,
                }
            }
        }
    }
}
