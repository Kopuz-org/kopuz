//! Source-agnostic Album page (issue #35). One grid + one detail render any
//! source: every cover is a reference the daemon resolves, and the divergent
//! affordances (tag/cover edit + delete-from-disk for local, downloads for a
//! server) gate on [`api::SourceCapabilities`] — no `is_server()`.

use components::dots_menu::{DotsMenu, MenuAction};
use components::playlist_modal::PlaylistModal;
use components::sort_control::SortControl;
use components::track_list_view::TrackListView;
use components::view_mode_toggle::ViewModeToggle;
use config::{AlbumViewMode, AppConfig};
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{
    use_active_source, use_album, use_album_tracks, use_albums, use_tracks_by_keys,
};
use std::collections::HashSet;

/// Copy a link to the clipboard and flash a small toast. Used by the YT album
/// page's share button (the `track_row` clipboard helper is crate-private to
/// `components`, so the page carries its own tiny copy).
fn copy_album_link(url: String) {
    let value = serde_json::to_string(&url).unwrap_or_else(|_| "\"\"".to_string());
    let js = format!(
        "navigator.clipboard.writeText({value}).then(() => {{\
            let t = document.getElementById('kopuz-toast');\
            if (!t) {{ t = document.createElement('div'); t.id = 'kopuz-toast';\
                t.style.cssText = 'position:fixed;left:50%;bottom:88px;transform:translateX(-50%);background:rgba(20,20,20,0.95);color:#fff;padding:10px 18px;border-radius:8px;font:14px system-ui,sans-serif;z-index:99999;box-shadow:0 4px 16px rgba(0,0,0,0.4);pointer-events:none;border:1px solid rgba(255,255,255,0.1);';\
                document.body.appendChild(t); }}\
            t.textContent = 'Copied link'; t.style.opacity = '1';\
            clearTimeout(t._h); t._h = setTimeout(() => {{ t.style.opacity = '0'; }}, 1800);\
        }}).catch((e) => console.error('clipboard writeText failed', e));"
    );
    let _ = dioxus::document::eval(&js);
}

/// One album-card menu entry, tagged so dispatch survives capability gating.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AlbumAction {
    Queue,
    Playlist,
    /// Local: delete the files + DB rows. Server: drop the cached rows (a re-sync
    /// re-adds them) — there's no remote album delete.
    Remove,
}

