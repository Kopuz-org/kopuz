use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use player::player;
use reader::{Library, PlaylistStore};

use crate::local::artist::LocalArtist;
use crate::server::artist::ServerArtist;

#[component]
pub fn Artist(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    player: Signal<player::Player>,
    mut is_playing: Signal<bool>,
    mut current_playing: Signal<u64>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
    on_close: EventHandler<()>,
) -> Element {
    let is_server = config.read().active_source == MusicSource::Server;

    rsx! {
        div {
            class: "p-8 pb-24",

            div { class: "w-full max-w-[1600px] mx-auto",
                div { class: "flex items-center justify-between mb-8",
                    button {
                        class: "flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                        onclick: move |_| on_close.call(()),
                        i { class: "fa-solid fa-arrow-left" }
                        "Back"
                    }
                }

                if is_server {
                    ServerArtist {
                        library,
                        config,
                        artist_name,
                        playlist_store,
                        queue,
                        current_queue_index,
                    }
                } else {
                    LocalArtist {
                        library,
                        config,
                        artist_name,
                        playlist_store,
                        queue,
                        current_queue_index,
                    }
                }
            }
        }
    }
}
