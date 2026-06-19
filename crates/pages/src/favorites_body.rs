use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};
use ::server::source::FavoritesSync;
use components::metadata_modal::MetadataModal;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::showcase::{self, SortField};
use components::track_row::TrackRow;
use components::virtual_scroll::{VirtualScrollView, use_virtual_scroll};
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_active_source, use_favorites, use_tracks_by_keys};
use hooks::use_player_controller::PlayerController;
use kopuz_route::Route;
use std::collections::HashSet;
use std::rc::Rc;
use tracing::Instrument;

const ITEM_HEIGHT: f64 = 60.0;

/// The source-agnostic Favorites body. Renders local or any server: covers via
/// the source seam, the favorites partition keyed on the active source, and the
/// remote-sync/download affordances gated on [`Capabilities`].
#[component]
pub fn FavoritesBody(
    config: Signal<AppConfig>,
    mut queue: Signal<Vec<reader::models::Track>>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<reader::TrackId>);
    let mut metadata_track = use_signal(|| None::<reader::models::Track>);
    let mut scroll_positions = use_context::<Signal<std::collections::HashMap<Route, f64>>>();
    let saved_scroll = scroll_positions
        .peek()
        .get(&Route::Favorites)
        .copied()
        .unwrap_or(0.0);
    let scroll_stat = use_signal(move || saved_scroll);
    let container_height = use_signal(|| 0.0_f64);
    // YT sync state:
    // - `is_syncing`: true while a fetch is in flight
    // - `synced_so_far`: count of tracks streamed into the library so far
    // - `refresh_nonce`: bumped by the manual refresh button to force a
    //   re-sync even when the library already has data on disk
    let mut is_syncing = use_signal(|| false);
    let mut synced_so_far: Signal<usize> = use_signal(|| 0);
    let mut refresh_nonce: Signal<u64> = use_signal(|| 0);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(HashSet::<reader::TrackId>::new);
    let sort_state = use_signal(|| None);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<reader::TrackId>);
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
    // The server id for the remote-sync effect (empty for local, which never syncs).
    let active_server_id = use_memo(move || {
        config
            .read()
            .active_source
            .server_id()
            .map(String::from)
            .unwrap_or_default()
    });
    let favorites_res = use_favorites();
    let fav_keys = use_memo(move || favorites_res.read().clone().unwrap_or_default());
    let fav_tracks_res = use_tracks_by_keys(source, fav_keys);

    use_effect(move || {
        // Only the active server syncs — a configured-but-inactive server (e.g. a
        // YT server while Local is active) must not pull favorites here.
        if !caps().sync {
            return;
        }
        let nonce = *refresh_nonce.read();
        // The capability — not the service — decides how favorites sync.
        let sync_mode = caps().favorites_sync;
        let read_db = consume_context::<hooks::ReadDb>();
        let source = active_source.peek().clone();
        let sid = active_server_id();
        spawn(
            async move {
                // Staleness/once guard: paginated (YT) skips if a prior sync stamped
                // or clean rows already exist; instant skips a recent pull.
                if nonce == 0 {
                    match sync_mode {
                        FavoritesSync::Paginated => {
                            let stamps: Option<serde_json::Value> = read_db
                                .meta_get("yt_sync", "timestamps")
                                .await
                                .ok()
                                .flatten()
                                .and_then(|s| serde_json::from_str(&s).ok());
                            // Dirty rows don't count: a locally-hearted never-pushed
                            // like must not suppress the initial import.
                            let favorites = read_db.favorites(&sid).await.unwrap_or_default().len();
                            let dirty = read_db
                                .dirty_favorites(&sid)
                                .await
                                .unwrap_or_default()
                                .len();
                            let already_synced = stamps
                                .as_ref()
                                .and_then(|v| v.get("last_yt_sync_at"))
                                .and_then(|v| v.as_u64())
                                .is_some()
                                || favorites > dirty;
                            if already_synced {
                                return;
                            }
                        }
                        FavoritesSync::Instant => {
                            let now = unix_now();
                            let last_pull: u64 = read_db
                                .meta_get("fav_pull", &sid)
                                .await
                                .ok()
                                .flatten()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0);
                            if last_pull <= now && now - last_pull < 30 * 60 {
                                return;
                            }
                        }
                    }
                }

                is_syncing.set(true);
                synced_so_far.set(0);

                match sync_mode {
                    FavoritesSync::Instant => {
                        // One shot: the remote set becomes the clean baseline (dirty
                        // local rows survive the replace — mirrors server::sync).
                        if let Ok(ids) = source.fetch_favorites().await
                            && source.replace_favorites_clean(&ids).await.is_ok()
                        {
                            let _ = source
                                .set_meta("fav_pull", &sid, &unix_now().to_string())
                                .await;
                            gens.bump(Table::Favorites);
                        }
                    }
                    FavoritesSync::Paginated => {
                        // One epoch for the whole walk: each page stamps its rows with
                        // it, and the end sweep drops anything not re-stamped (unliked
                        // remotely). Pages stream into the DB + UI live; cross-page
                        // dedup is ours (YT repeats tracks at page boundaries).
                        let epoch = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        let mut seen: HashSet<String> = HashSet::new();
                        let mut ids: Vec<String> = Vec::new();
                        let mut keep_albums: Vec<String> = Vec::new();
                        let mut cursor: Option<String> = None;
                        let mut completed = true;
                        loop {
                            let page = match source.fetch_favorites_page(cursor.clone()).await {
                                Ok(p) => p,
                                Err(e) => {
                                    tracing::warn!(error = %e, "favorites page fetch failed");
                                    completed = false;
                                    break;
                                }
                            };
                            let next = page.next.clone();
                            let fresh: Vec<reader::models::Track> = page
                                .tracks
                                .into_iter()
                                .filter(|t| {
                                    let k = t.id.key().to_string();
                                    !k.is_empty() && seen.insert(k)
                                })
                                .collect();
                            // Empty-after-dedup ⇒ exhausted (looping would hammer the
                            // same continuation with no progress).
                            if fresh.is_empty() {
                                break;
                            }
                            let page_refs: Vec<String> = fresh
                                .iter()
                                .filter_map(|t| {
                                    let k = t.id.key();
                                    (!k.is_empty()).then(|| k.to_string())
                                })
                                .collect();
                            let start_rank = ids.len() as i64;
                            ids.extend(page_refs.iter().cloned());
                            keep_albums.extend(fresh.iter().map(|t| t.album_id.clone()));
                            let albums = synthesize_albums(&fresh);
                            synced_so_far.set(ids.len());
                            for chunk in fresh.chunks(100) {
                                let _ = source.upsert_tracks(chunk).await;
                            }
                            let _ = source.upsert_albums(&albums).await;
                            let _ = source
                                .upsert_favorites_page(&page_refs, start_rank, epoch)
                                .await;
                            gens.bump_coalesced(Table::Tracks);
                            gens.bump_coalesced(Table::Favorites);
                            match next {
                                Some(n) => cursor = Some(n),
                                None => break,
                            }
                        }
                        if completed {
                            // Drop rows no longer liked remotely, then sweep favorites
                            // not re-stamped this epoch. Dirty local toggles survive.
                            keep_albums.sort();
                            keep_albums.dedup();
                            let _ = source.prune(&ids, &keep_albums).await;
                            if source.sweep_favorites(epoch).await.is_ok() {
                                gens.bump(Table::Favorites);
                            }
                            let mut stamps: serde_json::Value = read_db
                                .meta_get("yt_sync", "timestamps")
                                .await
                                .ok()
                                .flatten()
                                .and_then(|s| serde_json::from_str(&s).ok())
                                .unwrap_or_else(|| serde_json::json!({}));
                            stamps["last_yt_sync_at"] = serde_json::json!(unix_now());
                            let _ = source
                                .set_meta("yt_sync", "timestamps", &stamps.to_string())
                                .await;
                            gens.bump(Table::Tracks);
                            gens.bump(Table::Albums);
                        }
                    }
                }

                is_syncing.set(false);
            }
            .instrument(tracing::info_span!("favorites.sync")),
        );
    });

    let displayed_tracks: Vec<(reader::models::Track, Option<utils::CoverUrl>)> = {
        let conf = config.read();
        fav_tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|t| {
                let cover_url = ::server::cover::track(&conf, &t, 80);
                (t, cover_url)
            })
            .collect()
    };

    let sorted_displayed_tracks =
        showcase::sorted_track_pairs(&displayed_tracks, *sort_state.read());

    // Rc, not a Vec clone per row: the play handler needs the whole sorted
    // list as the queue, and cloning 800+ tracks × 800+ rows was quadratic.
    let queue_tracks: Rc<Vec<reader::models::Track>> = Rc::new(
        sorted_displayed_tracks
            .iter()
            .map(|(t, _)| t.clone())
            .collect(),
    );

    let currently_playing_path = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|track| track.id.clone())
    };

    let displayed_tracks_for_selection = sorted_displayed_tracks.clone();
    let is_empty = displayed_tracks.is_empty();
    let is_modern = config.read().ui_style == UiStyle::Modern;

    // Window the rows: only the visible slice (plus buffer) exists in the
    // DOM — the full 800+ row list made every scroll frame repaint a huge
    // layer and re-run per-row work.
    let scroll_info = use_virtual_scroll(
        *scroll_stat.read(),
        *container_height.read(),
        sorted_displayed_tracks.len(),
        ITEM_HEIGHT,
    );

    let tracks_nodes = sorted_displayed_tracks
        .iter()
        .enumerate()
        .skip(scroll_info.start_index)
        .take(scroll_info.items_to_render)
        .map(|(idx, pair)| (idx, pair.clone()))
        .map(|(idx, (track, cover_url))| {
            let cap = caps();
            let track_menu = track.clone();
            let track_path = track.id.clone();
            let track_select = track.id.clone();
            let track_add = track.clone();
            let track_queue = track.clone();
            let track_meta = track.clone();
            let track_delete = track.clone();
            let queue_source = queue_tracks.clone();
            let track_key = track.id.uid();
            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.id);
            let is_selected = selected_tracks.read().contains(&track_path);
            let matches_current_path = currently_playing_path.as_ref() == Some(&track.id);

            let item_id: String = track.id.key().to_string();
            let is_downloaded = cap.downloads
                && config
                    .read()
                    .offline_tracks
                    .get(&item_id)
                    .map(|p| std::path::Path::new(p).exists())
                    .unwrap_or(false);
            let is_downloading = cap.downloads && download_queue.read().items.iter().any(|i| i.id == item_id && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading));
            let item_id_dl = item_id.clone();
            let track_title = track.title.clone();
            let track_artist = track.artist.clone();

            rsx! {
                div { key: "{track_key}", style: "height: {ITEM_HEIGHT}px;",
                TrackRow {
                    track: track.clone(),
                    cover_url: cover_url.clone(),
                    on_start_radio: components::track_row::radio_handler(track.clone()),
                    row_num: Some(idx + 1),
                    is_menu_open,
                    is_album: false,
                    is_currently_playing: matches_current_path,
                    is_selection_mode: is_selection_mode(),
                    is_selected,
                    is_downloaded,
                    is_downloading,
                    on_long_press: move |_| {
                        is_selection_mode.set(true);
                        selected_tracks.write().insert(track_path.clone());
                    },
                    on_select: move |selected| {
                        if selected {
                            is_selection_mode.set(true);
                            selected_tracks.write().insert(track_select.clone());
                        } else {
                            selected_tracks.write().remove(&track_select);
                            if selected_tracks.read().is_empty() {
                                is_selection_mode.set(false);
                            }
                        }
                    },
                    on_click_menu: move |_| {
                        if active_menu_track.read().as_ref() == Some(&track_menu.id) {
                            active_menu_track.set(None);
                        } else {
                            active_menu_track.set(Some(track_menu.id.clone()));
                        }
                    },
                    on_add_to_playlist: move |_| {
                        selected_track_for_playlist.set(Some(track_add.id.clone()));
                        show_playlist_modal.set(true);
                        active_menu_track.set(None);
                    },
                    on_queue: move |_| {
                        ctrl.add_to_queue(vec![track_queue.clone()]);
                        active_menu_track.set(None);
                    },
                    on_close_menu: move |_| active_menu_track.set(None),
                    hide_delete: !cap.delete_from_disk,
                    on_view_metadata: cap.edit_tags.then(|| EventHandler::new(move |_| {
                        metadata_track.set(Some(track_meta.clone()));
                        active_menu_track.set(None);
                    })),
                    on_delete: move |_| {
                        active_menu_track.set(None);
                        if cap.delete_from_disk
                            && let Some(p) = track_delete.id.local_path()
                            && std::fs::remove_file(p).is_ok()
                        {
                            let s = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                            let key = track_delete.id.key().into_owned();
                            spawn(async move {
                                if s.delete_tracks(&[key]).await.is_ok() {
                                    gens.bump(Table::Tracks);
                                }
                            });
                        }
                    },
                    on_download: cap.downloads.then(|| EventHandler::new(move |_| {
                        if !is_downloaded {
                            active_menu_track.set(None);
                            queue_downloads(
                                vec![(item_id_dl.clone(), track_title.clone(), track_artist.clone())],
                                config,
                                download_queue,
                            );
                        }
                    })),
                    on_play: move |_| {
                        queue.set((*queue_source).clone());
                        ctrl.play_track(idx);
                    },
                }
                }
            }
        });

    rsx! {
        div {
            class: "flex-1 min-h-0 flex flex-col",
            if *show_playlist_modal.read() {
                PlaylistModal {
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
                            let pid = playlist_id.clone();
                            let src = active_source.peek().clone();
                            let refs: Vec<String> =
                                selected_paths.iter().map(|p| p.key().into_owned()).collect();
                            spawn(async move {
                                if !refs.is_empty() {
                                    let _ = src.add_to_playlist(&pid, &refs).await;
                                }
                            });
                        }
                        show_playlist_modal.set(false);
                        active_menu_track.set(None);
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
                            let playlist_name = name.clone();
                            let src = active_source.peek().clone();
                            let refs: Vec<String> =
                                selected_paths.iter().map(|p| p.key().into_owned()).collect();
                            spawn(async move {
                                if !refs.is_empty() {
                                    let _ = src.create_playlist(&playlist_name, &refs).await;
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
                                let s = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
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
                    on_add_to_queue: move |_| {
                        let selected = selected_tracks.read().clone();
                        if selected.is_empty() {
                            return;
                        }
                        let tracks: Vec<_> = displayed_tracks_for_selection
                            .iter()
                            .filter(|(t, _)| selected.contains(&t.id))
                            .map(|(track, _)| track.clone())
                            .collect();
                        if !tracks.is_empty() {
                            ctrl.add_to_queue(tracks);
                        }
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_add_to_playlist: move |_| {
                        show_playlist_modal.set(true);
                    },
                    on_delete: move |_| {
                        if caps().delete_from_disk {
                            let paths: Vec<_> = selected_tracks.read().iter().cloned().collect();
                            let mut keys = Vec::new();
                            for id in paths {
                                let Some(path) = id.local_path() else {
                                    continue;
                                };
                                if std::fs::remove_file(path).is_ok() {
                                    keys.push(id.key().into_owned());
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
                        }
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_cancel: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    }
                }
            }

            // Generic "Syncing with server" spinner for instant-sync sources.
            // Paginated sources (YT) have their own progress row below with a
            // track counter + refresh button — don't double-render.
            if *is_syncing.read() && caps().favorites_sync == FavoritesSync::Instant {
                div {
                    class: "flex items-center gap-2 text-slate-400 text-sm mb-4",
                    i { class: "fa-solid fa-circle-notch fa-spin" }
                    span { "{i18n::t(\"syncing_with_server\")}" }
                }
            }

            // Sync status row with a force-refresh button — shown for sources whose
            // favorites arrive page-by-page (the counter ticks up as pages stream
            // in). Sources with instant favorites have nothing to page, so it stays
            // out of the way.
            {
                let is_paginated_sync =
                    caps().favorites_sync == ::server::source::FavoritesSync::Paginated;
                let synced = *synced_so_far.read();
                let syncing = *is_syncing.read();
                let total = displayed_tracks.len();
                if is_paginated_sync {
                    rsx! {
                        div {
                            class: "flex items-center justify-between gap-3 mb-3 px-2 text-xs text-slate-400",
                            div {
                                class: "flex items-center gap-2",
                                if syncing {
                                    i { class: "fa-solid fa-arrows-rotate fa-spin text-indigo-300" }
                                    span {
                                        "{i18n::t_with(\"yt_syncing_progress\", &[(\"count\", synced.to_string())])}"
                                    }
                                } else if total > 0 {
                                    i { class: "fa-solid fa-check text-emerald-400" }
                                    span {
                                        "{i18n::t_with(\"yt_synced_total\", &[(\"count\", total.to_string())])}"
                                    }
                                }
                            }
                            button {
                                class: "px-3 py-1 rounded bg-white/5 hover:bg-white/10 text-white/80 transition-colors disabled:opacity-50",
                                disabled: syncing,
                                onclick: move |_| {
                                    let next = *refresh_nonce.peek() + 1;
                                    refresh_nonce.set(next);
                                },
                                i { class: "fa-solid fa-arrows-rotate mr-1" }
                                "{i18n::t(\"refresh\")}"
                            }
                        }
                    }
                } else {
                    rsx! {}
                }
            }

            if is_empty && !*is_syncing.read() {
                if fav_tracks_res.read().is_none() {
                    div { class: "flex items-center justify-center py-12",
                        i { class: "fa-solid fa-spinner fa-spin text-3xl text-white/20" }
                    }
                } else {
                    {
                        // Anonymous YT shows a sign-in prompt; otherwise the
                        // standard empty state with a source-appropriate hint.
                        let yt_anon = caps().discover
                            && config
                                .read()
                                .server
                                .as_ref()
                                .map(|s| s.yt_anonymous)
                                .unwrap_or(false);
                        let add_hint = i18n::t("heart_track_to_add");
                        rsx! {
                            div {
                                class: "flex flex-col items-center justify-center h-64 text-slate-500 text-center px-6",
                                if yt_anon {
                                    i { class: "fa-solid fa-right-to-bracket text-4xl mb-4 opacity-50" }
                                    p { class: "text-base", "{i18n::t(\"yt_anon_favorites\")}" }
                                } else {
                                    i { class: "fa-regular fa-heart text-4xl mb-4 opacity-30" }
                                    p { class: "text-base", "{i18n::t(\"no_favorites\")}" }
                                    p { class: "text-sm mt-1 opacity-70", "{add_hint}" }
                                }
                            }
                        }
                    }
                }
            } else if !is_empty {
                div {
                    class: "flex items-center gap-3 mb-4 px-2 text-sm font-medium text-slate-500 uppercase tracking-wider",
                    button {
                        class: if displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.id)) {
                            "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                        } else {
                            "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                        },
                        aria_label: i18n::t("select_all_tracks"),
                        onclick: move |_| {
                            let all_selected = !displayed_tracks.is_empty() && displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.id));
                            if all_selected {
                                selected_tracks.write().clear();
                                is_selection_mode.set(false);
                            } else {
                                selected_tracks.set(displayed_tracks.iter().map(|(track, _)| track.id.clone()).collect());
                                is_selection_mode.set(true);
                            }
                        },
                        if displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.id)) {
                            i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                        }
                    }
                    span { "{i18n::t(\"select_all\")}" }
                }
                div {
                    class: if is_modern {
                        "grid px-3 py-2 text-[10px] font-bold uppercase tracking-widest border-b mb-1"
                    } else {
                        "grid gap-6 px-2 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2 uppercase tracking-wider"
                    },
                    style: if is_modern {
                        "grid-template-columns: 40px 1fr 180px 180px 56px 40px; color: rgba(255,255,255,0.25); border-color: rgba(255,255,255,0.06);"
                    } else {
                        "grid-template-columns: 40px minmax(0, 1fr) 200px 200px 64px 40px; align-items: center;"
                    },
                    div {}
                    button {
                        class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Title),
                        "{i18n::t(\"title\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Title)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Artist),
                        "{i18n::t(\"artist\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Artist)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center gap-1 uppercase tracking-wider text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Album),
                        "{i18n::t(\"album\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Album)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center justify-end gap-1 uppercase tracking-wider text-right hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Duration),
                        i { class: "fa-regular fa-clock" }
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Duration)} text-[10px]" }
                    }
                    div {}
                }
                VirtualScrollView {
                    id: "favorites-scroll".to_string(),
                    class: if cfg!(target_os = "android") { "flex-1 overflow-y-auto overflow-x-hidden pb-20".to_string() } else { "flex-1 overflow-y-auto pb-20".to_string() },
                    scroll_stat,
                    container_height,
                    item_height: ITEM_HEIGHT,
                    saved_scroll,
                    top_pad: scroll_info.top_pad,
                    bottom_pad: scroll_info.bottom_pad,
                    onscroll: move |scroll| {
                        scroll_positions.write().insert(Route::Favorites, scroll);
                    },
                    {tracks_nodes}
                }
            }
        }
    }
}

