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
    on_navigate: EventHandler<String>,
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
    let page_container_class = crate::layout::page_container_class(&config.read().ui_style);

    rsx! {
        div {
            class: page_container_class,

            if artist_name.read().is_empty() {
                div { class: "flex-1 min-h-0 flex flex-col",
                    if !cfg!(target_os = "android") {
                        h1 { class: "text-3xl font-bold text-white mb-6 shrink-0", "{i18n::t(\"artists\")}" }
                    }

                    if is_server {
                        ServerArtist {
                            library,
                            config,
                            artist_name,
                            playlist_store,
                            on_navigate,
                            queue,
                            current_queue_index,
                        }
                    } else {
                        LocalArtist {
                            library,
                            config,
                            artist_name,
                            playlist_store,
                            on_navigate,
                            queue,
                            current_queue_index,
                        }
                    }
                }
            } else {
                div { class: "relative flex-1 min-h-0 flex flex-col w-full max-w-[1600px] mx-auto",
                    if !cfg!(target_os = "android") {
                        button {
                            class: "flex items-center gap-2 text-slate-400 hover:text-white transition-colors mb-6 group shrink-0",
                            onclick: move |_| artist_name.set(String::new()),
                            i { class: "fa-solid fa-chevron-left text-sm group-hover:-translate-x-0.5 transition-transform" }
                            span { class: "text-sm font-medium", "{i18n::t(\"back_to_artists\")}" }
                        }
                    }
                    if is_server {
                        ServerArtist {
                            library,
                            config,
                            artist_name,
                            playlist_store,
                            on_navigate,
                            queue,
                            current_queue_index,
                        }
                    } else {
                        LocalArtist {
                            library,
                            config,
                            artist_name,
                            playlist_store,
                            on_navigate,
                            queue,
                            current_queue_index,
                        }
                    }
                }
            }
        }
    }
}
