use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use player::player;
use reader::Library;

use crate::local::search::LocalSearch;
use crate::server::search::ServerSearch;

#[component]
pub fn Search(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    playlist_store: Signal<reader::PlaylistStore>,
    search_query: Signal<String>,
    player: Signal<player::Player>,
    is_playing: Signal<bool>,
    current_playing: Signal<u64>,
    current_song_cover_url: Signal<String>,
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_duration: Signal<u64>,
    current_song_progress: Signal<u64>,
    queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
) -> Element {
    let is_server = config.read().active_source == MusicSource::Server;

    rsx! {
        if is_server {
            ServerSearch {
                library,
                config,
                playlist_store,
                search_query,
                player,
                is_playing,
                current_playing,
                current_song_cover_url,
                current_song_title,
                current_song_artist,
                current_song_duration,
                current_song_progress,
                queue,
                current_queue_index,
            }
        } else {
            LocalSearch {
                library,
                config,
                playlist_store,
                search_query,
                player,
                is_playing,
                current_playing,
                current_song_cover_url,
                current_song_title,
                current_song_artist,
                current_song_duration,
                current_song_progress,
                queue,
                current_queue_index,
            }
        }
    }
}
