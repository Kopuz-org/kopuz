use components::metadata_modal::MetadataModal;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::showcase::{self, SortField};
use components::track_row::TrackRow;
use components::virtual_scroll::{VirtualScrollView, use_virtual_scroll};
use config::{AppConfig, UiStyle};
use dioxus::prelude::*;
use hooks::use_db_queries::{use_active_source, use_favorites, use_tracks_by_keys};
use hooks::use_player_controller::PlayerController;
use kopuz_route::Route;
use std::collections::HashSet;
use std::rc::Rc;

const ITEM_HEIGHT: f64 = 60.0;

/// The source-agnostic Favorites body. Renders local or any server: covers via
/// the source seam, the favorites partition keyed on the active source, and the
/// remote-sync/download affordances gated on [`Capabilities`].
#[component]
pub fn FavoritesBody(
    config: Signal<AppConfig>,
    mut queue: Signal<Vec<api::TrackInfo>>,
    search_query: Signal<String>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<String>);
    let mut metadata_track = use_signal(|| None::<api::TrackInfo>);
    let mut scroll_positions = use_context::<Signal<std::collections::HashMap<Route, f64>>>();
    let saved_scroll = scroll_positions
        .peek()
        .get(&Route::Favorites)
        .copied()
        .unwrap_or(0.0);
    let mut scroll_stat = use_signal(move || saved_scroll);
    let container_height = use_signal(|| 0.0_f64);
    let mut previous_search_query = use_signal(|| search_query.peek().clone());
    use_effect(move || {
        let query = search_query.read().clone();
        if *previous_search_query.peek() != query {
            previous_search_query.set(query);
            scroll_stat.set(0.0);
            scroll_positions.write().insert(Route::Favorites, 0.0);
            let _ = dioxus::document::eval(
                "let el = document.getElementById('favorites-scroll'); if (el) el.scrollTop = 0;",
            );
        }
    });
    // The import runs in the daemon, so what is in flight is a job: the page
    // follows it rather than counting anything itself, and revisiting the page
    // mid-sync shows the sync instead of a blank slate.
    let sync_job = hooks::jobs::use_job_progress(hooks::JobKind::FavoritesSync);
    let is_syncing = use_memo(move || sync_job.read().running);
    let synced_so_far = use_memo(move || sync_job.read().current.unwrap_or(0) as usize);

    // Multi-selection state
    let mut is_selection_mode = use_signal(|| false);
    let mut selected_tracks = use_signal(HashSet::<String>::new);
    let sort_state = use_signal(|| None);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<String>);
    let downloads = hooks::downloads::use_downloads();

    let source = use_active_source();
    let caps = hooks::sources::use_capabilities();
    let active_source_info = hooks::sources::use_active_source_info();
    let favorites_res = use_favorites();
    let fav_keys = use_memo(move || favorites_res.read().clone().unwrap_or_default());
    let fav_tracks_res = use_tracks_by_keys(source, fav_keys);

    let search_query_normalized = search_query.read().trim().to_lowercase();
    let loaded_tracks = fav_tracks_res.read().clone().unwrap_or_default();
    let has_favorites = !loaded_tracks.is_empty();
    let displayed_tracks: Vec<(api::TrackInfo, Option<utils::CoverUrl>)> = {
        loaded_tracks
            .into_iter()
            .filter(|track| track_matches_filter(track, &search_query_normalized))
            .map(|t| {
                let cover_url = hooks::artwork::for_track(&t, hooks::artwork::Size::Thumb);
                (t, cover_url)
            })
            .collect()
    };

    let sorted_displayed_tracks =
        showcase::sorted_track_pairs(&displayed_tracks, *sort_state.read());

    // Rc, not a Vec clone per row: the play handler needs the whole sorted
    // list as the queue, and cloning 800+ tracks × 800+ rows was quadratic.
    let queue_tracks: Rc<Vec<api::TrackInfo>> = Rc::new(
        sorted_displayed_tracks
            .iter()
            .map(|(t, _)| t.clone())
            .collect(),
    );

    let currently_playing_path = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|track| track.uid.clone())
    };

    let displayed_tracks_for_selection = sorted_displayed_tracks.clone();
    let is_empty = displayed_tracks.is_empty();
    let is_vaxry = config.read().ui_style == UiStyle::Vaxry;

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
            let track_path = track.key.clone();
            let track_select = track.key.clone();
            let track_add = track.clone();
            let track_queue = track.clone();
            let track_meta = track.clone();
            let track_delete = track.clone();
            let queue_source = queue_tracks.clone();
            let track_key = track.uid.clone();
            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.uid);
            let is_selected = selected_tracks.read().contains(&track_path);
            let matches_current_path = currently_playing_path.as_ref() == Some(&track.uid);

            let item_id: String = track.key.clone();
            let is_downloaded = cap.downloads
                && config
                    .read()
                    .offline_tracks
                    .get(&item_id)
                    .map(|p| std::path::Path::new(p).exists())
                    .unwrap_or(false);
            let is_downloading = cap.downloads && downloads.read().is_active(&item_id);
            let item_id_dl = item_id.clone();

            rsx! {
                div { key: "{track_key}", style: "height: {ITEM_HEIGHT}px;",
                TrackRow {
                    track: track.clone(),
                    cover_url: cover_url.clone(),
                    on_start_radio: components::track_row::radio_handler(track.key.clone()),
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
                        if active_menu_track.read().as_ref() == Some(&track_menu.uid) {
                            active_menu_track.set(None);
                        } else {
                            active_menu_track.set(Some(track_menu.uid.clone()));
                        }
                    },
                    on_add_to_playlist: move |_| {
                        selected_track_for_playlist.set(Some(track_add.key.clone()));
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
                        hooks::library_actions::delete_tracks(
                            vec![track_delete.key.clone()],
                            cap.delete_from_disk,
                        );
                    },
                    on_download: cap.downloads.then(|| EventHandler::new(move |_| {
                        if !is_downloaded {
                            active_menu_track.set(None);
                            hooks::downloads::start(
                                vec![item_id_dl.clone()],
                            );
                        }
                    })),
                    on_play: move |_| {
                        ctrl.play_queue_at((*queue_source).clone(), idx);
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
                            let refs: Vec<String> =
                                selected_paths.clone();
                            hooks::playlist_actions::add_tracks(playlist_id.clone(), refs);
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
                            let refs: Vec<String> =
                                selected_paths.clone();
                            hooks::playlist_actions::create_with(name.clone(), refs);
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
                    on_save: move |patch: api::TrackMetadataPatch| {
                        hooks::library_actions::edit_track(patch);
                        metadata_track.set(None);
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
                            .filter(|(t, _)| selected.contains(&t.key))
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
                        let keys: Vec<String> = selected_tracks
                            .read()
                            .iter()
                            .cloned()
                            .collect();
                        hooks::library_actions::delete_tracks(keys, caps().delete_from_disk);
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
            if *is_syncing.read() && caps().favorites_sync == api::FavoritesSyncMode::Instant {
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
                    caps().favorites_sync == api::FavoritesSyncMode::Paginated;
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
                                class: "px-3 py-1 rounded-lg bg-white/10 hover:bg-white/20 text-white/80 transition-colors disabled:opacity-50",
                                disabled: syncing,
                                onclick: move |_| {
                                    hooks::jobs::start(hooks::JobKind::FavoritesSync);
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
                        // A source usable without an account has nothing to show
                        // until someone signs in; the daemon says which it is.
                        let anonymous = active_source_info
                            .read()
                            .as_ref()
                            .is_some_and(|source| source.anonymous);
                        let add_hint = i18n::t("heart_track_to_add");
                        let no_results = i18n::t_with(
                            "no_results_found",
                            &[("query", search_query.read().trim().to_string())],
                        );
                        rsx! {
                            div {
                                class: "flex flex-col items-center justify-center h-64 text-slate-500 text-center px-6",
                                if has_favorites && !search_query_normalized.is_empty() {
                                    i { class: "fa-solid fa-magnifying-glass text-4xl mb-4 opacity-30" }
                                    p {
                                        class: "text-base",
                                        "{no_results}"
                                    }
                                } else if anonymous {
                                    i { class: "fa-solid fa-right-to-bracket text-4xl mb-4 opacity-50" }
                                    p { class: "text-base", "{i18n::t(\"source_anon_favorites\")}" }
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
                    class: "flex items-center gap-3 mb-4 px-2 text-sm font-medium text-slate-500",
                    button {
                        class: if displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.key)) {
                            "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                        } else {
                            "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                        },
                        aria_label: i18n::t("select_all_tracks"),
                        onclick: move |_| {
                            let all_selected = !displayed_tracks.is_empty() && displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.key));
                            if all_selected {
                                selected_tracks.write().clear();
                                is_selection_mode.set(false);
                            } else {
                                selected_tracks.set(displayed_tracks.iter().map(|(track, _)| track.key.clone()).collect());
                                is_selection_mode.set(true);
                            }
                        },
                        if displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.key)) {
                            i { class: "fa-solid fa-check", style: "font-size: 9px;" }
                        }
                    }
                    span { "{i18n::t(\"select_all\")}" }
                }
                div {
                    class: if is_vaxry {
                        "grid px-3 py-2 text-[10px] font-bold border-b mb-1"
                    } else {
                        "grid gap-6 px-2 py-2 border-b border-white/5 text-sm font-medium text-slate-500 mb-2"
                    },
                    style: if is_vaxry {
                        "grid-template-columns: 40px 1fr 180px 180px 56px 40px; color: rgba(255,255,255,0.25); border-color: rgba(255,255,255,0.06);"
                    } else {
                        "grid-template-columns: 40px minmax(0, 1fr) 200px 200px 64px 40px; align-items: center;"
                    },
                    div {}
                    button {
                        class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Title),
                        "{i18n::t(\"title\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Title)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Artist),
                        "{i18n::t(\"artist\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Artist)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center gap-1 text-left hover:text-white transition-colors",
                        onclick: move |_| showcase::toggle_sort_state(sort_state, SortField::Album),
                        "{i18n::t(\"album\")}"
                        i { class: "{showcase::sort_icon(*sort_state.read(), SortField::Album)} text-[10px]" }
                    }
                    button {
                        class: "flex items-center justify-end gap-1 text-right hover:text-white transition-colors",
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

fn track_matches_filter(track: &api::TrackInfo, query: &str) -> bool {
    query.is_empty()
        || track.title.to_lowercase().contains(query)
        || track.artist.to_lowercase().contains(query)
        || track.album.to_lowercase().contains(query)
        || track
            .artists
            .iter()
            .any(|artist| artist.to_lowercase().contains(query))
}

#[cfg(test)]
mod tests {
    use super::track_matches_filter;

    fn track() -> api::TrackInfo {
        api::TrackInfo {
            key: "test.flac".to_string(),
            uid: "test.flac".to_string(),
            album_id: "album-id".to_string(),
            title: "Midnight City".to_string(),
            artist: "M83".to_string(),
            album: "Hurry Up, We're Dreaming".to_string(),
            duration_ms: Some(244_000),
            khz: 44_100,
            track_number: Some(11),
            disc_number: Some(1),
            artists: vec!["Anthony Gonzalez".to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn favorites_filter_matches_track_metadata_case_insensitively() {
        let track = track();

        for query in ["", "midnight", "M83", "dreaming", "GONZALEZ"] {
            assert!(track_matches_filter(&track, &query.to_lowercase()));
        }
        assert!(!track_matches_filter(&track, "unrelated"));
    }
}
