use components::dots_menu::{DotsMenu, MenuAction};
use components::metadata_modal::MetadataModal;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use config::{AppConfig, ArtistPhotoSource, ArtistViewOrder};
use db::{Source, TrackFilter};
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_albums, use_all_tracks, use_artist_images, use_playlists};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

fn normalize_artist_key(value: &str) -> String {
    value.trim().to_lowercase()
}

#[component]
pub fn LocalArtist(
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    on_navigate: EventHandler<String>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let source = use_memo(|| Source::Local);
    let albums_res = use_albums(source);
    let all_tracks_filter = use_memo(|| TrackFilter::new(Source::Local));
    let all_tracks_res = use_all_tracks(all_tracks_filter);
    let artist_images_res = use_artist_images();
    let playlists_res = use_playlists();
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
    let mut metadata_track = use_signal(|| None::<reader::models::Track>);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());

    let mut open_album_menu = use_signal(|| None::<String>);
    let mut show_album_playlist_modal = use_signal(|| false);
    let mut pending_album_id_for_playlist = use_signal(|| None::<String>);

    let local_artists = use_memo(move || {
        let albums = albums_res.read().clone().unwrap_or_default();
        let tracks = all_tracks_res.read().clone().unwrap_or_default();
        let (_, local_artist_images, custom_artist_images) =
            artist_images_res.read().clone().unwrap_or_default();
        let use_artist_photo = config.read().artist_photo_source == ArtistPhotoSource::ArtistPhoto;
        let mut artist_map: HashMap<String, (String, Option<std::path::PathBuf>)> = HashMap::new();
        for album in &albums {
            artist_map
                .entry(normalize_artist_key(&album.artist))
                .or_insert_with(|| (album.artist.clone(), album.cover_path.clone()));
        }
        for track in &tracks {
            let cover = albums
                .iter()
                .find(|a| a.id == track.album_id)
                .and_then(|a| a.cover_path.clone());
            for artist in &track.artists {
                artist_map
                    .entry(normalize_artist_key(artist))
                    .or_insert_with(|| (artist.clone(), cover.clone()));
            }
        }
        if use_artist_photo {
            for (artist, image_path) in &local_artist_images {
                let normalized = normalize_artist_key(artist);
                let display_name = artist_map
                    .get(&normalized)
                    .map(|(display_name, _)| display_name.clone())
                    .unwrap_or_else(|| artist.clone());
                artist_map.insert(normalized, (display_name, Some(image_path.clone())));
            }
        }
        // Custom artist photos override both album cover and local_artist_images.
        for (artist, image_path) in &custom_artist_images {
            let normalized = normalize_artist_key(artist);
            let display_name = artist_map
                .get(&normalized)
                .map(|(display_name, _)| display_name.clone())
                .unwrap_or_else(|| artist.clone());
            artist_map.insert(normalized, (display_name, Some(image_path.clone())));
        }
        let mut artists: Vec<_> = artist_map.into_values().collect();
        artists.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        artists
    });

    let artist_tracks = use_memo(move || {
        let tracks = all_tracks_res.read().clone().unwrap_or_default();
        let albums = albums_res.read().clone().unwrap_or_default();
        let artist = artist_name.read();
        if artist.is_empty() {
            return Vec::new();
        }
        let artist_lc = artist.to_lowercase();
        let artist_album_ids: HashSet<String> = albums
            .iter()
            .filter(|a| a.artist.to_lowercase() == artist_lc)
            .map(|a| a.id.clone())
            .collect();
        tracks
            .iter()
            .filter(|t| {
                t.artists.iter().any(|a| a.to_lowercase() == artist_lc)
                    || artist_album_ids.contains(&t.album_id)
            })
            .cloned()
            .collect()
    });

    let artist_cover = use_memo(move || {
        let albums = albums_res.read().clone().unwrap_or_default();
        let (_, local_artist_images, custom_artist_images) =
            artist_images_res.read().clone().unwrap_or_default();
        let use_artist_photo = config.read().artist_photo_source == ArtistPhotoSource::ArtistPhoto;
        let artist = artist_name.read();
        if artist.is_empty() {
            return None;
        }
        let artist_lc = artist.to_lowercase();
        // Custom artist photo overrides every other source, regardless of config.
        if let Some(url) = custom_artist_images
            .iter()
            .find(|(name, _)| name.to_lowercase() == artist_lc)
            .and_then(|(_, path)| utils::format_artwork_url(Some(path)))
        {
            return Some(url);
        }
        if use_artist_photo {
            local_artist_images
                .iter()
                .find(|(name, _)| name.to_lowercase() == artist_lc)
                .map(|(_, path)| path)
                .and_then(|path| utils::format_artwork_url(Some(path)))
                .or_else(|| {
                    albums
                        .iter()
                        .find(|a| a.artist.to_lowercase() == artist_lc)
                        .and_then(|album| utils::format_artwork_url(album.cover_path.as_ref()))
                })
        } else {
            albums
                .iter()
                .find(|a| a.artist.to_lowercase() == artist_lc)
                .and_then(|album| utils::format_artwork_url(album.cover_path.as_ref()))
        }
    });

    let artist_albums = use_memo(move || {
        let all_albums = albums_res.read().clone().unwrap_or_default();
        let artist = artist_name.read();
        if artist.is_empty() {
            return Vec::new();
        }
        let artist_lc = artist.to_lowercase();
        let mut albums: Vec<_> = all_albums
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
    let add_tracks_to_playlist = move |playlist_id: String, paths: Vec<PathBuf>| {
        let store = playlists_res.read().clone().unwrap_or_default();
        let Some(playlist) = store.playlists.iter().find(|p| p.id == playlist_id) else {
            return;
        };
        let mut tracks = playlist.tracks.clone();
        for path in paths {
            if !tracks.contains(&path) {
                tracks.push(path);
            }
        }
        let refs: Vec<String> = tracks
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let db = consume_context::<db::Db>();
        spawn(async move {
            if db
                .set_playlist_tracks(&Source::Local, &playlist_id, &refs)
                .await
                .is_ok()
            {
                gens.bump(Table::Playlists);
            }
        });
    };

    let create_playlist = move |playlist_name: String, paths: Vec<PathBuf>| {
        let refs: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let id = uuid::Uuid::new_v4().to_string();
        let db = consume_context::<db::Db>();
        spawn(async move {
            if db
                .upsert_playlist_meta(&Source::Local, &id, &playlist_name, None, None)
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
    };

    let tracks_for_album = move |album_id: &str| -> Vec<PathBuf> {
        all_tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .iter()
            .filter(|t| t.album_id == album_id)
            .map(|t| t.id.uid_path())
            .collect()
    };

    let clear_selection =
        |is_selection_mode: &mut Signal<bool>, selected_tracks: &mut Signal<HashSet<PathBuf>>| {
            is_selection_mode.set(false);
            selected_tracks.write().clear();
        };

    rsx! {
        div {
            class: "flex-1 min-h-0 flex flex-col",
            if name.is_empty() {
                div { class: "flex-1 min-h-0 overflow-y-auto pb-20",
                    div { class: "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-8",
                    for (artist , cover_path) in local_artists() {
                        {
                            let cover_url = utils::format_artwork_thumb_url(cover_path.as_ref(), 320);
                            let art = artist.clone();
                            rsx! {
                                div {
                                    key: "{artist}",
                                    class: "group cursor-pointer flex flex-col items-center",
                                    style: "content-visibility: auto; contain-intrinsic-size: 0 180px;",
                                    onclick: move |_| artist_name.set(art.clone()),
                                    div {
                                        class: "aspect-square w-full rounded-full bg-stone-800 mb-4 overflow-hidden relative transition-all",
                                        style: "-webkit-user-drag: none;",
                                        ondragstart: move |evt| evt.prevent_default(),
                                        if let Some(url) = cover_url {
                                            img {
                                                src: "{url.as_ref()}",
                                                loading: "lazy",
                                                decoding: "async",
                                                draggable: "false",
                                                ondragstart: move |evt| evt.prevent_default(),
                                                class: "w-full h-full object-cover group-hover:scale-110 transition-transform duration-500"
                                            }
                                        } else {
                                            div { class: "w-full h-full flex items-center justify-center text-white/20",
                                                i { class: "fa-solid fa-microphone text-5xl" }
                                            }
                                        }
                                    }
                                    h3 { class: "text-white font-medium truncate text-center w-full group-hover:text-indigo-400 transition-colors", "{artist}" }
                                }
                            }
                        }
                    }
                    }
                }
            } else {
                div {
                    class: "relative flex-1 min-h-0 flex flex-col",
                    if *show_playlist_modal.read() {
                        PlaylistModal {
                            is_jellyfin: false,
                            overlay_class: Some("absolute inset-0 bg-black/80 flex items-center justify-center z-50".to_string()),
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

                    if let Some(track) = metadata_track.read().clone() {
                        MetadataModal {
                            track: track.clone(),
                            on_close: move |_| metadata_track.set(None),
                            on_save: move |edits: reader::models::TrackEdits| {
                                let Some(path) = track.id.local_path().map(|p| p.to_path_buf()) else {
                                    return;
                                };
                                match reader::write_tags(&path, &edits) {
                                    Ok(()) => {
                                        let mut t = track.clone();
                                        t.title = edits.title.trim().to_string();
                                        t.artist = edits.artist.trim().to_string();
                                        t.artists = edits
                                            .artist
                                            .split([';', ','])
                                            .map(|a| a.trim().to_string())
                                            .filter(|s| !s.is_empty())
                                            .collect();
                                        t.album = edits.album.trim().to_string();
                                        t.track_number = edits.track_number;
                                        t.disc_number = edits.disc_number;
                                        t.album_id = reader::metadata::make_album_id(
                                            edits.album.trim(),
                                            edits.artist.trim(),
                                        );
                                        let db = consume_context::<db::Db>();
                                        spawn(async move {
                                            if db.upsert_tracks(&Source::Local, &[t]).await.is_ok() {
                                                gens.bump(Table::Tracks);
                                            }
                                        });
                                        metadata_track.set(None);
                                    }
                                    Err(e) => {
                                        tracing::error!("failed to write tags for {}: {}", path.display(), e);
                                    }
                                }
                            },
                        }
                    }

                    if is_selection_mode() {
                        SelectionBar {
                            count: selected_tracks.read().len(),
                            class: Some("absolute bottom-24 left-1/2 -translate-x-1/2 bg-indigo-500 text-white px-6 py-2.5 rounded-full shadow-2xl flex items-center gap-4 z-50 animate-in fade-in zoom-in duration-200 font-mono".to_string()),
                            on_add_to_queue: move |_| {
                                let selected = selected_tracks.read().clone();
                                if selected.is_empty() {
                                    return;
                                }
                                let tracks: Vec<_> = artist_tracks()
                                    .iter()
                                    .filter(|t| selected.contains(&t.id.uid_path()))
                                    .cloned()
                                    .collect();

                                if !tracks.is_empty() {
                                    ctrl.add_to_queue(tracks);
                                }
                                clear_selection(&mut is_selection_mode, &mut selected_tracks);
                            },
                            on_add_to_playlist: move |_| show_playlist_modal.set(true),
                            on_delete: move |_| {
                                let paths: Vec<_> = selected_tracks.read().iter().cloned().collect();
                                let mut keys = Vec::new();
                                for path in &paths {
                                    if std::fs::remove_file(path).is_ok() {
                                        keys.push(path.to_string_lossy().into_owned());
                                    }
                                }
                                if !keys.is_empty() {
                                    let db = consume_context::<db::Db>();
                                    spawn(async move {
                                        if db.delete_tracks(&Source::Local, &keys).await.is_ok() {
                                            gens.bump(Table::Tracks);
                                        }
                                    });
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
                                is_jellyfin: false,
                                overlay_class: Some("absolute inset-0 bg-black/80 flex items-center justify-center z-50".to_string()),
                                on_close: move |_| show_album_playlist_modal.set(false),
                                on_add_to_playlist: move |playlist_id: String| {
                                    if let Some(album_id) = pending_album_id_for_playlist.read().clone() {
                                        let paths = tracks_for_album(&album_id);
                                        add_tracks_to_playlist(playlist_id, paths);
                                    }
                                    show_album_playlist_modal.set(false);
                                    pending_album_id_for_playlist.set(None);
                                },
                                on_create_playlist: move |playlist_name: String| {
                                    let paths = pending_album_id_for_playlist
                                        .read()
                                        .as_deref()
                                        .map(tracks_for_album)
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
                                let add_all_to_queue_text = i18n::t("add_all_to_queue").to_string();
                                let add_all_to_playlist_text = i18n::t("add_all_to_playlist").to_string();
                                let delete_album_text = i18n::t("delete_album").to_string();

                                let album_menu_actions = vec![
                                    MenuAction::new(add_all_to_queue_text.as_str(), "fa-solid fa-list-ul"),
                                    MenuAction::new(add_all_to_playlist_text.as_str(), "fa-solid fa-plus"),
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
                                                            div {
                                                                class: "aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                                                style: "-webkit-user-drag: none;",
                                                                ondragstart: move |evt| evt.prevent_default(),
                                                                if let Some(url) = &cover_url {
                                                                    img {
                                                                        src: "{url.as_ref()}",
                                                                        loading: "lazy",
                                                                        decoding: "async",
                                                                        draggable: "false",
                                                                        ondragstart: move |evt| evt.prevent_default(),
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
                                                                                let mut tracks_for_queue: Vec<_> = all_tracks_res
                                                                                    .read()
                                                                                    .clone()
                                                                                    .unwrap_or_default()
                                                                                    .into_iter()
                                                                                    .filter(|t| t.album_id == id)
                                                                                    .collect();

                                                                                tracks_for_queue.sort_by(|a, b| {
                                                                                    a.track_number
                                                                                        .cmp(&b.track_number)
                                                                                        .then_with(|| a.title.cmp(&b.title))
                                                                                });

                                                                                ctrl.add_to_queue(tracks_for_queue);
                                                                            }
                                                                            1 => {
                                                                                pending_album_id_for_playlist.set(Some(id.clone()));
                                                                                show_album_playlist_modal.set(true);
                                                                            }
                                                                            2 => {
                                                                                let tracks_to_delete: Vec<_> = all_tracks_res
                                                                                    .read()
                                                                                    .clone()
                                                                                    .unwrap_or_default()
                                                                                    .into_iter()
                                                                                    .filter(|t| t.album_id == id)
                                                                                    .collect();

                                                                                for track in &tracks_to_delete {
                                                                                    if let Some(path) = track.id.local_path() {
                                                                                        let _ = std::fs::remove_file(path);
                                                                                    }
                                                                                }

                                                                                let db = consume_context::<db::Db>();
                                                                                let album_id = id.clone();
                                                                                spawn(async move {
                                                                                    if db.delete_album(&Source::Local, &album_id).await.is_ok() {
                                                                                        gens.bump(Table::Tracks);
                                                                                        gens.bump(Table::Albums);
                                                                                    }
                                                                                });
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
                                description: String::new(),
                                cover_url: artist_cover(),
                                tracks: artist_tracks(),
                                on_cover_click: move |_| {
                                    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
                                    {
                                        let artist = artist_name.peek().clone();
                                        if artist.is_empty() {
                                            return;
                                        }
                                        spawn(async move {
                                            let file = rfd::AsyncFileDialog::new()
                                                .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                                                .pick_file()
                                                .await;
                                            if let Some(file) = file {
                                                let path = file.path().to_path_buf();
                                                let key = normalize_artist_key(&artist);
                                                let db = consume_context::<db::Db>();
                                                if db
                                                    .set_artist_image(
                                                        &key,
                                                        "custom",
                                                        Some(&path.to_string_lossy()),
                                                    )
                                                    .await
                                                    .is_ok()
                                                {
                                                    gens.bump(Table::Tracks);
                                                }
                                            }
                                        });
                                    }
                                },
                                active_track: active_menu_track.read().clone(),
                                is_selection_mode: is_selection_mode(),
                                selected_tracks: selected_tracks.read().clone(),
                                all_selected: !artist_tracks().is_empty() && artist_tracks().iter().all(|track| selected_tracks.read().contains(&track.id.uid_path())),
                                on_select_all: move |selected: bool| {
                                    if selected {
                                        selected_tracks.set(artist_tracks().into_iter().map(|track| track.id.uid_path()).collect());
                                        is_selection_mode.set(true);
                                    } else {
                                        selected_tracks.write().clear();
                                        is_selection_mode.set(false);
                                    }
                                },
                                on_long_press: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        is_selection_mode.set(true);
                                        selected_tracks.write().insert(track.id.uid_path());
                                    }
                                },
                                on_select: move |(idx, selected): (usize, bool)| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        if selected {
                                            is_selection_mode.set(true);
                                            selected_tracks.write().insert(track.id.uid_path());
                                        } else {
                                            selected_tracks.write().remove(&track.id.uid_path());
                                            if selected_tracks.read().is_empty() {
                                                is_selection_mode.set(false);
                                            }
                                        }
                                    }
                                },
                                on_play_all: move |_| {
                                       let is_shuffle = *ctrl.shuffle.peek();
                                       if is_shuffle {
                                           ctrl.play_queue_shuffled(artist_tracks());
                                       } else {
                                           ctrl.play_queue_linear(artist_tracks());
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
                                        let path = track.id.uid_path();
                                        let already_open = active_menu_track.read().as_ref() == Some(&path);
                                        active_menu_track.set((!already_open).then(|| path.clone()));
                                    }
                                },
                                on_close_menu: move |_| active_menu_track.set(None),
                                on_add_to_playlist: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        selected_track_for_playlist.set(Some(track.id.uid_path()));
                                        show_playlist_modal.set(true);
                                        active_menu_track.set(None);
                                    }
                                },
                                on_queue: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        ctrl.add_to_queue(vec![track.clone()]);
                                        active_menu_track.set(None);
                                    }
                                },
                                on_view_metadata: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        metadata_track.set(Some(track.clone()));
                                        active_menu_track.set(None);
                                    }
                                },
                                on_delete_track: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        if let Some(p) = track.id.local_path() {
                                            if std::fs::remove_file(p).is_ok() {
                                                let db = consume_context::<db::Db>();
                                                let key = track.id.key().into_owned();
                                                spawn(async move {
                                                    if db.delete_tracks(&Source::Local, &[key]).await.is_ok() {
                                                        gens.bump(Table::Tracks);
                                                    }
                                                });
                                            }
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

    let btn_active = "inline-flex items-center justify-center h-7 px-3 text-xs rounded-md bg-white/10 text-white font-medium transition-all";
    let btn_inactive = "inline-flex items-center justify-center h-7 px-3 text-xs rounded-md text-white/40 hover:text-white/80 transition-all";

    rsx! {
        div { class: "inline-flex items-center h-9 p-1 space-x-1 bg-white/5 border border-white/5 rounded-full",
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
