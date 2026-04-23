use dioxus::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use player::decoder;
use player::player::{NowPlayingMeta, Player};
use reader::Library;
use std::collections::HashSet;
use std::path::PathBuf;

#[component]
pub fn AlbumDetails(
    album_id: String,
    library: Signal<Library>,
    playlist_store: Signal<reader::PlaylistStore>,
    player: Signal<Player>,
    mut is_playing: Signal<bool>,
    mut current_song_cover_url: Signal<String>,
    mut current_song_title: Signal<String>,
    mut current_song_artist: Signal<String>,
    mut current_song_duration: Signal<u64>,
    mut current_song_progress: Signal<u64>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
    on_close: EventHandler<()>,
) -> Element {
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());

    let lib = library.read();
    let album = match lib.albums.iter().find(|a| a.id == album_id) {
        Some(a) => a,
        None => return rsx! { div { "{rust_i18n::t!(\"album_not_found\")}" } },
    };

    let mut tracks: Vec<_> = lib
        .tracks
        .iter()
        .filter(|t| t.album == album.title)
        .cloned()
        .collect();
    tracks.sort_by(|a, b| {
        a.track_number
            .cmp(&b.track_number)
            .then_with(|| a.title.cmp(&b.title))
    });

    let album_cover = utils::format_artwork_url(album.cover_path.as_ref());

    rsx! {
        div {
            class: "w-full max-w-[1600px] mx-auto",

            div { class: "flex items-center justify-between mb-8",
                button {
                    class: "flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                    onclick: move |_| on_close.call(()),
                    i { class: "fa-solid fa-arrow-left" }
                    "{rust_i18n::t!(\"back_to_albums\")}"
                }
            }

            crate::showcase::Showcase {
                name: album.title.clone(),
                description: album.artist.clone(),
                cover_url: album_cover,
                tracks: tracks.clone(),
                library: library,
                is_selection_mode: is_selection_mode(),
                selected_tracks: selected_tracks.read().clone(),
                on_long_press: {
                    let t_list = tracks.clone();
                    move |idx: usize| {
                        if let Some(t) = t_list.get(idx) {
                            is_selection_mode.set(true);
                            selected_tracks.write().insert(t.path.clone());
                        }
                    }
                },
                on_select: {
                    let t_list = tracks.clone();
                    move |(idx, selected): (usize, bool)| {
                        if let Some(t) = t_list.get(idx) {
                            if selected {
                                selected_tracks.write().insert(t.path.clone());
                            } else {
                                selected_tracks.write().remove(&t.path);
                                if selected_tracks.read().is_empty() {
                                    is_selection_mode.set(false);
                                }
                            }
                        }
                    }
                },
                active_track: active_menu_track.read().clone(),
                on_click_menu: {
                    let q = tracks.clone();
                    move |idx: usize| {
                        if let Some(t) = q.get(idx) {
                            if active_menu_track.read().as_ref() == Some(&t.path) {
                                active_menu_track.set(None);
                            } else {
                                active_menu_track.set(Some(t.path.clone()));
                            }
                        }
                    }
                },
                on_close_menu: move |_| active_menu_track.set(None),
                on_play: {
                    let q = tracks.clone();
                    move |idx: usize| {
                        queue.set(q.clone());
                        current_queue_index.set(idx);

                        if let Some(t) = q.get(idx) {
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                let (source, hint) = match decoder::open_file(&t.path) {
                                    Ok(s) => s,
                                    Err(_) => return,
                                };

                                let lib = library.peek();
                                let album_info = lib.albums.iter().find(|a| a.id == t.album_id);
                                let artwork = album_info.and_then(|a| {
                                    a.cover_path
                                        .as_ref()
                                        .map(|p| p.to_string_lossy().into_owned())
                                });

                                let meta = NowPlayingMeta {
                                    title: t.title.clone(),
                                    artist: t.artist.clone(),
                                    album: t.album.clone(),
                                    duration: std::time::Duration::from_secs(t.duration),
                                    artwork,
                                };
                                player.write().play(source, meta, hint);
                                current_song_title.set(t.title.clone());
                                current_song_artist.set(t.artist.clone());
                                current_song_duration.set(t.duration);
                                current_song_progress.set(0);
                                is_playing.set(true);

                                if let Some(album) = album_info {
                                    if let Some(url) =
                                        utils::format_artwork_url(album.cover_path.as_ref())
                                    {
                                        current_song_cover_url.set(url);
                                    } else {
                                        current_song_cover_url.set(String::new());
                                    }
                                } else {
                                    current_song_cover_url.set(String::new());
                                }
                            }
                            // Local file playback not supported on web
                            #[cfg(target_arch = "wasm32")]
                            let _ = t;
                        }
                    }
                },
                on_add_to_playlist: {
                    let q = tracks.clone();
                    move |idx: usize| {
                        if let Some(t) = q.get(idx) {
                            selected_track_for_playlist.set(Some(t.path.clone()));
                            show_playlist_modal.set(true);
                            active_menu_track.set(None);
                        }
                    }
                },
                on_delete_track: {
                    let q = tracks.clone();
                    move |idx: usize| {
                        if let Some(t) = q.get(idx) {
                            if std::fs::remove_file(&t.path).is_ok() {
                                library.write().remove_track(&t.path);
                                let cache_dir = std::path::Path::new("./cache").to_path_buf();
                                let lib_path = cache_dir.join("library.json");
                                let _ = library.read().save(&lib_path);
                            }
                            active_menu_track.set(None);
                        }
                    }
                }
            }

            if is_selection_mode() {
                crate::selection_bar::SelectionBar {
                    count: selected_tracks.read().len(),
                    on_add_to_playlist: move |_| {
                        show_playlist_modal.set(true);
                    },
                    on_delete: move |_| {
                        let paths: Vec<_> = selected_tracks.read().iter().cloned().collect();
                        for path in paths {
                            if std::fs::remove_file(&path).is_ok() {
                                library.write().remove_track(&path);
                            }
                        }
                        selected_tracks.write().clear();
                        is_selection_mode.set(false);
                        let cache_dir = std::path::Path::new("./cache").to_path_buf();
                        let lib_path = cache_dir.join("library.json");
                        let _ = library.read().save(&lib_path);
                    },
                    on_cancel: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    }
                }
            }

            if *show_playlist_modal.read() {
                crate::playlist_modal::PlaylistModal {
                    playlist_store: playlist_store,
                    is_jellyfin: false,
                    on_close: move |_| {
                        show_playlist_modal.set(false);
                        if is_selection_mode() {
                            is_selection_mode.set(false);
                            selected_tracks.write().clear();
                        }
                    },
                    on_add_to_playlist: move |playlist_id: String| {
                        let mut selected_paths = Vec::new();
                        if is_selection_mode() {
                            selected_paths = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            selected_paths.push(path);
                        }

                        if !selected_paths.is_empty() {
                            let mut store = playlist_store.write();
                            if let Some(playlist) = store.playlists.iter_mut().find(|p| p.id == playlist_id) {
                                for path in selected_paths {
                                    if !playlist.tracks.contains(&path) {
                                        playlist.tracks.push(path);
                                    }
                                }
                            }
                        }
                        show_playlist_modal.set(false);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_create_playlist: move |name: String| {
                        let mut selected_paths = Vec::new();
                        if is_selection_mode() {
                            selected_paths = selected_tracks.read().iter().cloned().collect();
                        } else if let Some(path) = selected_track_for_playlist.read().clone() {
                            selected_paths.push(path);
                        }

                        if !selected_paths.is_empty() {
                            let mut store = playlist_store.write();
                            store.playlists.push(reader::models::Playlist {
                                id: uuid::Uuid::new_v4().to_string(),
                                name,
                                tracks: selected_paths,
                            });
                        }
                        show_playlist_modal.set(false);
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    }
                }
            }
        }
    }
}
