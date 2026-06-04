use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use player::player;
use reader::Library;

use crate::local::search::LocalSearch;
use crate::server::search::ServerSearch;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use crate::spotify_page::{SpotifyPage, SpotifyView};

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
    on_select_album: EventHandler<String>,
) -> Element {
    let active_source = config.read().active_source;
    let is_server = active_source == MusicSource::Server;
    let is_spotify = active_source == MusicSource::Spotify;

    rsx! {
        if is_spotify {
            {spotify_section(library, playlist_store, config)}
        } else if is_server {
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
                on_select_album,
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
                on_select_album,
            }
        }

    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn spotify_section(
    library: Signal<Library>,
    playlist_store: Signal<reader::PlaylistStore>,
    config: Signal<AppConfig>,
) -> Element {
    rsx! { SpotifyPage { library, playlist_store, config, view: SpotifyView::Search } }
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn spotify_section(
    _library: Signal<Library>,
    _playlist_store: Signal<reader::PlaylistStore>,
    _config: Signal<AppConfig>,
) -> Element {
    rsx! {}
}
