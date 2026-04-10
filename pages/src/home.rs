use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use reader::{FavoritesStore, Library, PlaylistStore};

use crate::local::home::LocalHome;
use crate::server::home::ServerHome;

#[component]
pub fn Home(
    library: Signal<Library>,
    playlist_store: Signal<PlaylistStore>,
    favorites_store: Signal<FavoritesStore>,
    on_select_album: EventHandler<String>,
    on_play_album: EventHandler<String>,
    on_select_playlist: EventHandler<String>,
    on_search_artist: EventHandler<String>,
) -> Element {
    let config = use_context::<Signal<AppConfig>>();
    let is_server = config.read().active_source == MusicSource::Server;

    rsx! {
        div {
            class: "p-8 space-y-12 pb-32 animate-fade-in w-full max-w-[1600px] mx-auto",

            div { class: "flex items-center justify-between mb-2",
                h1 { class: "text-4xl font-black text-white tracking-tight", "Home" }
            }

            if is_server {
                ServerHome {
                    library,
                    playlist_store,
                    favorites_store,
                    on_select_album,
                    on_play_album,
                    on_select_playlist,
                    on_search_artist,
                }
            } else {
                LocalHome {
                    library,
                    playlist_store,
                    favorites_store,
                    on_select_album,
                    on_play_album,
                    on_select_playlist,
                    on_search_artist,
                }
            }
        }
    }
}