#[component]
pub fn Album(
    config: Signal<AppConfig>,
    album_id: Signal<String>,
    mut queue: Signal<Vec<api::TrackInfo>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let source = use_active_source();
    let caps = hooks::sources::use_capabilities();
    let nav_ctrl = use_context::<components::NavigationController>();

    let open_album_menu = use_signal(|| None::<String>);
    let mut show_album_playlist_modal = use_signal(|| false);
    let pending_album_id_for_playlist = use_signal(|| None::<String>);

    let albums_res = use_albums(source);

    // First visit to a server with an empty cache → pull once.
    let mut has_fetched = use_signal(|| false);
    use_effect(move || {
        if !caps().sync || *has_fetched.read() {
            return;
        }
        if let Some(albums) = albums_res.read().clone() {
            has_fetched.set(true);
            if albums.is_empty() {
                hooks::jobs::start(hooks::JobKind::LibrarySync);
            }
        }
    });

    let pending_album_id = use_memo(move || {
        pending_album_id_for_playlist
            .read()
            .clone()
            .unwrap_or_default()
    });
    let pending_tracks_res = use_album_tracks(source, pending_album_id);

    rsx! {
        div {
            class: if cfg!(target_os = "android") { "px-4 pt-2 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },

            if album_id.read().is_empty() {
                div { class: "flex-1 min-h-0 flex flex-col",
                    if !cfg!(target_os = "android") {
                        h1 { class: "text-3xl font-semibold tracking-tight text-white mb-6 shrink-0", "{i18n::t(\"all_albums\")}" }
                    }

                    AlbumGrid {
                        config,
                        album_id,
                        open_album_menu,
                        show_album_playlist_modal,
                        pending_album_id_for_playlist,
                    }

                    if *show_album_playlist_modal.read() {
                        PlaylistModal {
                            on_close: move |_| show_album_playlist_modal.set(false),
                            on_add_to_playlist: move |playlist_id: String| {
                                if pending_album_id_for_playlist.read().is_some() {
                                    let refs: Vec<String> = pending_tracks_res
                                        .read()
                                        .clone()
                                        .unwrap_or_default()
                                        .iter()
                                        .map(|t| t.key.clone())
                                        .collect();
                                    hooks::playlist_actions::add_tracks(playlist_id, refs);
                                }
                                show_album_playlist_modal.set(false);
                            },
                            on_create_playlist: move |name: String| {
                                if pending_album_id_for_playlist.read().is_some() {
                                    let refs: Vec<String> = pending_tracks_res
                                        .read()
                                        .clone()
                                        .unwrap_or_default()
                                        .iter()
                                        .map(|t| t.key.clone())
                                        .collect();
                                    hooks::playlist_actions::create_with(name, refs);
                                }
                                show_album_playlist_modal.set(false);
                            },
                        }
                    }
                }
            } else {
                AlbumDetail {
                    config,
                    album_id_str: album_id.read().clone(),
                    queue,
                    current_queue_index,
                    on_close: move |_| nav_ctrl.go_back(),
                }
            }
        }
    }
}

#[component]
fn AlbumGrid(
    mut config: Signal<AppConfig>,
    mut album_id: Signal<String>,
    mut open_album_menu: Signal<Option<String>>,
    mut show_album_playlist_modal: Signal<bool>,
    mut pending_album_id_for_playlist: Signal<Option<String>>,
) -> Element {
    let source = use_active_source();
    let caps = hooks::sources::use_capabilities();
    let is_offline = use_context::<Signal<bool>>();
    let ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let albums_res = use_albums(source);

    let album_sort = use_signal(|| config.peek().album_sort.clone());
    use_effect(move || {
        let curr = album_sort.read().clone();
        if config.peek().album_sort != curr {
            config.write().album_sort = curr;
        }
    });
    let view_mode = use_signal(|| config.peek().album_view_mode);
    use_effect(move || {
        let curr = *view_mode.read();
        if config.peek().album_view_mode != curr {
            config.write().album_view_mode = curr;
        }
    });
    let available_sort_fields = use_memo(move || {
        hooks::sort::available_album_fields(&albums_res.read().clone().unwrap_or_default())
    });

    // Offline (server): only albums with downloaded tracks. Album ids come from
    // the downloaded tracks themselves. The grid dedupes by title — the detail
    // re-aggregates same-titled albums.
    let offline_keys = use_memo(move || -> Vec<String> {
        if !(caps().downloads && *is_offline.read()) {
            return Vec::new();
        }
        config
            .read()
            .offline_tracks
            .iter()
            .filter(|(_, p)| std::path::Path::new(p).exists())
            .map(|(id, _)| id.clone())
            .collect()
    });
    let offline_tracks_res = use_tracks_by_keys(source, offline_keys);
    let downloaded_album_ids = use_memo(move || -> HashSet<String> {
        if !(caps().downloads && *is_offline.read()) {
            return HashSet::new();
        }
        offline_tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|t| t.album_id.clone())
            .collect()
    });

    let albums = use_memo(move || {
        let offline = caps().downloads && *is_offline.read();
        let downloaded = downloaded_album_ids();
        let mut seen = HashSet::new();
        let mut albums = albums_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| !offline || downloaded.contains(&a.id))
            .filter(|a| seen.insert(a.title.trim().to_lowercase()))
            .collect::<Vec<_>>();
        hooks::sort::sort_albums(&mut albums, &album_sort.read());
        albums
    });

    // Restore the grid scroll once after the albums first render; guarded so DB
    // reactivity re-runs don't keep snapping the view back to the saved offset.
    let mut scroll_restored = use_signal(|| false);
    use_effect(move || {
        if *scroll_restored.read() || albums().is_empty() {
            return;
        }
        scroll_restored.set(true);
        let _ = dioxus::document::eval(&crate::scroll_persist::restore_eval(
            "album-grid-scroll",
            "albums",
        ));
    });

    rsx! {
        div { class: "flex-1 min-h-0 flex flex-col",
        div { class: "flex items-center justify-end gap-2 mb-4 shrink-0",
            ViewModeToggle { mode: view_mode }
            SortControl { criteria: album_sort, available: available_sort_fields() }
        }
        div {
            id: "album-grid-scroll",
            class: "flex-1 min-h-0 overflow-y-auto pb-8",
            onscroll: move |e| crate::scroll_persist::save("albums", e.scroll_top()),
            if albums().is_empty() {
                p { class: "text-slate-500", "{i18n::t(\"no_albums_found\")}" }
            } else {
                div {
                    // Cards keep identical classes in both modes (`.vcard*` hooks are
                    // restyled by the `.view-list` CSS), so toggling only patches this
                    // container's class — no per-card re-render or cover refetch.
                    class: if *view_mode.read() == AlbumViewMode::List { "view-list" } else { "grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-6" },
                    for album in albums() {
                        {
                            let cap = caps();
                            let id_for_nav = album.id.clone();
                            let id_for_menu = album.id.clone();
                            let is_open = open_album_menu.read().as_deref() == Some(&album.id);
                            let cover_url = hooks::artwork::for_album(&album, hooks::artwork::Size::Thumb);
                            let remove_label = if cap.delete_from_disk {
                                i18n::t("delete_album").to_string()
                            } else {
                                i18n::t("remove_from_cache").to_string()
                            };
                            let actions = vec![
                                MenuAction::new(i18n::t("add_all_to_queue").as_str(), "fa-solid fa-list-ul"),
                                MenuAction::new(i18n::t("add_all_to_playlist").as_str(), "fa-solid fa-plus"),
                                MenuAction::new(remove_label.as_str(), "fa-solid fa-trash").destructive(),
                            ];
                            let tags = [AlbumAction::Queue, AlbumAction::Playlist, AlbumAction::Remove];
                            rsx! {
                                div {
                                    key: "{album.id}",
                                    class: if is_open { "vcard group relative z-50 p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors" } else { "vcard group relative p-4 bg-white/5 rounded-xl hover:bg-white/10 transition-colors" },
                                    style: if is_open { "content-visibility: visible; contain: none;" } else { "content-visibility: auto;" },
                                    oncontextmenu: {
                                        let id = id_for_menu.clone();
                                        move |evt| {
                                            evt.prevent_default();
                                            open_album_menu.set(Some(id.clone()));
                                        }
                                    },

                                    div {
                                        class: "vcard-click cursor-pointer",
                                        onclick: move |_| album_id.set(id_for_nav.clone()),
                                        div {
                                            class: "vcard-cover aspect-square rounded-lg bg-stone-800 mb-3 overflow-hidden relative",
                                            style: "-webkit-user-drag: none;",
                                            ondragstart: move |evt| evt.prevent_default(),
                                            if let Some(url) = &cover_url {
                                                img { src: "{url}", class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-300", decoding: "async", loading: "lazy", draggable: "false", ondragstart: move |evt| evt.prevent_default() }
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
                                            actions,
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
                                                let title = album.title.clone();
                                                move |idx: usize| {
                                                    open_album_menu.set(None);
                                                    let Some(tag) = tags.get(idx).copied() else { return };
                                                    match tag {
                                                        AlbumAction::Queue => {
                                                            hooks::library_actions::with_album_keys(id.clone(), move |keys| {
                                                                let mut ctrl = ctrl;
                                                                ctrl.set_queue_keys(keys, api::QueueMode::Append, None);
                                                            });
                                                        }
                                                        AlbumAction::Playlist => {
                                                            pending_album_id_for_playlist.set(Some(id.clone()));
                                                            show_album_playlist_modal.set(true);
                                                        }
                                                        AlbumAction::Remove => {
                                                            if cap.delete_from_disk {
                                                                hooks::library_actions::delete_album(id.clone(), true);
                                                            } else {
                                                                // A server splits one release across
                                                                // same-titled albums, so dropping the
                                                                // cache means dropping all of them.
                                                                let all = albums_res.read().clone().unwrap_or_default();
                                                                for album in all.iter().filter(|album| album.title == title) {
                                                                    hooks::library_actions::delete_album(album.id.clone(), false);
                                                                }
                                                            }
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
        }
        }
    }
}

#[component]
fn AlbumDetail(
    config: Signal<AppConfig>,
    album_id_str: String,
    mut queue: Signal<Vec<api::TrackInfo>>,
    current_queue_index: Signal<usize>,
    on_close: EventHandler<()>,
) -> Element {
    let nav_ctrl = use_context::<components::NavigationController>();
    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let api = hooks::use_api();
    let caps = hooks::sources::use_capabilities();
    let is_offline = use_context::<Signal<bool>>();
    let downloads = hooks::downloads::use_downloads();

    let album_id_memo = use_memo(use_reactive!(|album_id_str| album_id_str));
    let album_res = use_album(source, album_id_memo);
    let albums_res = use_albums(source);

    // Discover albums are opened by their YT browse id (MPRE…) and aren't in the
    // local DB until saved. When the DB has no row for the id, fetch the album
    // straight from the catalog remote by that browse id so every searched /
    // discovered album renders (header + full track list) instead of "not found".
    let direct_remote_res: Resource<Option<api::CatalogDetail>> = {
        let api = api.clone();
        use_resource(move || {
            let want = !*is_offline.read();
            let db_has = album_res.read().clone().flatten().is_some();
            let id = album_id_memo();
            let api = api.clone();
            async move {
                if !want || db_has || id.trim().is_empty() {
                    return None;
                }
                api.catalog_detail(api::CatalogDetailRequest {
                    kind: api::CatalogItemKind::Album,
                    id,
                    continuation: None,
                })
                .await
                .ok()
            }
        })
    };

    let album_loading = album_res.read().is_none();
    let album = match album_res.read().clone().flatten() {
        Some(a) => a,
        None => {
            // Not saved locally — render the remote album directly if it resolved.
            if let Some(remote) = direct_remote_res.read().clone().flatten() {
                let mut tracks = remote.tracks;
                tracks.sort_by(|a, b| {
                    a.disc_number
                        .unwrap_or(1)
                        .cmp(&b.disc_number.unwrap_or(1))
                        .then_with(|| {
                            a.track_number
                                .unwrap_or(0)
                                .cmp(&b.track_number.unwrap_or(0))
                        })
                });
                return rsx! {
                    div { class: "absolute inset-0 flex flex-col overflow-hidden p-8",
                        YtAlbumDetail {
                            config,
                            title: remote.title,
                            artist: remote.subtitle.unwrap_or_default(),
                            year: remote.year,
                            browse_id: Some(remote.id),
                            local_cover: hooks::artwork::url(remote.artwork.as_ref(), hooks::artwork::Size::Thumb),
                            tracks,
                            on_close,
                        }
                    }
                };
            }
            // Still resolving (DB miss not yet confirmed, or remote in flight).
            if album_loading || direct_remote_res.read().is_none() {
                return rsx! { div {} };
            }
            return rsx! { div { "{i18n::t(\"album_not_found\")}" } };
        }
    };

    // The grid dedupes albums by title, so the detail aggregates every
    // same-titled album's tracks.
    let info_title = album.title.clone();
    let matching_ids = use_memo(move || -> Vec<String> {
        let title = info_title.clone();
        let ids: Vec<String> = albums_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.title == title)
            .map(|a| a.id)
            .collect();
        ids
    });
    let tracks_res = {
        let api = api.clone();
        use_resource(move || {
            let _ = gens.generation(Table::Tracks);
            let ids = matching_ids();
            let api = api.clone();
            async move {
                let mut out = Vec::new();
                for id in ids {
                    let page = api
                        .album_tracks(
                            id,
                            api::Page {
                                offset: 0,
                                limit: u32::MAX,
                            },
                        )
                        .await
                        .unwrap_or_default();
                    out.extend(page.items);
                }
                out
            }
        })
    };

    // Catalog remotes (YT) store albums under a title+artist hash with no
    // browse id, so the library only ever holds the few tracks the user saved —
    // an album page would show 1 of 18 songs. The daemon resolves the saved
    // album to its remote listing (header + every track), the way YT Music
    // shows it. `None` for local/other sources and while offline; drives both
    // the full track list and the YT-styled header.
    let remote_album_res: Resource<Option<api::CatalogDetail>> = {
        let api = api.clone();
        use_resource(move || {
            let want = caps().albums == api::AlbumPresentation::Remote && !*is_offline.read();
            let album = album_res.read().clone().flatten();
            let api = api.clone();
            async move {
                let album = album?;
                if !want || album.title.trim().is_empty() {
                    return None;
                }
                api.catalog_detail(api::CatalogDetailRequest {
                    kind: api::CatalogItemKind::Album,
                    id: album.id,
                    continuation: None,
                })
                .await
                .ok()
            }
        })
    };

    let tracks = use_memo(move || {
        let offline = caps().downloads && *is_offline.read();
        let conf = config.read();

        // Full album from the catalog remote (already in album order). Used
        // whenever it resolved; the locally-saved subset is the fallback.
        if !offline && let Some(remote) = remote_album_res.read().clone().flatten() {
            let mut remote = remote.tracks;
            remote.sort_by(|a, b| {
                a.disc_number
                    .unwrap_or(1)
                    .cmp(&b.disc_number.unwrap_or(1))
                    .then_with(|| {
                        a.track_number
                            .unwrap_or(0)
                            .cmp(&b.track_number.unwrap_or(0))
                    })
            });
            return remote;
        }

        let mut tracks: Vec<api::TrackInfo> = tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| !offline || conf.offline_tracks.contains_key(&t.key))
            .collect();
        tracks.sort_by(|a, b| {
            a.disc_number
                .unwrap_or(1)
                .cmp(&b.disc_number.unwrap_or(1))
                .then_with(|| {
                    a.track_number
                        .unwrap_or(0)
                        .cmp(&b.track_number.unwrap_or(0))
                })
        });
        tracks
    });

    let album_title = album.title.clone();
    let album_artist = album.artist.clone();
    let album_artist_for_nav = album_artist.clone();
    let cover_url = hooks::artwork::for_album(&album, hooks::artwork::Size::Thumb);
    let cap = caps();
    let aid = album.id.clone();

    // The daemon removes the stored picture and forgets the file it saved,
    // so this only has to say which album.
    let cover_reset_action = if cap.edit_tags && album.artwork.is_some() {
        let aid = aid.clone();
        Some(rsx! {
            button {
                class: "inline-flex items-center justify-center h-9 w-9 rounded-full text-sm font-medium transition-colors border border-white/12 hover:bg-white/10",
                style: "color: var(--color-white); opacity: 0.6;",
                aria_label: i18n::t("remove_cover").to_string(),
                title: i18n::t("remove_cover").to_string(),
                onclick: move |_| {
                    hooks::library_actions::remove_artwork(api::ArtworkTarget::Album(aid.clone()));
                },
                i { class: "fa-solid fa-trash text-xs" }
            }
        })
    } else {
        None
    };

    let aid_cover = aid.clone();
    let tracks_delete = tracks();
    let tracks_download = tracks();
    let tracks_download_all = tracks();
    let tracks_delete_all = tracks();

    let is_downloading_all = cap.downloads
        && tracks()
            .iter()
            .any(|track| downloads.read().is_active(&track.key));

    // YT-Music-style album page: the whole catalog-remote (YT) side renders this,
    // from the moment the page opens — header built from the local album row so it
    // shows instantly, track list filling from the locally-saved subset until the
    // remote album resolves the full listing. Local / other sources keep the
    // standard TrackListView.
    let cover_url_yt = cover_url.clone();
    let yt_title = album.title.clone();
    let yt_artist = album.artist.clone();
    // Prefer the remote album's year once resolved; fall back to the local row.
    let yt_remote = remote_album_res.read().clone().flatten();
    let yt_year = yt_remote
        .as_ref()
        .and_then(|a| a.year.clone())
        .or_else(|| (album.year > 0).then(|| album.year.to_string()));
    let yt_browse_id = yt_remote.as_ref().map(|a| a.id.clone());

    rsx! {
        div { class: "absolute inset-0 flex flex-col overflow-hidden p-8",
            if cap.albums == api::AlbumPresentation::Remote {
                YtAlbumDetail {
                    config,
                    title: yt_title,
                    artist: yt_artist,
                    year: yt_year,
                    browse_id: yt_browse_id,
                    local_cover: cover_url_yt,
                    tracks: tracks(),
                    on_close,
                }
            } else {
            TrackListView {
                name: album_title,
                description: album_artist,
                on_description_click: Some(EventHandler::new(move |_| {
                    nav_ctrl.navigate_to_artist(album_artist_for_nav.clone());
                })),
                cover_url,
                is_album: true,
                release_year: (album.year > 0).then_some(album.year),
                tracks: tracks(),
                on_close,
                enable_metadata: cap.edit_tags,
                show_delete_in_selection: cap.delete_from_disk,
                is_downloading_all,
                // Picking the file is the UI's; storing it is not.
                on_cover_click: cap.edit_tags.then(|| EventHandler::new(move |_| {
                    let aid = aid_cover.clone();
                    let _ = &aid;
                    #[cfg(not(target_os = "android"))]
                    spawn(async move {
                        let Some(file) = rfd::AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await
                        else {
                            return;
                        };
                        let path = file.path().to_path_buf();
                        let Ok(bytes) = tokio::fs::read(&path).await else {
                            return;
                        };
                        hooks::library_actions::upload_artwork(
                            api::ArtworkTarget::Album(aid),
                            hooks::library_actions::content_type_for(&path),
                            bytes,
                        );
                    });
                })),
                actions: cover_reset_action,
                on_delete_track: cap.delete_from_disk.then(|| EventHandler::new(move |idx: usize| {
                    if let Some(track) = tracks_delete.get(idx) {
                        hooks::library_actions::delete_tracks(
                            vec![track.key.clone()],
                            true,
                        );
                    }
                })),
                on_selection_delete: cap.delete_from_disk.then(|| EventHandler::new(move |keys: Vec<String>| {
                    hooks::library_actions::delete_tracks(keys, true);
                })),
                on_download_track: cap.downloads.then(|| EventHandler::new(move |idx: usize| {
                    if let Some(t) = tracks_download.get(idx) {
                        let key = &t.key;
                        if key.is_empty() {
                            return;
                        }

                        let downloaded = config.read().offline_tracks.get(key)
                            .map(|p| std::path::Path::new(p).exists())
                            .unwrap_or(false);
                        if downloaded {
                            hooks::downloads::remove(vec![key.to_string()]);
                        } else {
                            hooks::downloads::start(vec![key.to_string()]);
                        }
                    }
                })),
                on_download_all: cap.downloads.then(|| EventHandler::new(move |_: ()| {
                    let requests: Vec<String> = tracks_download_all.iter().filter_map(|t| {
                        let k = t.key.clone();
                        (!k.is_empty()).then_some(k)
                    }).collect();
                    hooks::downloads::start(requests);
                })),
                on_delete_all: cap.downloads.then(|| EventHandler::new(move |_: ()| {
                    let ids: Vec<String> = tracks_delete_all.iter().filter_map(|t| {
                        let k = t.key.clone();
                        (!k.is_empty()).then_some(k)
                    }).collect();
                    hooks::downloads::remove(ids);
                })),
            }
            }
        }
    }
}

/// YT-Music-style album page: a left meta column (cover, artist link, title,
/// "Album", song count · duration · year, play / shuffle / download) beside the
/// full track list. Shown only for the catalog remote (YT) once the album
/// resolved; local/other sources use [`TrackListView`]. Rows reuse [`TrackRow`]
/// so play / queue / menu / download behave exactly as everywhere else.
#[component]
fn YtAlbumDetail(
    config: Signal<AppConfig>,
    title: String,
    artist: String,
    year: Option<String>,
    browse_id: Option<String>,
    local_cover: Option<utils::CoverUrl>,
    tracks: Vec<api::TrackInfo>,
    on_close: EventHandler<()>,
) -> Element {
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let nav_ctrl = use_context::<components::NavigationController>();
    let downloads = hooks::downloads::use_downloads();

    let mut active_menu = use_signal(|| None::<String>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut playlist_track = use_signal(|| None::<String>);

    let total: u64 = tracks.iter().filter_map(|t| t.duration_secs()).sum();
    let dur_min = total / 60;
    let song_count = tracks.len();
    let artist_name = artist;
    let artist_for_nav = artist_name.clone();

    // Current track for the row highlight. Read `current_queue_index`
    // *reactively* (`current_track()` peeks, so the page wouldn't re-render on a
    // skip) so the highlighted row follows next/prev.
    let current_id = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|t| t.uid)
    };
    let offline_tracks = config.read().offline_tracks.clone();

    // Whether every album track is downloaded for offline — drives the download
    // button's toggle (download all ⇄ remove all).
    let all_downloaded = !tracks.is_empty()
        && tracks.iter().all(|t| {
            offline_tracks
                .get(&t.key)
                .map(|p| std::path::Path::new(p).exists())
                .unwrap_or(false)
        });

    let tracks_play_all = tracks.clone();
    let tracks_download_all = tracks.clone();
    let artist_for_nav_btn = artist_name.clone();
    // Prefer the provider's album page; fall back to its first track page.
    // The daemon knows which sources have web pages and how they spell them;
    // an id and a key are all that leave here.
    let share_api = hooks::use_api();
    let share_id = browse_id.clone();
    let share_key = tracks.first().map(|track| track.key.clone());
    let share_url = use_resource(move || {
        let api = share_api.clone();
        let (id, key) = (share_id.clone(), share_key.clone());
        async move {
            if let Some(id) = id
                && let Ok(Some(url)) = api.album_web_url(id).await
            {
                return Some(url);
            }
            match key {
                Some(key) => api.track_web_url(key).await.ok().flatten(),
                None => None,
            }
        }
    });
    let share_url = share_url.read().clone().flatten();

    rsx! {
        div { class: "w-full max-w-[1600px] mx-auto select-none flex-1 min-h-0 flex flex-col",
            if !cfg!(target_os = "android") {
                components::back_button::BackButton {
                    on_click: move |_| on_close.call(()),
                }
            }

            div { class: "flex-1 min-h-0 flex flex-col md:flex-row gap-10 overflow-hidden",

                // Left meta column.
                div { class: "md:w-[320px] shrink-0 flex flex-col items-center md:items-start text-center md:text-left gap-5 md:pt-2",
                    div {
                        class: "w-full max-w-[300px] aspect-square rounded-lg bg-stone-800 overflow-hidden relative shrink-0 shadow-2xl shadow-black/40",
                        if let Some(url) = &local_cover {
                            img { src: "{url.as_ref()}", class: "w-full h-full object-cover", decoding: "async" }
                        } else {
                            div { class: "w-full h-full flex items-center justify-center text-white/20",
                                i { class: "fa-solid fa-compact-disc text-7xl" }
                            }
                        }
                    }
                    div { class: "flex flex-col gap-2 w-full",
                        button {
                            class: "text-sm font-semibold text-white/60 hover:text-white hover:underline transition-colors truncate max-w-full self-center md:self-start",
                            onclick: move |_| nav_ctrl.navigate_to_artist(artist_for_nav.clone()),
                            "{artist_name}"
                        }
                        h1 { class: "text-3xl font-semibold tracking-tight text-white leading-[1.1] break-words", "{title}" }
                        div { class: "text-sm text-slate-400 flex flex-wrap items-center gap-x-2 justify-center md:justify-start",
                            if year.is_some() {
                                span { class: "uppercase tracking-wide text-xs font-semibold text-white/40", "{i18n::t(\"album\")}" }
                                span { class: "text-white/30", "•" }
                            }
                            span { "{i18n::t_with(\"showcase_song_count\", &[(\"count\", song_count.to_string())])}" }
                            span { class: "text-white/30", "•" }
                            span { "{dur_min} {i18n::t(\"min\")}" }
                            if let Some(y) = year {
                                span { class: "text-white/30", "•" }
                                span { "{y}" }
                            }
                        }
                    }
                    div { class: "flex items-center gap-3 mt-1",
                        // Download all / remove downloads.
                        button {
                            class: "w-11 h-11 rounded-full border border-white/15 flex items-center justify-center text-slate-300 hover:text-white hover:border-white/30 transition-colors disabled:opacity-40",
                            title: if all_downloaded { "Remove download".to_string() } else { "Download".to_string() },
                            disabled: downloads.read().running,
                            onclick: move |_| {
                                if all_downloaded {
                                    let ids: Vec<String> = tracks_download_all.iter().filter_map(|t| {
                                        let k = t.key.clone();
                                        (!k.is_empty()).then_some(k)
                                    }).collect();
                                    hooks::downloads::remove(ids);
                                } else {
                                    let reqs: Vec<String> = tracks_download_all.iter().filter_map(|t| {
                                        let k = t.key.clone();
                                        (!k.is_empty()).then_some(k)
                                    }).collect();
                                    hooks::downloads::start(reqs);
                                }
                            },
                            i { class: if all_downloaded { "fa-solid fa-trash" } else { "fa-solid fa-download" } }
                        }
                        // Go to artist.
                        button {
                            class: "w-11 h-11 rounded-full border border-white/15 flex items-center justify-center text-slate-300 hover:text-white hover:border-white/30 transition-colors",
                            title: "Go to artist".to_string(),
                            onclick: move |_| nav_ctrl.navigate_to_artist(artist_for_nav_btn.clone()),
                            i { class: "fa-solid fa-user" }
                        }
                        // Play (primary).
                        button {
                            class: "w-16 h-16 rounded-full bg-indigo-500 hover:bg-indigo-400 text-black flex items-center justify-center transition-transform hover:scale-105 shadow-lg shadow-black/30",
                            title: i18n::t("play").to_string(),
                            onclick: move |_| {
                                if *ctrl.shuffle.peek() {
                                    ctrl.play_queue_shuffled(tracks_play_all.clone());
                                } else {
                                    ctrl.play_queue_linear(tracks_play_all.clone());
                                }
                            },
                            i { class: "fa-solid fa-play text-2xl ml-1" }
                        }
                        // Shuffle.
                        button {
                            class: format!("w-11 h-11 rounded-full border flex items-center justify-center transition-colors {}", if *ctrl.shuffle.read() { "text-indigo-500 bg-white/10 border-white/30" } else { "text-slate-300 border-white/15 hover:text-white hover:border-white/30" }),
                            title: i18n::t("shuffle").to_string(),
                            onclick: move |_| ctrl.toggle_shuffle(),
                            i { class: "fa-solid fa-shuffle" }
                        }
                        // Share.
                        if let Some(url) = share_url {
                            button {
                                class: "w-11 h-11 rounded-full border border-white/15 flex items-center justify-center text-slate-300 hover:text-white hover:border-white/30 transition-colors",
                                title: "Share".to_string(),
                                onclick: move |_| copy_album_link(url.clone()),
                                i { class: "fa-solid fa-arrow-up-from-bracket" }
                            }
                        }
                    }
                }

                // Track list.
                div { class: "flex-1 min-h-0 overflow-y-auto pb-24",
                    for (idx, track) in tracks.iter().cloned().enumerate() {
                        {
                            let cover_url = hooks::artwork::for_track(&track, hooks::artwork::Size::Thumb);
                            let is_menu_open = active_menu.read().as_ref() == Some(&track.uid);
                            let is_current = current_id.as_ref() == Some(&track.uid);
                            let key = track.key.clone();
                            let is_downloaded = offline_tracks
                                .get(&key)
                                .map(|p| std::path::Path::new(p).exists())
                                .unwrap_or(false);
                            let row_tracks = tracks.clone();
                            let menu_id = track.uid.clone();
                            let pl_id = track.uid.clone();
                            let dl_track = track.clone();
                            let q_track = track.clone();
                            rsx! {
                                components::track_row::TrackRow {
                                    key: "{track.uid}",
                                    track: track.clone(),
                                    cover_url,
                                    is_album: true,
                                    hide_delete: true,
                                    row_num: Some(components::showcase::track_row_number(
                                        track.track_number,
                                        idx + 1,
                                        true,
                                    )),
                                    is_menu_open,
                                    is_currently_playing: is_current,
                                    is_downloaded,
                                    on_start_radio: components::track_row::radio_handler(track.key.clone()),
                                    on_play: move |_| {
                                        ctrl.play_queue_at(row_tracks.clone(), idx);
                                    },
                                    on_queue: Some(EventHandler::new(move |_| {
                                        ctrl.add_to_queue(vec![q_track.clone()]);
                                        active_menu.set(None);
                                    })),
                                    on_click_menu: move |_| {
                                        let open = active_menu.read().as_ref() == Some(&menu_id);
                                        active_menu.set((!open).then(|| menu_id.clone()));
                                    },
                                    on_close_menu: move |_| active_menu.set(None),
                                    on_add_to_playlist: move |_| {
                                        playlist_track.set(Some(pl_id.clone()));
                                        show_playlist_modal.set(true);
                                        active_menu.set(None);
                                    },
                                    on_delete: move |_| {},
                                    on_download: Some(EventHandler::new(move |_| {
                                        let k = &dl_track.key;
                                        if k.is_empty() {
                                            return;
                                        }

                                        let downloaded = config.read().offline_tracks.get(k)
                                            .map(|p| std::path::Path::new(p).exists())
                                            .unwrap_or(false);
                                        if downloaded {
                                            hooks::downloads::remove(vec![k.to_string()]);
                                        } else {
                                            hooks::downloads::start(vec![k.to_string()]);
                                        }
                                        active_menu.set(None);
                                    })),
                                }
                            }
                        }
                    }
                }
            }

            if *show_playlist_modal.read() {
                PlaylistModal {
                    on_close: move |_| {
                        show_playlist_modal.set(false);
                        playlist_track.set(None);
                    },
                    on_add_to_playlist: move |playlist_id: String| {
                        if let Some(id) = playlist_track.read().clone() {
                            hooks::playlist_actions::add_tracks(
                                playlist_id,
                                vec![id],
                            );
                        }
                        show_playlist_modal.set(false);
                        playlist_track.set(None);
                    },
                    on_create_playlist: move |name: String| {
                        if let Some(id) = playlist_track.read().clone() {
                            hooks::playlist_actions::create_with(
                                name,
                                vec![id],
                            );
                        }
                        show_playlist_modal.set(false);
                        playlist_track.set(None);
                    },
                }
            }
        }
    }
}
