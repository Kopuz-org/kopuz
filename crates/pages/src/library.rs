use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use player::player;
use reader::Library;

use crate::local::library::LocalLibrary;
use crate::server::library::ServerLibrary;

#[component]
pub fn LibraryPage(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    playlist_store: Signal<reader::PlaylistStore>,
    on_rescan: EventHandler,
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
) -> Element {
    let is_server = config.read().active_source == MusicSource::Server;

    rsx! {
        if is_server {
            ServerLibrary {
                library,
                config,
                playlist_store,
                queue,
            }
        } else {
            LocalLibrary {
                library,
                config,
                playlist_store,
                on_rescan,
                queue,
            }
        }
    }
}
