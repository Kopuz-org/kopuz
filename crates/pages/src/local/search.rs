use components::playlist_modal::PlaylistModal;
use components::search_bar::SearchBar;
use components::search_genre_detail::SearchGenreDetail;
use components::search_genres::SearchGenres;
use components::search_results::SearchResults;
use config::{AppConfig, UiStyle};
use db::Source;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_albums, use_genre_tracks};
use hooks::use_search_data::use_search_data;
use player::player;

#[component]
pub fn LocalSearch(
    config: Signal<AppConfig>,
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
    let data = use_search_data(search_query, config);
    let mut selected_genre = use_signal(|| None::<String>);

    let mut active_menu_track = use_signal(|| None::<std::path::PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let selected_track_for_playlist = use_signal(|| None::<std::path::PathBuf>);

    let gens = hooks::db_reactivity::use_generations();
    let source = use_memo(|| Source::Local);
    let albums_res = use_albums(source);
    let selected_genre_memo =
        use_memo(move || selected_genre.read().clone().unwrap_or_default());
    let genre_tracks_res = use_genre_tracks(source, selected_genre_memo);

    let genre_tracks = use_memo(move || {
        let tracks = genre_tracks_res.read().clone().unwrap_or_default();
        if tracks.is_empty() {
            return Vec::new();
        }
        let all_albums = albums_res.read().clone().unwrap_or_default();
        let album_map: std::collections::HashMap<&String, &reader::models::Album> =
            all_albums.iter().map(|a| (&a.id, a)).collect();
        tracks
            .iter()
            .map(|track| {
                let cover = album_map
                    .get(&track.album_id)
                    .and_then(|a| a.cover_path.as_ref())
                    .and_then(|c| utils::format_artwork_url(Some(c)));
                (track.clone(), cover)
            })
            .collect()
    });

    let is_modern = config.read().ui_style == UiStyle::Modern;

    rsx! {
        div {
            class: if is_modern { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "p-8 absolute inset-0 flex flex-col" },

            if *show_playlist_modal.read() {
                PlaylistModal {
                    is_jellyfin: false,
                    on_close: move |_| show_playlist_modal.set(false),
                    on_add_to_playlist: move |playlist_id: String| {
                        if let Some(path) = selected_track_for_playlist.read().clone() {
                            let db = consume_context::<db::Db>();
                            spawn(async move {
                                let store = db.load_playlists(None).await.unwrap_or_default();
                                if let Some(playlist) =
                                    store.playlists.iter().find(|p| p.id == playlist_id)
                                {
                                    let mut tracks = playlist.tracks.clone();
                                    if !tracks.contains(&path) {
                                        tracks.push(path);
                                    }
                                    let refs: Vec<String> = tracks
                                        .iter()
                                        .map(|p| p.to_string_lossy().into_owned())
                                        .collect();
                                    if db
                                        .set_playlist_tracks(&Source::Local, &playlist_id, &refs)
                                        .await
                                        .is_ok()
                                    {
                                        gens.bump(Table::Playlists);
                                    }
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                    },
                    on_create_playlist: move |name: String| {
                        if let Some(path) = selected_track_for_playlist.read().clone() {
                            let refs = vec![path.to_string_lossy().into_owned()];
                            let id = uuid::Uuid::new_v4().to_string();
                            let db = consume_context::<db::Db>();
                            spawn(async move {
                                if db
                                    .upsert_playlist_meta(&Source::Local, &id, &name, None, None)
                                    .await
                                    .is_ok()
                                    && db
                                        .set_playlist_tracks(&Source::Local, &id, &refs)
                                        .await
                                        .is_ok()
                                {
                                    gens.bump(Table::Playlists);
                                }
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
