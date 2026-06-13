use components::playlist_modal::PlaylistModal;
use components::search_bar::SearchBar;
use components::search_genre_detail::SearchGenreDetail;
use components::search_genres::SearchGenres;
use components::search_results::SearchResults;
use config::{AppConfig, UiStyle};
use db::Source;
use dioxus::prelude::*;
use hooks::use_db_queries::use_genre_tracks;
use hooks::use_search_data::use_search_data;
use player::player;

#[component]
pub fn JellyfinSearch(
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

    let active_server_id = use_memo(move || {
        let c = config.read();
        c.active_server_id
            .clone()
            .or_else(|| c.server.as_ref().and_then(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let server_source = use_memo(move || Source::Server(active_server_id()));
    let selected_genre_memo = use_memo(move || selected_genre.read().clone().unwrap_or_default());
    let genre_tracks_res = use_genre_tracks(server_source, selected_genre_memo);

    let genre_tracks = use_memo(move || {
        let conf = config.read();
        genre_tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|track| {
                let cover = conf.server.as_ref().and_then(|server| {
                    utils::map_cover_url(utils::jellyfin_image::resolve_track_cover(
                        track.cover.as_deref(),
                        &track.id.key(),
                        &track.album_id,
                        &server.url,
                        server.access_token.as_deref(),
                        80,
                        80,
                    ))
                });
                (track, cover)
            })
            .collect::<Vec<_>>()
    });

    let is_modern = config.read().ui_style == UiStyle::Modern;

    rsx! {
        div {
            class: if is_modern { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "p-8 absolute inset-0 flex flex-col" },

            if *show_playlist_modal.read() {
                PlaylistModal {
                    is_jellyfin: true,
                    on_close: move |_| show_playlist_modal.set(false),
                    on_add_to_playlist: move |playlist_id: String| {
                        if let Some(path) = selected_track_for_playlist.read().clone() {
                            let path_clone = path.clone();
                            let pid = playlist_id.clone();
                            spawn(async move {
                                let Some(conn) =
                                    ::server::server_ops::ServerConn::resolve(&config.peek())
                                else {
                                    return;
                                };
                                let item_ids: Vec<String> =
                                    ::server::server_ops::parse_item_id(
                                        path_clone.to_str().unwrap_or_default(),
                                    )
                                    .map(|id| vec![id.to_string()])
                                    .unwrap_or_default();
                                let _ = ::server::server_ops::add_tracks_to_playlist(
                                    &conn, &pid, &item_ids,
                                )
                                .await;
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
                    },
                    on_create_playlist: move |name: String| {
                        if let Some(path) = selected_track_for_playlist.read().clone() {
                            let path_clone = path.clone();
                            let playlist_name = name.clone();
                            spawn(async move {
                                let Some(conn) =
                                    ::server::server_ops::ServerConn::resolve(&config.peek())
                                else {
                                    return;
                                };
                                let Some(item_id) = ::server::server_ops::parse_item_id(
                                    path_clone.to_str().unwrap_or_default(),
                                ) else {
                                    // Unparseable track id → don't create an empty playlist.
                                    return;
                                };
                                let _ = ::server::server_ops::create_server_playlist(
                                    &conn,
                                    &playlist_name,
                                    &[item_id.to_string()],
                                )
                                .await;
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

pub use JellyfinSearch as ServerSearch;
