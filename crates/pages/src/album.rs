//! Source-agnostic Album page (issue #35). One grid + one detail render any
//! source: covers resolve through the source layer, and the divergent
//! affordances (tag/cover edit + delete-from-disk for local, downloads for a
//! server) gate on the resolved source's [`Capabilities`] — no `is_server()`.

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
use std::path::PathBuf;

use crate::server::download_manager::{
    DownloadQueue, DownloadStatus, delete_downloads, queue_downloads,
};

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
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
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
                spawn(async move {
                    let _ = crate::server::subsonic_sync::sync_server_library(false).await;
                });
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
            class: if cfg!(target_os = "android") { "px-4 pt-2 pb-28 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },

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
                                        .map(|t| t.id.key().into_owned())
                                        .collect();
                                    let s = active_source.peek().clone();
                                    spawn(async move {
                                        if !refs.is_empty()
                                            && s.add_to_playlist(&playlist_id, &refs).await.is_ok()
                                        {
                                            gens.bump(Table::Playlists);
                                        }
                                    });
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
                                        .map(|t| t.id.key().into_owned())
                                        .collect();
                                    let s = active_source.peek().clone();
                                    spawn(async move {
                                        if !refs.is_empty()
                                            && s.create_playlist(&name, &refs).await.is_ok()
                                        {
                                            gens.bump(Table::Playlists);
                                        }
                                    });
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
    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
    let is_offline = use_context::<Signal<bool>>();
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
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
        reader::sort::available_album_fields(&albums_res.read().clone().unwrap_or_default())
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
        reader::sort::sort_albums(&mut albums, &album_sort.read());
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
                            let cover_url = ::server::cover::from_path(&config.read(), album.cover_path.as_deref(), 360);
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
                                                            let album_src = active_source.peek().clone();
                                                            let album_id = id.clone();
                                                            spawn(async move {
                                                                let mut tracks = album_src.album_tracks(&album_id).await.unwrap_or_default();
                                                                tracks.sort_by(|a, b| {
                                                                    a.track_number.cmp(&b.track_number)
                                                                        .then_with(|| a.title.cmp(&b.title))
                                                                });
                                                                ctrl.add_to_queue(tracks);
                                                            });
                                                        }
                                                        AlbumAction::Playlist => {
                                                            pending_album_id_for_playlist.set(Some(id.clone()));
                                                            show_album_playlist_modal.set(true);
                                                        }
                                                        AlbumAction::Remove => {
                                                            if cap.delete_from_disk {
                                                                let album_src = active_source.peek().clone();
                                                                let album_id = id.clone();
                                                                spawn(async move {
                                                                    let to_delete = album_src.album_tracks(&album_id).await.unwrap_or_default();
                                                                    for track in &to_delete {
                                                                        if let Some(path) = track.id.local_path() {
                                                                            let _ = std::fs::remove_file(path);
                                                                        }
                                                                    }
                                                                    if album_src.delete_album(&album_id).await.is_ok() {
                                                                        gens.bump(Table::Tracks);
                                                                        gens.bump(Table::Albums);
                                                                    }
                                                                });
                                                            } else {
                                                                // Server: drop every same-titled album's cache.
                                                                let album_src = active_source.peek().clone();
                                                                let all = albums_res.read().clone().unwrap_or_default();
                                                                let ids: Vec<String> = all.iter().filter(|a| a.title == title).map(|a| a.id.clone()).collect();
                                                                spawn(async move {
                                                                    for aid in &ids {
                                                                        let _ = album_src.delete_album(aid).await;
                                                                    }
                                                                    gens.bump(Table::Tracks);
                                                                    gens.bump(Table::Albums);
                                                                });
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
    mut queue: Signal<Vec<reader::models::Track>>,
    current_queue_index: Signal<usize>,
    on_close: EventHandler<()>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let nav_ctrl = use_context::<components::NavigationController>();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
    let is_offline = use_context::<Signal<bool>>();
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let album_id_memo = use_memo(use_reactive!(|album_id_str| album_id_str));
    let album_res = use_album(source, album_id_memo);
    let albums_res = use_albums(source);

    // Discover albums are opened by their YT browse id (MPRE…) and aren't in the
    // local DB until saved. When the DB has no row for the id, fetch the album
    // straight from the catalog remote by that browse id so every searched /
    // discovered album renders (header + full track list) instead of "not found".
    let direct_remote_res: Resource<Option<::server::source::RemoteAlbum>> = {
        use_resource(move || {
            let want = !*is_offline.read();
            let db_has = album_res.read().clone().flatten().is_some();
            let id = album_id_memo();
            let src = active_source.peek().clone();
            utils::offload(async move {
                if !want || db_has || id.trim().is_empty() {
                    return None;
                }
                src.fetch_album_by_ref(&id).await.ok().flatten()
            })
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
                            artist: remote.artist.unwrap_or_default(),
                            year: remote.year,
                            browse_id: Some(remote.browse_id),
                            local_cover: remote.thumbnail.map(utils::cover_url_from_string),
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
        use_resource(move || {
            let _ = gens.generation(Table::Tracks);
            let (src, ids) = (active_source(), matching_ids());
            utils::offload(async move {
                let mut out = Vec::new();
                for id in &ids {
                    out.extend(src.album_tracks(id).await.unwrap_or_default());
                }
                out
            })
        })
    };

    // Catalog remotes (YT) store albums under a title+artist hash with no
    // browse id, so the library only ever holds the few tracks the user saved —
    // an album page would show 1 of 18 songs. Resolve the album's browse id on
    // demand and fetch the full album (header + every track), the way YT Music
    // shows it. `None` for local/other sources (gated on `discover`) and while
    // offline; drives both the full track list and the YT-styled header.
    let remote_album_res: Resource<Option<::server::source::RemoteAlbum>> = {
        use_resource(move || {
            let want = caps().albums == ::server::source::AlbumType::YtMusic && !*is_offline.read();
            let album = album_res.read().clone().flatten();
            let src = active_source.peek().clone();
            utils::offload(async move {
                let album = album?;
                if !want || album.title.trim().is_empty() {
                    return None;
                }
                src.fetch_album_by_meta(&album.title, &album.artist)
                    .await
                    .ok()
                    .flatten()
            })
        })
    };

    let full_album_tracks_res = use_resource(move || {
        let want = caps().albums == ::server::source::AlbumType::Standard
            && caps().sync
            && !*is_offline.read();
        let ids = matching_ids();
        let src = active_source.peek().clone();
        utils::offload(async move {
            if !want {
                return Vec::new();
            }
            let mut out: Vec<reader::models::Track> = Vec::new();
            for id in &ids {
                out.extend(src.fetch_album_tracks(id).await.unwrap_or_default());
            }
            out
        })
    });

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

        let mut tracks: Vec<reader::models::Track> = tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| !offline || conf.offline_tracks.contains_key(t.id.key().as_ref()))
            .collect();
        if !offline {
            let full = full_album_tracks_res.read().clone().unwrap_or_default();
            if full.len() > tracks.len() {
                tracks = full;
            }
        }
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
    let cover_url = ::server::cover::from_path(&config.read(), album.cover_path.as_deref(), 512);
    let cap = caps();
    let aid = album.id.clone();

    let cover_cache = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.cache_dir().join("covers"))
        .unwrap_or_else(|| PathBuf::from("./cache/covers"));

    // Local-only custom cover reset.
    let cover_reset_action = if cap.edit_tags && album.cover_path.is_some() {
        let aid = aid.clone();
        let delete_cover = album.cover_path.clone();
        let cover_cache = cover_cache.clone();
        Some(rsx! {
            button {
                class: "inline-flex items-center justify-center h-9 w-9 rounded-full text-sm font-medium transition-colors border border-white/12 hover:bg-white/10",
                style: "color: var(--color-white); opacity: 0.6;",
                aria_label: i18n::t("remove_cover").to_string(),
                title: i18n::t("remove_cover").to_string(),
                onclick: move |_| {
                    let aid = aid.clone();
                    let delete_cover = delete_cover.clone();
                    let cover_cache = cover_cache.clone();
                    let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                    spawn(async move {
                        if local.update_album_cover(&aid, None, false).await.is_ok() {
                            gens.bump(Table::Albums);
                        }
                        if let Some(path) = delete_cover
                            && path.starts_with(&cover_cache)
                        {
                            let _ = tokio::fs::remove_file(&path).await;
                        }
                    });
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

    let is_downloading_all = cap.downloads && {
        let q = download_queue.read();
        tracks().iter().any(|t| {
            let key = t.id.key();
            q.items.iter().any(|i| {
                i.id.as_str() == key.as_ref()
                    && matches!(
                        i.status,
                        DownloadStatus::Queued | DownloadStatus::Downloading
                    )
            })
        })
    };

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
    let yt_browse_id = yt_remote.as_ref().map(|a| a.browse_id.clone());

    rsx! {
        div { class: "absolute inset-0 flex flex-col overflow-hidden p-8",
            if cap.albums == ::server::source::AlbumType::YtMusic {
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
                tracks: tracks(),
                on_close,
                enable_metadata: cap.edit_tags,
                show_delete_in_selection: cap.delete_from_disk,
                is_downloading_all,
                on_cover_click: cap.edit_tags.then(|| EventHandler::new(move |_| {
                    let aid = aid_cover.clone();
                    let _ = &aid;
                    #[cfg(not(target_os = "android"))]
                    let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                    #[cfg(not(target_os = "android"))]
                    spawn(async move {
                        let file = rfd::AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await;
                        if let Some(file) = file {
                            let path = file.path().to_path_buf();
                            let Ok(data) = tokio::fs::read(&path).await else { return };
                            let cover_cache = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
                                .map(|d| d.cache_dir().join("covers"))
                                .unwrap_or_else(|| PathBuf::from("./cache/covers"));
                            if let Ok(saved) = reader::utils::save_cover(&aid, &data, path.extension().and_then(|e| e.to_str()), &cover_cache) {
                                let saved_str = saved.to_string_lossy().into_owned();
                                if local.update_album_cover(&aid, Some(&saved_str), true).await.is_ok() {
                                    gens.bump(Table::Albums);
                                }
                            }
                        }
                    });
                })),
                actions: cover_reset_action,
                on_delete_track: cap.delete_from_disk.then(|| EventHandler::new(move |idx: usize| {
                    if let Some(t) = tracks_delete.get(idx)
                        && let Some(track_path) = t.id.local_path()
                        && std::fs::remove_file(track_path).is_ok()
                    {
                        let s = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                        let key = t.id.key().into_owned();
                        spawn(async move {
                            if s.delete_tracks(&[key]).await.is_ok() {
                                gens.bump(Table::Tracks);
                            }
                        });
                    }
                })),
                on_selection_delete: cap.delete_from_disk.then(|| EventHandler::new(move |paths: Vec<PathBuf>| {
                    let mut keys = Vec::new();
                    for path in &paths {
                        if std::fs::remove_file(path).is_ok() {
                            keys.push(path.to_string_lossy().into_owned());
                        }
                    }
                    if !keys.is_empty() {
                        let s = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                        spawn(async move {
                            if s.delete_tracks(&keys).await.is_ok() {
                                gens.bump(Table::Tracks);
                            }
                        });
                    }
                })),
                on_download_track: cap.downloads.then(|| EventHandler::new(move |idx: usize| {
                    if let Some(t) = tracks_download.get(idx) {
                        let key = t.id.key();
                        if key.is_empty() {
                            return;
                        }
                        let key = key.as_ref();
                        let downloaded = config.read().offline_tracks.get(key)
                            .map(|p| std::path::Path::new(p).exists())
                            .unwrap_or(false);
                        if downloaded {
                            delete_downloads(vec![key.to_string()], config, download_queue);
                        } else {
                            queue_downloads(vec![(key.to_string(), t.title.clone(), t.artist.clone())], config, download_queue);
                        }
                    }
                })),
                on_download_all: cap.downloads.then(|| EventHandler::new(move |_: ()| {
                    let requests: Vec<(String, String, String)> = tracks_download_all.iter().filter_map(|t| {
                        let k = t.id.key();
                        (!k.is_empty()).then(|| (k.into_owned(), t.title.clone(), t.artist.clone()))
                    }).collect();
                    queue_downloads(requests, config, download_queue);
                })),
                on_delete_all: cap.downloads.then(|| EventHandler::new(move |_: ()| {
                    let ids: Vec<String> = tracks_delete_all.iter().filter_map(|t| {
                        let k = t.id.key();
                        (!k.is_empty()).then(|| k.into_owned())
                    }).collect();
                    delete_downloads(ids, config, download_queue);
                })),
            }
            }
        }
    }
}

/// YT-Music-style album page: a left meta column (cover, artist link, title,
/// "Album • year", song count · duration, play / shuffle / download) beside the
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
    tracks: Vec<reader::models::Track>,
    on_close: EventHandler<()>,
) -> Element {
    let mut ctrl = use_context::<hooks::use_player_controller::PlayerController>();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let nav_ctrl = use_context::<components::NavigationController>();
    let download_queue = use_context::<Signal<DownloadQueue>>();
    let gens = hooks::db_reactivity::use_generations();
    let cover_for = hooks::use_db_queries::use_cover_resolver(80);

    let mut active_menu = use_signal(|| None::<reader::TrackId>);
    let mut show_playlist_modal = use_signal(|| false);
    let mut playlist_track = use_signal(|| None::<reader::TrackId>);

    let total: u64 = tracks.iter().map(|t| t.duration).sum();
    let dur_min = total / 60;
    let song_count = tracks.len();
    let artist_name = artist;
    let artist_for_nav = artist_name.clone();

    // Current track for the row highlight. Read `current_queue_index`
    // *reactively* (`current_track()` peeks, so the page wouldn't re-render on a
    // skip) so the highlighted row follows next/prev.
    let current_id = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|t| t.id)
    };
    let offline_tracks = config.read().offline_tracks.clone();

    // Whether every album track is downloaded for offline — drives the download
    // button's toggle (download all ⇄ remove all).
    let all_downloaded = !tracks.is_empty()
        && tracks.iter().all(|t| {
            let k = t.id.key();
            offline_tracks
                .get(k.as_ref())
                .map(|p| std::path::Path::new(p).exists())
                .unwrap_or(false)
        });

    let tracks_play_all = tracks.clone();
    let tracks_download_all = tracks.clone();
    let artist_for_nav_btn = artist_name.clone();
    // Prefer the provider's album page; fall back to its first track page.
    let share_url = browse_id
        .as_ref()
        .and_then(|id| match tracks.first().and_then(|t| t.id.service()) {
            Some(config::MusicService::Spotify) => {
                Some(format!("https://open.spotify.com/album/{id}"))
            }
            Some(config::MusicService::YtMusic) => {
                Some(format!("https://music.youtube.com/browse/{id}"))
            }
            _ => None,
        })
        .or_else(|| tracks.first().and_then(|t| active_source.peek().web_url(t)));

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
                            if let Some(y) = year {
                                span { class: "uppercase tracking-wide text-xs font-semibold text-white/40", "{i18n::t(\"album\")}" }
                                span { class: "text-white/30", "•" }
                                span { "{y}" }
                                span { class: "text-white/30", "•" }
                            }
                            span { "{i18n::t_with(\"showcase_song_count\", &[(\"count\", song_count.to_string())])}" }
                            span { class: "text-white/30", "•" }
                            span { "{dur_min} {i18n::t(\"min\")}" }
                        }
                    }
                    div { class: "flex items-center gap-3 mt-1",
                        // Download all / remove downloads.
                        button {
                            class: "w-11 h-11 rounded-full border border-white/15 flex items-center justify-center text-slate-300 hover:text-white hover:border-white/30 transition-colors disabled:opacity-40",
                            title: if all_downloaded { "Remove download".to_string() } else { "Download".to_string() },
                            disabled: download_queue.read().is_active(),
                            onclick: move |_| {
                                if all_downloaded {
                                    let ids: Vec<String> = tracks_download_all.iter().filter_map(|t| {
                                        let k = t.id.key();
                                        (!k.is_empty()).then(|| k.into_owned())
                                    }).collect();
                                    delete_downloads(ids, config, download_queue);
                                } else {
                                    let reqs: Vec<(String, String, String)> = tracks_download_all.iter().filter_map(|t| {
                                        let k = t.id.key();
                                        (!k.is_empty()).then(|| (k.into_owned(), t.title.clone(), t.artist.clone()))
                                    }).collect();
                                    queue_downloads(reqs, config, download_queue);
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
                            class: format!("w-11 h-11 rounded-full border flex items-center justify-center transition-colors {}", if *ctrl.shuffle.read() { "bg-white/10 border-white/30" } else { "text-slate-300 border-white/15 hover:text-white hover:border-white/30" }),
                            style: if *ctrl.shuffle.read() { "color: var(--color-indigo-500);" } else { "" },
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
                            let cover_url = cover_for(&track);
                            let is_menu_open = active_menu.read().as_ref() == Some(&track.id);
                            let is_current = current_id.as_ref() == Some(&track.id);
                            let key = track.id.key().into_owned();
                            let is_downloaded = offline_tracks
                                .get(&key)
                                .map(|p| std::path::Path::new(p).exists())
                                .unwrap_or(false);
                            let row_tracks = tracks.clone();
                            let menu_id = track.id.clone();
                            let pl_id = track.id.clone();
                            let dl_track = track.clone();
                            let q_track = track.clone();
                            rsx! {
                                components::track_row::TrackRow {
                                    key: "{track.id.uid()}",
                                    track: track.clone(),
                                    cover_url,
                                    is_album: true,
                                    hide_delete: true,
                                    row_num: Some(idx + 1),
                                    is_menu_open,
                                    is_currently_playing: is_current,
                                    is_downloaded,
                                    on_start_radio: components::track_row::radio_handler(track.clone()),
                                    on_play: move |_| {
                                        ctrl.queue.set(row_tracks.clone());
                                        ctrl.play_track(idx);
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
                                        let k = dl_track.id.key();
                                        if k.is_empty() {
                                            return;
                                        }
                                        let k = k.as_ref();
                                        let downloaded = config.read().offline_tracks.get(k)
                                            .map(|p| std::path::Path::new(p).exists())
                                            .unwrap_or(false);
                                        if downloaded {
                                            delete_downloads(vec![k.to_string()], config, download_queue);
                                        } else {
                                            queue_downloads(vec![(k.to_string(), dl_track.title.clone(), dl_track.artist.clone())], config, download_queue);
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
                            let refs = vec![id.key().into_owned()];
                            let s = active_source.peek().clone();
                            spawn(async move {
                                if s.add_to_playlist(&playlist_id, &refs).await.is_ok() {
                                    gens.bump(Table::Playlists);
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        playlist_track.set(None);
                    },
                    on_create_playlist: move |name: String| {
                        if let Some(id) = playlist_track.read().clone() {
                            let refs = vec![id.key().into_owned()];
                            let s = active_source.peek().clone();
                            spawn(async move {
                                if s.create_playlist(&name, &refs).await.is_ok() {
                                    gens.bump(Table::Playlists);
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        playlist_track.set(None);
                    },
                }
            }
        }
    }
}