/// Seconds since the Unix epoch (0 on a backward clock step).
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build a list of synthetic Album entries out of the user's YT tracks.
/// YT doesn't expose a separate albums endpoint, so we group by
/// Track.album_id (assigned in search.rs::synthesize_album_id) and pick
/// the first track per group as the album's representative for title +
/// artist + cover.
fn synthesize_albums(tracks: &[reader::models::Track]) -> Vec<reader::models::Album> {
    use std::collections::HashMap;
    use std::path::PathBuf;
    let mut by_album: HashMap<String, &reader::models::Track> = HashMap::new();
    for t in tracks {
        if t.album_id.is_empty() {
            continue;
        }
        by_album.entry(t.album_id.clone()).or_insert(t);
    }
    by_album
        .into_iter()
        .map(|(album_id, t)| {
            // Reuse the first track's thumbnail as the album cover, in the
            // form `jellyfin_image_url_from_path` decodes: a raw URL via the
            // `directurl:` prefix, an already-embedded tag via `ytmusic:_:`.
            let cover_path = t.cover.as_deref().map(|c| {
                if c.starts_with("http://") || c.starts_with("https://") {
                    PathBuf::from(format!("directurl:{c}"))
                } else {
                    PathBuf::from(format!("ytmusic:_:{c}"))
                }
            });
            reader::models::Album {
                id: album_id,
                title: if t.album.is_empty() {
                    "Singles".to_string()
                } else {
                    t.album.clone()
                },
                artist: t.artist.clone(),
                genre: String::new(),
                year: 0,
                cover_path,
                manual_cover: false,
            }
        })
        .collect()
}
