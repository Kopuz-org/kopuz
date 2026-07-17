//! Source-agnostic Artists page (issue #35). One component renders any source:
//! the data path is source-scoped query hooks, covers/images resolve through the
//! source layer (`server::cover`), and the few divergent affordances (tag edit,
//! delete-from-disk, downloads, playlist mutation) gate on the resolved source's
//! [`Capabilities`](server::source::Capabilities) — never on `is_server()`.

use components::dots_menu::{DotsMenu, MenuAction};
use components::metadata_modal::MetadataModal;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::sort_control::SortControl;
use components::view_mode_toggle::ViewModeToggle;
use config::{
    AlbumSortField, AlbumViewMode, AppConfig, ArtistPhotoSource, ArtistSortField, ArtistViewOrder,
    SortDirection,
};
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{
    use_active_source, use_albums, use_artist_images, use_artist_sample_tracks, use_artist_tracks,
    use_artists, use_tracks_by_keys,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tracing::Instrument;

use crate::server::download_manager::{DownloadQueue, delete_downloads, queue_downloads};

fn normalize_artist_key(value: &str) -> String {
    value.trim().to_lowercase()
}

/// One album-card menu entry, tagged so dispatch survives the entry set being
/// built dynamically from capabilities (indices shift as entries are gated in).
#[derive(Clone, Copy, PartialEq, Eq)]
enum AlbumAction {
    Queue,
    Playlist,
    DeleteAlbum,
    Download { downloaded: bool },
}

#[component]
pub fn Artist(
    config: Signal<AppConfig>,
    artist_name: Signal<String>,
    player: Signal<player::player::Player>,
    on_navigate: EventHandler<String>,
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
    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let nav_ctrl = use_context::<components::NavigationController>();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    // Capabilities, read off the resolved source — the single seam the page gates
    // its divergent affordances on (no `is_server()` / `match service`).
    let caps = use_memo(move || active_source.read().capabilities());
    // Diagnostic (debug): what source/caps this page is actually rendering, logged
    // whenever they change — confirms the page follows the sidebar source toggle.
    use_effect(move || {
        tracing::debug!(target: "kopuz::source", source = %source().as_str(), caps = ?caps(), "artist page source");
    });

    let is_offline = use_context::<Signal<bool>>();
    let download_queue = use_context::<Signal<DownloadQueue>>();
    let mut fetched_artist_images =
        use_context::<Signal<std::collections::HashMap<String, String>>>();
    // In-flight guard for the remote artist-image fetch — page-local; the persisted
    // `fetched_artist_images` map is what skips a refetch across navigations.
    let mut is_fetching_images = use_signal(|| false);
    // Records that a fetch already ran this mount. Distinguishes "fetched, found
    // nothing" (e.g. YT yields no images) from "never fetched" — without it an
    // empty result leaves the map empty and the effect respawns forever.
    let mut images_fetch_done = use_signal(|| false);

    let albums_res = use_albums(source);
    let artist_counts_res = use_artists(source);
    let sample_tracks_res = use_artist_sample_tracks(source, u32::MAX);
    let artist_memo = use_memo(move || artist_name.read().clone());
    let artist_tracks_res = use_artist_tracks(source, artist_memo);
    let artist_images_res = use_artist_images();

    // Server + offline: keys of tracks downloaded for offline, used to restrict the
    // artist/album listing to what's actually available. Empty otherwise (cheap).
    let offline_keys = use_memo(move || -> Vec<String> {
        if !caps().downloads || !*is_offline.read() {
            return Vec::new();
        }
        config
            .read()
            .offline_tracks
            .iter()
            .filter(|(_, path)| std::path::Path::new(path).exists())
            .map(|(id, _)| id.clone())
            .collect()
    });
    let offline_tracks_res = use_tracks_by_keys(source, offline_keys);

    let sort_order = use_signal(move || config.read().artist_view_order.clone());
    use_effect(move || {
        let curr = sort_order.read().clone();
        if config.peek().artist_view_order != curr {
            config.write().artist_view_order = curr;
        }
    });

    let album_sort = use_signal(|| config.peek().artist_album_sort.clone());
    use_effect(move || {
        let curr = album_sort.read().clone();
        if config.peek().artist_album_sort != curr {
            config.write().artist_album_sort = curr;
        }
    });

    let artist_sort = use_signal(|| config.peek().artist_sort.clone());
    use_effect(move || {
        let curr = artist_sort.read().clone();
        if config.peek().artist_sort != curr {
            config.write().artist_sort = curr;
        }
    });

    let album_view_mode = use_signal(|| config.peek().artist_album_view_mode);
    use_effect(move || {
        let curr = *album_view_mode.read();
        if config.peek().artist_album_view_mode != curr {
            config.write().artist_album_view_mode = curr;
        }
    });

    let artists_view_mode = use_signal(|| config.peek().artists_view_mode);
    use_effect(move || {
        let curr = *artists_view_mode.read();
        if config.peek().artists_view_mode != curr {
            config.write().artists_view_mode = curr;
        }
    });

    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();

    let mut show_playlist_modal = use_signal(|| false);
    let mut active_menu_track = use_signal(|| None::<reader::TrackId>);
    let mut selected_track_for_playlist = use_signal(|| None::<reader::TrackId>);
    let mut metadata_track = use_signal(|| None::<reader::models::Track>);

    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(HashSet::<reader::TrackId>::new);

    let mut open_album_menu = use_signal(|| None::<String>);
    let mut show_album_playlist_modal = use_signal(|| false);
    let mut pending_album_id_for_playlist = use_signal(|| None::<String>);

    // Remote artist-image fetch (servers only): fills the in-memory cache the
    // image resolution reads. Gated on `sync` so local never runs it; YT yields
    // nothing. The per-service fetch lives behind the facade.
    use_effect(move || {
        // YT resolves its avatars per-artist below; this server path would yield
        // nothing for it anyway.
        if !caps().sync || caps().albums == ::server::source::AlbumType::YtMusic {
            return;
        }
        if config.read().artist_photo_source != ArtistPhotoSource::ArtistPhoto {
            return;
        }
        if *is_fetching_images.read()
            || *images_fetch_done.read()
            || !fetched_artist_images.read().is_empty()
        {
            return;
        }
        is_fetching_images.set(true);
        let source = active_source.peek().clone();
        spawn(
            async move {
                let images: std::collections::HashMap<String, String> = source
                    .fetch_artist_images()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                fetched_artist_images.set(images);
                images_fetch_done.set(true);
                is_fetching_images.set(false);
            }
            .instrument(tracing::info_span!("artist.fetch_images")),
        );
    });

    // YT Music: the Artists grid shows real YT artist photos and nothing else.
    // There's no bulk endpoint, so resolve each library artist's avatar from the
    // YT "Artists" search, a few in flight at a time, writing results in as they
    // land so the grid fills progressively. Keyed by display name (the grid reads
    // `fetched.get(&display)`).
    use_effect(move || {
        if caps().albums != ::server::source::AlbumType::YtMusic {
            return;
        }
        if config.read().artist_photo_source != ArtistPhotoSource::ArtistPhoto {
            return;
        }
        if *images_fetch_done.read() || *is_fetching_images.read() {
            return;
        }
        // Wait for the DB artist-image cache to load so we can skip artists whose
        // photo was persisted on a previous run (otherwise we'd refetch every
        // time the page opens).
        let db_imgs = artist_images_res.read();
        let Some((_, db_photos)) = db_imgs.clone() else {
            return;
        };
        drop(db_imgs);

        let albums = albums_res.read().clone().unwrap_or_default();
        let sample = sample_tracks_res.read().clone().unwrap_or_default();
        if albums.is_empty() && sample.is_empty() {
            // Library not loaded yet — wait for a real artist set.
            return;
        }
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for album in &albums {
            if !album.artist.trim().is_empty() {
                names.insert(album.artist.clone());
            }
        }
        for track in &sample {
            for artist in &track.artists {
                if !artist.trim().is_empty() {
                    names.insert(artist.clone());
                }
            }
        }
        // Skip artists already resolved this session (context map) or persisted
        // to the DB on a previous run (keyed by normalized name).
        let already = fetched_artist_images.read();
        let names: Vec<String> = names
            .into_iter()
            .filter(|n| {
                !already.contains_key(n) && !db_photos.contains_key(&normalize_artist_key(n))
            })
            .collect();
        drop(already);
        if names.is_empty() {
            images_fetch_done.set(true);
            return;
        }
        // Mark done up front so the effect doesn't respawn as the workers write
        // partial results back into the map.
        is_fetching_images.set(true);
        images_fetch_done.set(true);

        let workers = 6usize;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(names.into_iter()));
        for _ in 0..workers {
            let source = active_source.peek().clone();
            let shared = shared.clone();
            spawn(
                async move {
                    while let Some(name) = shared.lock().ok().and_then(|mut it| it.next()) {
                        // Always record an outcome so the grid can tell "resolved,
                        // no photo" (→ album fallback) from "still loading"
                        // (→ placeholder). "" is the no-photo sentinel.
                        let url = source
                            .fetch_artist_image(&name)
                            .await
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        // Persist found photos to the DB (kind "server" → the
                        // grid's `photos` map) so future opens load them instantly
                        // instead of re-searching YT.
                        if !url.is_empty() {
                            let _ = source
                                .set_artist_image(
                                    &normalize_artist_key(&name),
                                    "server",
                                    Some(&url),
                                )
                                .await;
                        }
                        fetched_artist_images.write().insert(name, url);
                    }
                }
                .instrument(tracing::info_span!("artist.fetch_yt_images")),
            );
        }
        is_fetching_images.set(false);
    });

    // The artist grid: every artist with a resolved avatar. Avatar precedence is
    // source-agnostic — custom photo, then (when "artist photo" is on) a fetched
    // remote / local-DB photo, then the album cover via the source cover seam.
    let artists = use_memo(move || -> Vec<(String, Option<utils::CoverUrl>)> {
        let albums = albums_res.read().clone().unwrap_or_default();
        let sample = sample_tracks_res.read().clone().unwrap_or_default();
        let (overrides, photos) = artist_images_res.read().clone().unwrap_or_default();
        let fetched = fetched_artist_images.read();
        let conf = config.read();
        let use_photo = conf.artist_photo_source == ArtistPhotoSource::ArtistPhoto;
        // YT resolves photos one artist at a time. Until an artist resolves we
        // render a placeholder rather than its album cover, so the card doesn't
        // visibly swap album→photo (which read as a loading glitch). The map
        // carries a "" sentinel for "resolved, no photo" → album fallback.
        let is_yt = caps().albums == ::server::source::AlbumType::YtMusic;
        let offline = caps().downloads && *is_offline.read();

        // norm → (display name, first album cover-path).
        let mut artist_map: HashMap<String, (String, Option<PathBuf>)> = HashMap::new();
        for album in &albums {
            artist_map
                .entry(normalize_artist_key(&album.artist))
                .or_insert_with(|| (album.artist.clone(), album.cover_path.clone()));
        }
        for track in &sample {
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

        let downloaded: HashSet<String> = if offline {
            offline_tracks_res
                .read()
                .clone()
                .unwrap_or_default()
                .iter()
                .map(|t| t.artist.to_lowercase())
                .collect()
        } else {
            HashSet::new()
        };

        // Per-artist counts for the count-based sort fields; keyed by the
        // normalized name so differently-cased credits collapse into one bucket.
        let mut track_counts: HashMap<String, u32> = HashMap::new();
        for (name, n) in artist_counts_res.read().clone().unwrap_or_default() {
            *track_counts.entry(normalize_artist_key(&name)).or_default() += n;
        }
        let mut album_counts: HashMap<String, u32> = HashMap::new();
        for album in &albums {
            *album_counts
                .entry(normalize_artist_key(&album.artist))
                .or_default() += 1;
        }

        let out: Vec<(String, Option<utils::CoverUrl>)> = artist_map
            .into_iter()
            .filter(|(_, (display, _))| !offline || downloaded.contains(&display.to_lowercase()))
            .map(|(norm, (display, album_cover))| {
                // YT + photos on, artist not resolved yet → placeholder (no album
                // flash). Unless the user set a custom override / DB photo exists.
                if is_yt
                    && use_photo
                    && !fetched.contains_key(&display)
                    && !overrides.contains_key(&norm)
                    && !photos.contains_key(&norm)
                {
                    return (display, None);
                }
                // "" sentinel = resolved with no photo → fall back to album cover.
                let fetched_url = fetched
                    .get(&display)
                    .map(|u| u.as_str())
                    .filter(|u| !u.is_empty());
                let cover = ::server::cover::artist(
                    &conf,
                    overrides.get(&norm).map(|p| p.as_path()),
                    photos.get(&norm),
                    fetched_url,
                    album_cover.as_deref(),
                    use_photo,
                    320,
                );
                (display, cover)
            })
            .collect();
        // Decorate with the normalized name once, sort by the stacked criteria
        // (name always breaks remaining ties), then strip the key back off.
        let criteria = artist_sort.read().clone();
        let mut keyed: Vec<(String, (String, Option<utils::CoverUrl>))> = out
            .into_iter()
            .map(|entry| (normalize_artist_key(&entry.0), entry))
            .collect();
        keyed.sort_by(|(ka, _), (kb, _)| {
            for c in &criteria {
                let ord = match c.field {
                    ArtistSortField::Name => ka.cmp(kb),
                    ArtistSortField::Tracks => track_counts
                        .get(ka)
                        .copied()
                        .unwrap_or(0)
                        .cmp(&track_counts.get(kb).copied().unwrap_or(0)),
                    ArtistSortField::Albums => album_counts
                        .get(ka)
                        .copied()
                        .unwrap_or(0)
                        .cmp(&album_counts.get(kb).copied().unwrap_or(0)),
                };
                let ord = match c.direction {
                    SortDirection::Asc => ord,
                    SortDirection::Desc => ord.reverse(),
                };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            ka.cmp(kb)
        });
        keyed.into_iter().map(|(_, entry)| entry).collect()
    });

    // Restore the grid's scroll position once, after the artist list first
    // renders. Guarded so the incremental photo loads (which re-run the memo)
    // don't keep yanking the view back to the saved offset.
    let mut scroll_restored = use_signal(|| false);
    use_effect(move || {
        if *scroll_restored.read() || !artist_name.peek().is_empty() {
            return;
        }
        if artists().is_empty() {
            return;
        }
        scroll_restored.set(true);
        let _ = dioxus::document::eval(&crate::scroll_persist::restore_eval(
            "artist-grid-scroll",
            "artists",
        ));
    });

    let artist_tracks = use_memo(move || {
        if artist_name.read().is_empty() {
            return Vec::new();
        }
        let tracks = artist_tracks_res.read().clone().unwrap_or_default();
        if !(caps().downloads && *is_offline.read()) {
            return tracks;
        }
        let conf = config.read();
        tracks
            .into_iter()
            .filter(|t| {
                let id = t.id.key();
                conf.offline_tracks
                    .get(id.as_ref())
                    .map(|p| std::path::Path::new(p).exists())
                    .unwrap_or(false)
            })
            .collect()
    });

    let artist_cover = use_memo(move || {
        let artist = artist_name.read();
        if artist.is_empty() {
            return None;
        }
        let norm = normalize_artist_key(&artist);
        let (overrides, photos) = artist_images_res.read().clone().unwrap_or_default();
        let fetched = fetched_artist_images.read();
        let conf = config.read();
        let use_photo = conf.artist_photo_source == ArtistPhotoSource::ArtistPhoto;
        let album_cover = albums_res
            .read()
            .clone()
            .unwrap_or_default()
            .iter()
            .find(|a| a.artist.to_lowercase() == artist.to_lowercase())
            .and_then(|a| a.cover_path.clone());
        ::server::cover::artist(
            &conf,
            overrides.get(&norm).map(|p| p.as_path()),
            photos.get(&norm),
            fetched.get(artist.as_str()).map(|u| u.as_str()),
            album_cover.as_deref(),
            use_photo,
            512,
        )
    });

    let artist_albums = use_memo(move || {
        let artist = artist_name.read();
        if artist.is_empty() {
            return Vec::new();
        }
        let artist_lc = artist.to_lowercase();
        let all_albums = albums_res.read().clone().unwrap_or_default();
        let offline = caps().downloads && *is_offline.read();
        let downloaded_ids: HashSet<String> = if offline {
            offline_tracks_res
                .read()
                .clone()
                .unwrap_or_default()
                .iter()
                .map(|t| t.album_id.clone())
                .collect()
        } else {
            HashSet::new()
        };
        let mut albums: Vec<_> = all_albums
            .iter()
            .filter(|a| a.artist.to_lowercase() == artist_lc)
            .filter(|a| !offline || downloaded_ids.contains(&a.id))
            .cloned()
            .collect();
        reader::sort::sort_albums(&mut albums, &album_sort.read());
        let mut seen = HashSet::new();
        albums.retain(|album| seen.insert(album.title.trim().to_lowercase()));
        albums
    });

    // Every album here shares the artist, so that field would never break a tie.
    let album_sort_fields = use_memo(move || {
        let mut fields = reader::sort::available_album_fields(&artist_albums.read());
        fields.retain(|f| *f != AlbumSortField::Artist);
        fields
    });

    let name = artist_name.read().clone();
    let page_container_class = crate::layout::page_container_class(&config.read().ui_style);

    // The refs (item ids / local paths) of the currently-selected tracks — derived
    // from the in-hand `Track`s via the typed id, so it's source-uniform.
    let refs_for = move |paths: &HashSet<reader::TrackId>| -> Vec<String> {
        artist_tracks()
            .iter()
            .filter(|t| paths.contains(&t.id))
            .map(|t| t.id.key().into_owned())
            .collect()
    };

    rsx! {
        div {
            class: page_container_class,

            if name.is_empty() {
                div { class: "flex-1 min-h-0 flex flex-col",
                    if !cfg!(target_os = "android") {
                        h1 { class: "text-3xl font-bold text-white mb-6 shrink-0", "{i18n::t(\"artists\")}" }
                    }
                    div { class: "flex items-center justify-end gap-2 mb-4 shrink-0",
                        ViewModeToggle { mode: artists_view_mode }
                        SortControl {
                            criteria: artist_sort,
                            available: vec![
                                ArtistSortField::Name,
                                ArtistSortField::Tracks,
                                ArtistSortField::Albums,
                            ],
                        }
                    }
                    div {
                        id: "artist-grid-scroll",
                        class: "flex-1 min-h-0 overflow-y-auto pb-20",
                        onscroll: move |e| crate::scroll_persist::save("artists", e.scroll_top()),
                        div {
                            // Same trick as the album grids: cards are static, only this
                            // class flips, `.view-list` CSS restyles the `.vcard*` hooks.
                            class: if *artists_view_mode.read() == AlbumViewMode::List { "view-list" } else { "grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-8" },
                            for (artist , cover_url) in artists() {
                                {
                                    let art = artist.clone();
                                    rsx! {
                                        div {
                                            key: "{artist}",
                                            class: "vcard group cursor-pointer flex flex-col items-center",
                                            style: "content-visibility: auto;",
                                            onclick: move |_| artist_name.set(art.clone()),
                                            div {
                                                class: "vcard-avatar aspect-square w-full rounded-full bg-stone-800 mb-4 overflow-hidden relative",
                                                style: "-webkit-user-drag: none;",
                                                ondragstart: move |evt| evt.prevent_default(),
                                                if let Some(url) = cover_url {
                                                    img {
                                                        src: "{url}",
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
                                            h3 { class: "vcard-meta text-white font-medium truncate text-center w-full group-hover:text-indigo-400 transition-colors", "{artist}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "relative flex-1 min-h-0 flex flex-col w-full max-w-[1600px] mx-auto",
                    if !cfg!(target_os = "android") {
                        components::back_button::BackButton {
                            on_click: move |_| nav_ctrl.go_back(),
                        }
                    }
                    div { class: "relative flex-1 min-h-0 flex flex-col",

                        if *show_playlist_modal.read() {
                            PlaylistModal {
                                overlay_class: Some("absolute inset-0 bg-black/80 flex items-center justify-center z-50".to_string()),
                                on_close: move |_| {
                                    show_playlist_modal.set(false);
                                    is_selection_mode.set(false);
                                    selected_tracks.write().clear();
                                },
                                on_add_to_playlist: move |playlist_id: String| {
                                    let paths: HashSet<reader::TrackId> = if is_selection_mode() {
                                        selected_tracks.read().clone()
                                    } else {
                                        selected_track_for_playlist.read().iter().cloned().collect()
                                    };
                                    let refs = refs_for(&paths);
                                    if !refs.is_empty() {
                                        let s = active_source.peek().clone();
                                        spawn(async move {
                                            if s.add_to_playlist(&playlist_id, &refs).await.is_ok() {
                                                gens.bump(Table::Playlists);
                                            }
                                        });
                                    }
                                    show_playlist_modal.set(false);
                                    active_menu_track.set(None);
                                    is_selection_mode.set(false);
                                    selected_tracks.write().clear();
                                },
                                on_create_playlist: move |name: String| {
                                    let paths: HashSet<reader::TrackId> = if is_selection_mode() {
                                        selected_tracks.read().clone()
                                    } else {
                                        selected_track_for_playlist.read().iter().cloned().collect()
                                    };
                                    let refs = refs_for(&paths);
                                    if !refs.is_empty() {
                                        let s = active_source.peek().clone();
                                        spawn(async move {
                                            if s.create_playlist(&name, &refs).await.is_ok() {
                                                gens.bump(Table::Playlists);
                                            }
                                        });
                                    }
                                    show_playlist_modal.set(false);
                                    active_menu_track.set(None);
                                    is_selection_mode.set(false);
                                    selected_tracks.write().clear();
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
                                            let s = active_source.peek().clone();
                                            spawn(async move {
                                                if s.upsert_tracks(&[t]).await.is_ok() {
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
                                show_delete: caps().delete_from_disk,
                                class: Some("absolute bottom-24 left-1/2 -translate-x-1/2 bg-indigo-500 text-white px-6 py-2.5 rounded-full shadow-2xl flex items-center gap-4 z-50 animate-in fade-in zoom-in duration-200 font-mono".to_string()),
                                on_add_to_queue: move |_| {
                                    let selected = selected_tracks.read().clone();
                                    let tracks: Vec<_> = artist_tracks()
                                        .iter()
                                        .filter(|t| selected.contains(&t.id))
                                        .cloned()
                                        .collect();
                                    if !tracks.is_empty() {
                                        ctrl.add_to_queue(tracks);
                                    }
                                    is_selection_mode.set(false);
                                    selected_tracks.write().clear();
                                },
                                on_add_to_playlist: move |_| show_playlist_modal.set(true),
                                on_delete: move |_| {
                                    if caps().delete_from_disk {
                                        let paths: Vec<_> = selected_tracks.read().iter().cloned().collect();
                                        let mut keys = Vec::new();
                                        for id in &paths {
                                            let Some(path) = id.local_path() else {
                                                continue;
                                            };
                                            if std::fs::remove_file(path).is_ok() {
                                                keys.push(id.key().into_owned());
                                            }
                                        }
                                        if !keys.is_empty() {
                                            let s = active_source.peek().clone();
                                            spawn(async move {
                                                if s.delete_tracks(&keys).await.is_ok() {
                                                    gens.bump(Table::Tracks);
                                                }
                                            });
                                        }
                                    }
                                    is_selection_mode.set(false);
                                    selected_tracks.write().clear();
                                },
                                on_cancel: move |_| {
                                    is_selection_mode.set(false);
                                    selected_tracks.write().clear();
                                },
                            }
                        }

                        if *sort_order.read() == ArtistViewOrder::Albums {
                            if *show_album_playlist_modal.read() {
                                PlaylistModal {
                                    overlay_class: Some("absolute inset-0 bg-black/80 flex items-center justify-center z-50".to_string()),
                                    on_close: move |_| show_album_playlist_modal.set(false),
                                    on_add_to_playlist: move |playlist_id: String| {
                                        if let Some(album_id) = pending_album_id_for_playlist.read().clone() {
                                            let s = active_source.peek().clone();
                                            spawn(async move {
                                                let refs: Vec<String> = s
                                                    .album_tracks(&album_id)
                                                    .await
                                                    .unwrap_or_default()
                                                    .iter()
                                                    .filter_map(|t| {
                                                        let k = t.id.key();
                                                        (!k.is_empty()).then(|| k.into_owned())
                                                    })
                                                    .collect();
                                                if !refs.is_empty()
                                                    && s.add_to_playlist(&playlist_id, &refs).await.is_ok()
                                                {
                                                    gens.bump(Table::Playlists);
                                                }
                                            });
                                        }
                                        show_album_playlist_modal.set(false);
                                        pending_album_id_for_playlist.set(None);
                                    },
                                    on_create_playlist: move |playlist_name: String| {
                                        let album_id = pending_album_id_for_playlist.read().clone();
                                        let s = active_source.peek().clone();
                                        spawn(async move {
                                            let refs: Vec<String> = match album_id {
                                                Some(id) => s
                                                    .album_tracks(&id)
                                                    .await
                                                    .unwrap_or_default()
                                                    .iter()
                                                    .filter_map(|t| {
                                                        let k = t.id.key();
                                                        (!k.is_empty()).then(|| k.into_owned())
                                                    })
                                                    .collect(),
                                                None => Vec::new(),
                                            };
                                            if !refs.is_empty()
                                                && s.create_playlist(&playlist_name, &refs).await.is_ok()
                                            {
                                                gens.bump(Table::Playlists);
                                            }
                                        });
                                        show_album_playlist_modal.set(false);
                                        pending_album_id_for_playlist.set(None);
                                    },
                                }
                            }

                            div { class: "flex items-center justify-between mb-4",
                                SortOrderToggle { sort_order }
                                div { class: "flex items-center gap-2",
                                    ViewModeToggle { mode: album_view_mode }
                                    SortControl { criteria: album_sort, available: album_sort_fields() }
                                }
                            }

                            if artist_albums().is_empty() {
                                p { class: "text-slate-500", "{i18n::t(\"no_albums_found\")}" }
                            } else {
                                div {
                                    class: if *album_view_mode.read() == AlbumViewMode::List { "view-list" } else { "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-6" },
                                    for album in artist_albums() {
                                        {
                                            let cap = caps();
                                            let id_for_menu = album.id.clone();
                                            let id_for_navigate = album.id.clone();
                                            let is_open = open_album_menu.read().as_deref() == Some(&album.id);
                                            // Same size in both modes so toggling never refetches covers.
                                            let cover_url = ::server::cover::from_path(&config.read(), album.cover_path.as_deref(), 320);
                                            // Whether every track of this album is downloaded (servers only).
                                            let downloaded = cap.downloads && {
                                                let all = artist_tracks_res.read().clone().unwrap_or_default();
                                                let conf = config.read();
                                                let aid = album.id.clone();
                                                let tracks: Vec<_> = all.iter().filter(|t| t.album_id == aid).collect();
                                                !tracks.is_empty() && tracks.iter().all(|t| {
                                                    let tid = t.id.key();
                                                    conf.offline_tracks.get(tid.as_ref())
                                                        .map(|p| std::path::Path::new(p).exists())
                                                        .unwrap_or(false)
                                                })
                                            };
                                            // Build the menu from capabilities — entries are tagged so
                                            // dispatch survives the gating.
                                            let mut entries: Vec<(MenuAction, AlbumAction)> = vec![
                                                (MenuAction::new(i18n::t("add_all_to_queue").as_str(), "fa-solid fa-list-ul"), AlbumAction::Queue),
                                            ];
                                            if cap.playlists != ::server::source::PlaylistOps::None {
                                                entries.push((MenuAction::new(i18n::t("add_all_to_playlist").as_str(), "fa-solid fa-plus"), AlbumAction::Playlist));
                                            }
                                            if cap.delete_from_disk {
                                                entries.push((MenuAction::new(i18n::t("delete_album").as_str(), "fa-solid fa-trash").destructive(), AlbumAction::DeleteAlbum));
                                            }
                                            if cap.downloads {
                                                let label = if downloaded { "Remove downloads" } else { "Download Album" };
                                                let icon = if downloaded { "fa-solid fa-trash" } else { "fa-solid fa-download" };
                                                entries.push((MenuAction::new(label, icon), AlbumAction::Download { downloaded }));
                                            }
                                            let menu_actions: Vec<MenuAction> = entries.iter().map(|(m, _)| m.clone()).collect();
                                            let action_tags: Vec<AlbumAction> = entries.iter().map(|(_, a)| *a).collect();
                                            rsx! {
                                                div {
                                                    key: "{album.id}",
                                                    class: "vcard group relative p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors",
                                                    style: "content-visibility: auto;",
                                                    onclick: move |_| on_navigate.call(id_for_navigate.clone()),
                                                    oncontextmenu: {
                                                        let id = id_for_menu.clone();
                                                        move |evt| {
                                                            evt.prevent_default();
                                                            open_album_menu.set(Some(id.clone()));
                                                        }
                                                    },
                                                    div {
                                                        class: "vcard-click cursor-pointer",
                                                        div {
                                                            class: "vcard-cover aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                                            style: "-webkit-user-drag: none;",
                                                            ondragstart: move |evt| evt.prevent_default(),
                                                            if let Some(url) = &cover_url {
                                                                img {
                                                                    src: "{url}",
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
                                                        div {
                                                            class: "vcard-meta",
                                                            h3 { class: "text-white font-medium truncate", "{album.title}" }
                                                            p { class: "text-sm text-stone-400 truncate", "{album.artist}" }
                                                        }
                                                    }

                                                    div { class: "vcard-menu absolute bottom-3 right-3",
                                                        DotsMenu {
                                                            actions: menu_actions,
                                                            is_open,
                                                            on_open: {
                                                                let id = id_for_menu.clone();
                                                                move |_| open_album_menu.set(Some(id.clone()))
                                                            },
                                                            on_close: move |_| open_album_menu.set(None),
                                                            button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100 bg-black/40".to_string(),
                                                            anchor: "right".to_string(),
                                                            on_action: {
                                                                let id = id_for_menu.clone();
                                                                let tags = action_tags.clone();
                                                                move |idx: usize| {
                                                                    open_album_menu.set(None);
                                                                    let Some(tag) = tags.get(idx).copied() else { return };
                                                                    match tag {
                                                                        AlbumAction::Queue => {
                                                                            let album_src = active_source.peek().clone();
                                                                            let album_id = id.clone();
                                                                            spawn(async move {
                                                                                let mut tracks = album_src.album_tracks(&album_id).await.unwrap_or_default();
                                                                                tracks.sort_by(|a, b| {
                                                                                    a.track_number.cmp(&b.track_number)
                                                                                        .then_with(|| a.title.cmp(&b.title))
                                                                                });
                                                                                let mut ctrl = ctrl;
                                                                                ctrl.add_to_queue(tracks);
                                                                            });
                                                                        }
                                                                        AlbumAction::Playlist => {
                                                                            pending_album_id_for_playlist.set(Some(id.clone()));
                                                                            show_album_playlist_modal.set(true);
                                                                        }
                                                                        AlbumAction::DeleteAlbum => {
                                                                            let s = active_source.peek().clone();
                                                                            let album_id = id.clone();
                                                                            spawn(async move {
                                                                                let to_delete = s.album_tracks(&album_id).await.unwrap_or_default();
                                                                                for track in &to_delete {
                                                                                    if let Some(path) = track.id.local_path() {
                                                                                        let _ = std::fs::remove_file(path);
                                                                                    }
                                                                                }
                                                                                if s.delete_album(&album_id).await.is_ok() {
                                                                                    gens.bump(Table::Tracks);
                                                                                    gens.bump(Table::Albums);
                                                                                }
                                                                            });
                                                                        }
                                                                        AlbumAction::Download { downloaded } => {
                                                                            let album_src = active_source.peek().clone();
                                                                            let album_id = id.clone();
                                                                            spawn(async move {
                                                                                let tracks = album_src.album_tracks(&album_id).await.unwrap_or_default();
                                                                                if downloaded {
                                                                                    let ids: Vec<String> = tracks.iter().filter_map(|t| {
                                                                                        let k = t.id.key();
                                                                                        (!k.is_empty()).then(|| k.into_owned())
                                                                                    }).collect();
                                                                                    delete_downloads(ids, config, download_queue);
                                                                                } else {
                                                                                    let requests: Vec<(String, String, String)> = tracks.iter().filter_map(|t| {
                                                                                        let k = t.id.key();
                                                                                        (!k.is_empty()).then(|| (k.into_owned(), t.title.clone(), t.artist.clone()))
                                                                                    }).collect();
                                                                                    queue_downloads(requests, config, download_queue);
                                                                                }
                                                                            });
                                                                        }
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
                        } else if artist_tracks().is_empty() {
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
                                    #[cfg(not(target_os = "android"))]
                                    {
                                        let artist = artist_name.peek().clone();
                                        if artist.is_empty() {
                                            return;
                                        }
                                        let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                                        spawn(async move {
                                            let file = rfd::AsyncFileDialog::new()
                                                .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                                                .pick_file()
                                                .await;
                                            if let Some(file) = file {
                                                let path = file.path().to_path_buf();
                                                let key = normalize_artist_key(&artist);
                                                if local
                                                    .set_artist_image(&key, "custom", Some(&path.to_string_lossy()))
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
                                all_selected: !artist_tracks().is_empty() && artist_tracks().iter().all(|track| selected_tracks.read().contains(&track.id)),
                                on_select_all: move |selected: bool| {
                                    if selected {
                                        selected_tracks.set(artist_tracks().into_iter().map(|track| track.id).collect());
                                        is_selection_mode.set(true);
                                    } else {
                                        selected_tracks.write().clear();
                                        is_selection_mode.set(false);
                                    }
                                },
                                on_long_press: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        is_selection_mode.set(true);
                                        selected_tracks.write().insert(track.id.clone());
                                    }
                                },
                                on_select: move |(idx, selected): (usize, bool)| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        if selected {
                                            is_selection_mode.set(true);
                                            selected_tracks.write().insert(track.id.clone());
                                        } else {
                                            selected_tracks.write().remove(&track.id);
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
                                        let path = track.id.clone();
                                        let already_open = active_menu_track.read().as_ref() == Some(&path);
                                        active_menu_track.set((!already_open).then(|| path.clone()));
                                    }
                                },
                                on_close_menu: move |_| active_menu_track.set(None),
                                on_add_to_playlist: move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        selected_track_for_playlist.set(Some(track.id.clone()));
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
                                on_view_metadata: caps().edit_tags.then(|| EventHandler::new(move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        metadata_track.set(Some(track.clone()));
                                        active_menu_track.set(None);
                                    }
                                })),
                                on_delete_track: EventHandler::new(move |idx: usize| {
                                    if caps().delete_from_disk
                                        && let Some(track) = artist_tracks().get(idx)
                                        && let Some(p) = track.id.local_path()
                                        && std::fs::remove_file(p).is_ok()
                                    {
                                        let s = active_source.peek().clone();
                                        let key = track.id.key().into_owned();
                                        spawn(async move {
                                            if s.delete_tracks(&[key]).await.is_ok() {
                                                gens.bump(Table::Tracks);
                                            }
                                        });
                                    }
                                    active_menu_track.set(None);
                                }),
                                on_download_track: caps().downloads.then(|| EventHandler::new(move |idx: usize| {
                                    if let Some(track) = artist_tracks().get(idx) {
                                        let item_id = track.id.key();
                                        if !item_id.is_empty() {
                                            let item_id = item_id.as_ref();
                                            let is_downloaded = config.read().offline_tracks.get(item_id)
                                                .map(|p| std::path::Path::new(p).exists())
                                                .unwrap_or(false);
                                            if is_downloaded {
                                                delete_downloads(vec![item_id.to_string()], config, download_queue);
                                            } else {
                                                queue_downloads(vec![(item_id.to_string(), track.title.clone(), track.artist.clone())], config, download_queue);
                                            }
                                        }
                                        active_menu_track.set(None);
                                    }
                                })),
                                on_download_all: caps().downloads.then(|| EventHandler::new(move |_: ()| {
                                    let requests: Vec<(String, String, String)> = artist_tracks().iter().filter_map(|t| {
                                        let k = t.id.key();
                                        (!k.is_empty()).then(|| (k.into_owned(), t.title.clone(), t.artist.clone()))
                                    }).collect();
                                    queue_downloads(requests, config, download_queue);
                                })),
                                on_delete_all: caps().downloads.then(|| EventHandler::new(move |_: ()| {
                                    let ids: Vec<String> = artist_tracks().iter().filter_map(|t| {
                                        let k = t.id.key();
                                        (!k.is_empty()).then(|| k.into_owned())
                                    }).collect();
                                    delete_downloads(ids, config, download_queue);
                                })),
                                is_downloading_all: download_queue.read().is_active(),
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
