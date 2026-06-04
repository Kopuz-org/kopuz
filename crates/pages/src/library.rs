use config::{AppConfig, MusicSource};
use dioxus::prelude::*;
use player::player;
use reader::Library;

use crate::local::library::LocalLibrary;
use crate::server::library::ServerLibrary;
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use crate::spotify_page::{SpotifyPage, SpotifyView};

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
    let active_source = config.read().active_source;
    let is_server = active_source == MusicSource::Server;
    let is_spotify = active_source == MusicSource::Spotify;

    rsx! {
        if is_spotify {
            {spotify_library(library, playlist_store, config)}
        } else if is_server {
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

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn spotify_library(
    library: Signal<Library>,
    playlist_store: Signal<reader::PlaylistStore>,
    config: Signal<AppConfig>,
) -> Element {
    rsx! { SpotifyPage { library, playlist_store, config, view: SpotifyView::Library } }
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn spotify_library(
    _library: Signal<Library>,
    _playlist_store: Signal<reader::PlaylistStore>,
    _config: Signal<AppConfig>,
) -> Element {
    rsx! {}
}
