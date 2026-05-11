use components::dots_menu::{DotsMenu, MenuAction};
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use config::{AppConfig, ArtistViewOrder};
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[component]
pub fn LocalArtist(
    library: Signal<Library>,
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    playlist_store: Signal<PlaylistStore>,
    on_navigate: EventHandler<String>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let sort_order = use_signal(move || config.read().artist_view_order.clone());
    use_effect(move || {
        let curr = sort_order.read().clone();
        if config.peek().artist_view_order != curr {
            config.write().artist_view_order = curr;
        }
    });

    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();

    let mut show_playlist_modal = use_signal(|| false);
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());

    let mut open_album_menu = use_signal(|| None::<String>);
    let mut show_album_playlist_modal = use_signal(|| false);
    let mut pending_album_id_for_playlist = use_signal(|| None::<String>);

    let local_artists = use_memo(move || {
        let lib = library.read();
        let mut artist_map: HashMap<String, Option<std::path::PathBuf>> = HashMap::new();
        for album in &lib.albums {
            artist_map
                .entry(album.artist.clone())
                .or_insert_with(|| album.cover_path.clone());
        }
        for track in &lib.tracks {
            let cover = lib
                .albums
                .iter()
                .find(|a| a.id == track.album_id)
                .and_then(|a| a.cover_path.clone());
            for artist in &track.artists {
                artist_map.entry(artist.clone()).or_insert_with(|| cover.clone());
            }
        }
        let mut artists: Vec<_> = artist_map.into_iter().collect();
        artists.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        artists
    });

    let artist_tracks = use_memo(move || {
        let lib = library.read();
        let artist = artist_name.read();
        if artist.is_empty() {
            return Vec::new();
        }
        let artist_lc = artist.to_lowercase();
        let artist_album_ids: HashSet<String> = lib
            .albums
            .iter()
            .filter(|a| a.artist.to_lowercase() == artist_lc)
            .map(|a| a.id.clone())
            .collect();
        lib.tracks
            .iter()
            .filter(|t| {
                t.artists.iter().any(|a| a.to_lowercase() == artist_lc)
                    || artist_album_ids.contains(&t.album_id)
            })
            .cloned()
            .collect()
    });

    let artist_cover = use_memo(move || {
        let lib = library.read();
        let artist = artist_name.read();
        if artist.is_empty() {
            return None;
        }
        let artist_lc = artist.to_lowercase();
        lib.albums
            .iter()
            .find(|a| a.artist.to_lowercase() == artist_lc)
            .and_then(|album| utils::format_artwork_url(album.cover_path.as_ref()))
    });

    let artist_albums = use_memo(move || {
        let lib = library.read();
        let artist = artist_name.read();
        if artist.is_empty() {
            return Vec::new();
        }
        let artist_lc = artist.to_lowercase();
        let mut albums: Vec<_> = lib
            .albums
            .iter()
            .filter(|a| a.artist.to_lowercase() == artist_lc)
            .cloned()
            .collect();
        albums.sort_by(|a, b| {
            a.title
                .trim()
                .to_lowercase()
                .cmp(&b.title.trim().to_lowercase())
        });
        let mut seen = HashSet::new();
        albums.retain(|album| seen.insert(album.title.trim().to_lowercase()));
        albums
    });

    let name = artist_name.read().clone();

    let mut add_tracks_to_playlist = move |playlist_id: String, paths: Vec<PathBuf>| {
        let mut store = playlist_store.write();
        if let Some(playlist) = store.playlists.iter_mut().find(|p| p.id == playlist_id) {
            for path in paths {
                if !playlist.tracks.contains(&path) {
                    playlist.tracks.push(path);
                }
            }
        }
    };

    let mut create_playlist = move |playlist_name: String, paths: Vec<PathBuf>| {
        playlist_store
            .write()
            .playlists
            .push(reader::models::Playlist {
                id: uuid::Uuid::new_v4().to_string(),
                name: playlist_name,
                tracks: paths,
                cover_path: None,
            });
    };

    let tracks_for_album = |library: &Library, album_id: &str| -> Vec<PathBuf> {
        library
            .tracks
            .iter()
            .filter(|t| t.album_id == album_id)
            .map(|t| t.path.clone())
            .collect()
    };

    let clear_selection =
        |is_selection_mode: &mut Signal<bool>, selected_tracks: &mut Signal<HashSet<PathBuf>>| {
            is_selection_mode.set(false);
            selected_tracks.write().clear();
        };

    rsx! {
        div {
            if name.is_empty() {
                div { class: "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-8",
                    for (artist , cover_path) in local_artists() {
                        {
                            let cover_url = utils::format_artwork_url(cover_path.as_ref());
                            let art = artist.clone();
                            rsx! {
                                div {
                                    key: "{artist}",
                                    class: "group cursor-pointer flex flex-col items-center",
                                    style: "content-visibility: auto; contain-intrinsic-size: 0 180px;",
                                    onclick: move |_| artist_name.set(art.clone()),
                                    div { class: "aspect-square w-full rounded-full bg-stone-800 mb-4 overflow-hidden relative transition-all",
                                        if let Some(url) = cover_url {
                                            img {
                                                src: "{url.as_ref()}",
                                                loading: "lazy",
                                                class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-500"
                                            }
                                        } else {
                                            div { class: "w-full h-full flex items-center justify-center text-white/20",
                                                i { class: "fa-solid fa-microphone text-5xl" }
                                            }
                                        }
                                    }
                                    h3 { class: "text-white font-medium truncate text-center w-full group-hover:text-indigo-400 transition-colors", "{artist}" }
                                    p { class: "text-xs text-slate-500 uppercase tracking-wider mt-1", "{i18n::t(\"artist\")}" }
                                }
                            }
                        }
                    }
                }
            } else {
                div {
                    if *show_playlist_modal.read() {
                        PlaylistModal {
                            playlist_store,
                            is_jellyfin: false,
                            on_close: move |_| {
                                show_playlist_modal.set(false);
                                clear_selection(&mut is_selection_mode, &mut selected_tracks);
                            },
                            on_add_to_playlist: move |playlist_id: String| {
                                let paths = if is_selection_mode() {
                                    selected_tracks.read().iter().cloned().collect()
                                } else {
                                    selected_track_for_playlist.read().iter().cloned().collect()
                                };
                                add_tracks_to_playlist(playlist_id, paths);
                                show_playlist_modal.set(false);
                                active_menu_track.set(None);
                                clear_selection(&mut is_selection_mode, &mut selected_tracks);
                            },
                            on_create_playlist: move |playlist_name: String| {
                                let paths = if is_selection_mode() {
                                    selected_tracks.read().iter().cloned().collect()
                                } else {
                                    selected_track_for_playlist.read().iter().cloned().collect()
                                };
                                create_playlist(playlist_name, paths);
                                show_playlist_modal.set(false);
                                active_menu_track.set(None);
                                clear_selection(&mut is_selection_mode, &mut selected_tracks);
                            },
                        }
                    }

                    if is_selection_mode() {
                        SelectionBar {
                            count: selected_tracks.read().len(),
                            on_add_to_playlist: move |_| show_playlist_modal.set(true),
                            on_delete: move |_| {
                                let paths: Vec<_> = selected_tracks.read().iter().cloned().collect();
                                for path in &paths {
                                    if std::fs::remove_file(path).is_ok() {
                                        library.write().remove_track(path);
                                    }
                                }
                                clear_selection(&mut is_selection_mode, &mut selected_tracks);
                            },
                            on_cancel: move |_| {
                                clear_selection(&mut is_selection_mode, &mut selected_tracks);
                            },
                        }
                    }

                    if *sort_order.read() == ArtistViewOrder::Albums {
                        if *show_album_playlist_modal.read() {
                            PlaylistModal {
                                playlist_store,
                                is_jellyfin: false,
                                on_close: move |_| show_album_playlist_modal.set(false),
                                on_add_to_playlist: move |playlist_id: String| {
                                    if let Some(album_id) = pending_album_id_for_playlist.read().clone() {
                                        let lib = library.read();
                                        let paths = tracks_for_album(&lib, &album_id);
                                        drop(lib);
                                        add_tracks_to_playlist(playlist_id, paths);
                                    }
                                    show_album_playlist_modal.set(false);
                                    pending_album_id_for_playlist.set(None);
                                },
                                on_create_playlist: move |playlist_name: String| {
                                    let paths = pending_album_id_for_playlist
                                        .read()
                                        .as_deref()
                                        .map(|id| {
                                            let lib = library.read();
                                            tracks_for_album(&lib, id)
                                        })
                                        .unwrap_or_default();
                                    create_playlist(playlist_name, paths);
                                    show_album_playlist_modal.set(false);
                                    pending_album_id_for_playlist.set(None);
                                },
                            }
                        }

                        SortOrderToggle { sort_order }

                        if artist_albums().is_empty() {
                            p { class: "text-slate-500", "{i18n::t(\"no_albums_found\")}" }
                        } else {
                            {
                                let add_all_to_playlist_text = i18n::t("add_all_to_playlist").to_string();
                                let delete_album_text = i18n::t("delete_album").to_string();
                                let album_menu_actions = vec![
                                    MenuAction::new(add_all_to_playlist_text.as_str(), "fa-solid fa-list-music"),
                                    MenuAction::new(delete_album_text.as_str(), "fa-solid fa-trash").destructive(),
                                ];
                                rsx! {
                                    div { class: "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-6",
                                        for album in artist_albums() {
                                            {
                                                let id_for_menu = album.id.clone();
                                                let id_for_action = album.id.clone();
                                                let is_open = open_album_menu.read().as_deref() == Some(&album.id);
                                                let cover_url = utils::format_artwork_url(album.cover_path.as_ref());
                                                rsx! {
                                                    div {
                                                        key: "{album.id}",
                                                        class: "group relative p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors",
                                                        onclick: move |_| on_navigate.call(album.id.clone()),
                                                        oncontextmenu: {
                                                            let id = id_for_menu.clone();
                                                            move |evt| {
                                                                evt.prevent_default();
                                                                open_album_menu.set(Some(id.clone()));
                                                            }
                                                        },
                                                        div { class: "cursor-pointer",
                                                            div { class: "aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                                                if let Some(url) = &cover_url {
                                                                    img {
                                                                        src: "{url.as_ref()}",
                                                                        loading: "lazy",
                                                                        class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-300",
                                                                    }
                                                                } else {
                                                                    div { class: "w-full h-full flex items-center justify-center",
                                                                        i { class: "fa-solid fa-compact-disc text-4xl text-white/20" }
                                                                    }
                                                                }
                                                            }
                                                            h3 { class: "text-white font-medium truncate", "{album.title}" }
                                                            p { class: "text-sm text-stone-400 truncate", "{album.artist}" }
                                                        }

                                                        div { class: "absolute bottom-3 right-3",
                                                            DotsMenu {
                                                                actions: album_menu_actions.clone(),
                                                                is_open,
                                                                on_open: {
                                                                    let id = id_for_menu.clone();
                                                                    move |_| open_album_menu.set(Some(id.clone()))
                                                                },
                                                                on_close: move |_| open_album_menu.set(None),
                                                                button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100 bg-black/40".to_string(),
                                                                anchor: "right".to_string(),
                                                                on_action: {
                                                                    let id = id_for_action.clone();
                                                                    move |idx: usize| {
                                                                        open_album_menu.set(None);
                                                                        match idx {
                                                                            0 => {
                                                                                pending_album_id_for_playlist.set(Some(id.clone()));
                                                                                show_album_playlist_modal.set(true);
                                                                            }
                                                                            1 => {
                                                                                let tracks_to_delete: Vec<_> = library
                                                                                    .read()
                                                                                    .tracks
                                                                                    .iter()
                                                                                    .filter(|t| t.album_id == id)
                                                                                    .map(|t| t.path.clone())
                                                                                    .collect();
                                                                                for path in &tracks_to_delete {
                                                                                    let _ = std::fs::remove_file(path);
                                                                                }
                                                                                let mut lib = library.write();
                                                                                lib.albums.retain(|a| a.id != id);
                                                                                lib.tracks.retain(|t| t.album_id != id);
                                                                            }
                                                                            _ => {}
                                                                        }
                                                                    }
                                                                },
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        if artist_tracks().is_empty() {
                            div { class: "flex flex-col items-center justify-center h-64 text-slate-500",
                                i { class: "fa-regular fa-music text-4xl mb-4 opacity-30" }
                                p { class: "text-base", "{i18n::t(\"no_tracks_found\")}" }
                            }
                        } else {
                            components::showcase::Showcase {
                                name: name.clone(),
                                description: i18n::t("artist").to_string(),
                                cover_url: artist_cover(),
                                tracks: artist_tracks(),
                                library,
                                active_track: active_menu_track.read().clone(),
                                is_selection_mode: is_selection_mode(),
                                selected_tracks: selected_tracks.read().clone(),
                                on_long_press: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        is_selection_mode.set(true);
                                        selected_tracks.write().insert(track.path.clone());
                                    }
                                },
                                on_select: move |(idx, selected): (usize, bool)| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        if selected {
                                            selected_tracks.write().insert(track.path.clone());
                                        } else {
                                            selected_tracks.write().remove(&track.path);
                                            if selected_tracks.read().is_empty() {
                                                is_selection_mode.set(false);
                                            }
                                        }
                                    }
                                },
                                on_play: move |idx: usize| {
                                    let tracks = artist_tracks();
                                    queue.set(tracks.clone());
                                    current_queue_index.set(idx);
                                    ctrl.play_track(idx);
                                },
                                on_click_menu: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        let path = &track.path;
                                        let already_open = active_menu_track.read().as_ref() == Some(path);
                                        active_menu_track.set((!already_open).then(|| path.clone()));
                                    }
                                },
                                on_close_menu: move |_| active_menu_track.set(None),
                                on_add_to_playlist: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        selected_track_for_playlist.set(Some(track.path.clone()));
                                        show_playlist_modal.set(true);
                                        active_menu_track.set(None);
                                    }
                                },
                                on_delete_track: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        if std::fs::remove_file(&track.path).is_ok() {
                                            library.write().remove_track(&track.path);
                                        }
                                    }
                                    active_menu_track.set(None);
                                },
                                actions: Some(rsx! {
                                    SortOrderToggle { sort_order }
                                }),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SortOrderToggle(mut sort_order: Signal<ArtistViewOrder>) -> Element {
    let is_tracks = *sort_order.read() == ArtistViewOrder::Tracks;

    let btn_active =
        "px-3 py-1 text-xs rounded-md bg-white/10 text-white font-medium transition-all";
    let btn_inactive =
        "px-3 py-1 text-xs rounded-md text-white/40 hover:text-white/80 transition-all";

    rsx! {
        div { class: "flex justify-end mb-4",
            div { class: "flex space-x-1 bg-white/5 border border-white/5 p-1 rounded-lg",
                button {
                    class: if is_tracks { btn_active } else { btn_inactive },
                    onclick: move |_| sort_order.set(ArtistViewOrder::Tracks),
                    "{i18n::t(\"tracks\")}"
                }
                button {
                    class: if !is_tracks { btn_active } else { btn_inactive },
                    onclick: move |_| sort_order.set(ArtistViewOrder::Albums),
                    "{i18n::t(\"albums\")}"
                }
            }
        }
    }
}
