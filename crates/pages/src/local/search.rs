use components::playlist_modal::PlaylistModal;
use components::search_bar::SearchBar;
use components::search_genre_detail::SearchGenreDetail;
use components::search_genres::SearchGenres;
use components::search_results::SearchResults;
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_search_data::use_search_data;
use player::player;
use reader::Library;

#[component]
pub fn LocalSearch(
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
    let data = use_search_data(library, search_query, config);
    let mut selected_genre = use_signal(|| None::<String>);

    let mut active_menu_track = use_signal(|| None::<std::path::PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let selected_track_for_playlist = use_signal(|| None::<std::path::PathBuf>);

    let genre_tracks = use_memo(move || {
        let genre = selected_genre.read();

        if let Some(g) = &*genre {
            let lib = library.read();

            let valid_album_ids: std::collections::HashSet<&String> = lib
                .albums
                .iter()
                .filter(|a| a.genre.to_lowercase().contains(&g.to_lowercase()))
                .map(|a| &a.id)
                .collect();

            let album_map: std::collections::HashMap<&String, &reader::models::Album> =
                lib.albums.iter().map(|a| (&a.id, a)).collect();

            let mut matching_tracks = Vec::new();
            for track in &lib.tracks {
                if valid_album_ids.contains(&track.album_id) {
                    let cover = album_map
                        .get(&track.album_id)
                        .and_then(|a| a.cover_path.as_ref())
                        .and_then(|c| utils::format_artwork_url(Some(c)));
                    matching_tracks.push((track.clone(), cover));
                }
            }
            matching_tracks
        } else {
            Vec::new()
        }
    });

    let is_modern = config.read().ui_style == UiStyle::Modern;

    rsx! {
        div {
            class: if is_modern { "px-6 pt-6 pb-24" } else { "p-8" },

            if *show_playlist_modal.read() {
                PlaylistModal {
                    playlist_store: playlist_store,
                    is_jellyfin: false,
                    on_close: move |_| show_playlist_modal.set(false),
                    on_add_to_playlist: move |playlist_id: String| {
                        if let Some(path) = selected_track_for_playlist.read().clone() {
                            let mut store = playlist_store.write();
                            if let Some(playlist) = store.playlists.iter_mut().find(|p| p.id == playlist_id) {
                                if !playlist.tracks.contains(&path) {
                                    playlist.tracks.push(path);
                                }
                            }
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                    },
                    on_create_playlist: move |name: String| {
                        if let Some(path) = selected_track_for_playlist.read().clone() {
                            let mut store = playlist_store.write();
                            store.playlists.push(reader::models::Playlist {
                                id: uuid::Uuid::new_v4().to_string(),
                                name,
                                tracks: vec![path],
                                cover_path: None,
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                    },
                }
            }

            if let Some(genre) = selected_genre.read().as_ref() {
                SearchGenreDetail {
                    genre: genre.clone(),
                    genre_tracks: genre_tracks.read().clone(),
                    genres: (data.genres)().clone(),
                    on_back: move |_| selected_genre.set(None),
                    library,
                    playlist_store,
                    player,
                    is_playing,
                    current_song_cover_url,
                    current_song_title,
                    current_song_artist,
                    current_song_duration,
                    current_song_progress,
                    queue,
                    current_queue_index,
                    active_menu_track,
                    show_playlist_modal,
                    selected_track_for_playlist,
                }
            } else {
                SearchBar { search_query: data.search_query }

                if let Some(Some((tracks, albums))) = data.search_results.cloned() {
                    SearchResults {
                        search_query: data.search_query.read().clone(),
                        tracks: tracks.clone(),
                        albums: albums.clone(),
                        library,
                        playlist_store,
                        player,
                        is_playing,
                        current_song_cover_url,
                        current_song_title,
                        current_song_artist,
                        current_song_duration,
                        current_song_progress,
                        queue,
                        current_queue_index,
                        active_menu_track,
                        show_playlist_modal,
                        selected_track_for_playlist,
                        on_select_album,
                    }
                } else {
                    SearchGenres {
                        genres: (data.genres)().clone(),
                        on_select_genre: move |genre| selected_genre.set(Some(genre)),
                    }
                }
            }
        }
    }
}
