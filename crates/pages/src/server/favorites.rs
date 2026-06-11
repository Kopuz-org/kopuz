use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};
use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use components::playlist_modal::PlaylistModal;
use components::selection_bar::SelectionBar;
use components::showcase::{self, SortField};
use components::track_row::TrackRow;
use config::{AppConfig, MusicService, UiStyle};
use db::Source;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_favorites, use_tracks_by_keys};
use hooks::use_player_controller::PlayerController;
use std::collections::HashSet;
use std::path::PathBuf;
use tracing::Instrument;

#[component]
pub fn JellyfinFavorites(
    config: Signal<AppConfig>,
    mut queue: Signal<Vec<reader::models::Track>>,
) -> Element {
    let mut ctrl = use_context::<PlayerController>();
    let mut active_menu_track = use_signal(|| None::<PathBuf>);
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
    let mut selected_tracks = use_signal(|| HashSet::<PathBuf>::new());
    let sort_state = use_signal(|| None);
    let mut show_playlist_modal = use_signal(|| false);
    let mut selected_track_for_playlist = use_signal(|| None::<PathBuf>);
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let gens = hooks::db_reactivity::use_generations();
    let active_server_id = use_memo(move || {
        let c = config.read();
        c.active_server_id
            .clone()
            .or_else(|| c.server.as_ref().and_then(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let server_source = use_memo(move || Source::Server(active_server_id()));
    let favorites_res = use_favorites(active_server_id);
    let fav_keys = use_memo(move || favorites_res.read().clone().unwrap_or_default());
    let fav_tracks_res = use_tracks_by_keys(server_source, fav_keys);

    use_effect(move || {
        let nonce = *refresh_nonce.read();

        let token = match config.peek().server.as_ref().and_then(|s| s.access_token.clone()) {
            Some(t) => t,
            None => return,
        };
        let service = config.peek().server.as_ref().map(|s| s.service);
        let is_ytmusic = service == Some(MusicService::YtMusic);

        let db = consume_context::<db::Db>();
        let sid = active_server_id();
        spawn(async move {
            if is_ytmusic && nonce == 0 {
                let stamps: Option<serde_json::Value> = db
                    .meta_get("yt_sync", "timestamps")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|s| serde_json::from_str(&s).ok());
                let already_synced = stamps
                    .as_ref()
                    .and_then(|v| v.get("last_yt_sync_at"))
                    .and_then(|v| v.as_u64())
                    .is_some();
                if already_synced {
                    return;
                }
            }

            is_syncing.set(true);
            synced_so_far.set(0);
            let device_id = config.peek().device_id.clone();
            let server_snapshot = config.peek().server.clone();
            let Some(server) = server_snapshot else {
                is_syncing.set(false);
                return;
            };
            let user_id = server.user_id.clone().unwrap_or_default();
            let url = server.url.clone();
            let source = Source::Server(sid.clone());

            let _ids: Vec<String> = match server.service {
                MusicService::Jellyfin => {
                    let remote =
                        JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                    remote
                        .get_favorite_items()
                        .await
                        .map(|items| items.into_iter().map(|i| i.id).collect())
                        .unwrap_or_default()
                }
                MusicService::Subsonic | MusicService::Custom => {
                    let remote = SubsonicClient::new(&url, &user_id, &token);
                    remote.get_starred_song_ids().await.unwrap_or_default()
                }
                MusicService::YtMusic => {
                    let yt =
                        ::server::ytmusic::YouTubeMusicClient::with_cookies(token);

                    let mut accumulated: Vec<reader::models::Track> = Vec::new();
                    let result = yt
                        .stream_liked_songs(|page| {
                            accumulated.extend(page.iter().cloned());
                            let albums = synthesize_albums(&accumulated);
                            synced_so_far.set(accumulated.len());
                            let db = db.clone();
                            let source = Source::Server(sid.clone());
                            spawn(async move {
                                for chunk in page.chunks(100) {
                                    let _ = db.upsert_tracks(&source, chunk).await;
                                }
                                for chunk in albums.chunks(100) {
                                    let _ = db.upsert_albums(&source, chunk).await;
                                }
                                gens.bump_coalesced(Table::Tracks);
                                gens.bump_coalesced(Table::Albums);
                            });
                        })
                        .await;

                    match result {
                        Ok(()) => {
                            let ids: Vec<String> = accumulated
                                .iter()
                                .filter_map(|t| {
                                    let k = t.id.key();
                                    (!k.is_empty()).then(|| k.to_string())
                                })
                                .collect();
                            // Full stream completed — drop YT rows no longer
                            // liked remotely (replaces the legacy clear).
                            let mut keep_albums: Vec<String> = accumulated
                                .iter()
                                .map(|t| t.album_id.clone())
                                .collect();
                            keep_albums.sort();
                            keep_albums.dedup();
                            let _ = db.prune_source(&source, &ids, &keep_albums).await;
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            let mut stamps: serde_json::Value = db
                                .meta_get("yt_sync", "timestamps")
                                .await
                                .ok()
                                .flatten()
                                .and_then(|s| serde_json::from_str(&s).ok())
                                .unwrap_or_else(|| serde_json::json!({}));
                            stamps["last_yt_sync_at"] = serde_json::json!(now);
                            let _ = db
                                .meta_put("yt_sync", "timestamps", &stamps.to_string())
                                .await;
                            let liked_cover =
                                accumulated.first().and_then(|t| t.cover.as_deref()).map(
                                    |c| {
                                        if c.starts_with("http://") || c.starts_with("https://") {
                                            utils::jellyfin_image::encode_cover_url(c)
                                        } else {
                                            c.to_string()
                                        }
                                    },
                                );
                            if db
                                .upsert_playlist_meta(
                                    &source,
                                    "LM",
                                    "Liked Songs",
                                    None,
                                    liked_cover.as_deref(),
                                )
                                .await
                                .is_ok()
                                && db.set_playlist_tracks(&source, "LM", &ids).await.is_ok()
                            {
                                gens.bump(Table::Playlists);
                            }
                            gens.bump(Table::Tracks);
                            gens.bump(Table::Albums);
                            ids
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "YT favorites sync failed");
                            Vec::new()
                        }
                    }
                }
            };

            is_syncing.set(false);
        }.instrument(tracing::info_span!("favorites.sync")));
    });

    let displayed_tracks: Vec<(reader::models::Track, Option<utils::CoverUrl>)> = {
        let server = config.read();
        let server_ref = server.server.as_ref().cloned();

        fav_tracks_res
            .read()
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|t| {
                let cover_url = if let Some(ref srv) = server_ref {
                    utils::map_cover_url(
                        utils::jellyfin_image::resolve_track_cover(
                            t.cover.as_deref(),
                            &t.id.key(),
                            &t.album_id,
                            &srv.url,
                            srv.access_token.as_deref(),
                            80,
                            80,
                        ),
                    )
                } else {
                    None
                };
                (t, cover_url)
            })
            .collect()
    };

    let sorted_displayed_tracks =
        showcase::sorted_track_pairs(&displayed_tracks, *sort_state.read());

    let queue_tracks: Vec<reader::models::Track> = sorted_displayed_tracks
        .iter()
        .map(|(t, _)| t.clone())
        .collect();

    let currently_playing_path = {
        let idx = *ctrl.current_queue_index.read();
        ctrl.get_track_at(idx).map(|track| track.id.uid_path())
    };

    let displayed_tracks_for_selection = sorted_displayed_tracks.clone();
    let is_empty = displayed_tracks.is_empty();
    let is_modern = config.read().ui_style == UiStyle::Modern;

    let tracks_nodes = sorted_displayed_tracks
        .iter()
        .cloned()
        .enumerate()
        .map(|(idx, (track, cover_url))| {
            let track_menu = track.clone();
            let track_path = track.id.uid_path();
            let track_select = track.id.uid_path();
            let track_add = track.clone();
            let track_queue = track.clone();
            let queue_source = queue_tracks.clone();
            let track_key = format!("{}-{}", track.id.uid(), idx);
            let is_menu_open = active_menu_track.read().as_ref() == Some(&track.id.uid_path());
            let is_selected = selected_tracks.read().contains(&track_path);
            let matches_current_path = currently_playing_path.as_ref() == Some(&track.id.uid_path());

            let item_id: String = track.id.key().to_string();
            let is_downloaded = if let Some(path_str) = config.read().offline_tracks.get(&item_id) {
                std::path::Path::new(path_str).exists()
            } else {
                false
            };
            let is_downloading = download_queue.read().items.iter().any(|i| i.id == item_id && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading));
            let item_id_dl = item_id.clone();
            let track_title = track.title.clone();
            let track_artist = track.artist.clone();

            rsx! {
                TrackRow {
                    key: "{track_key}",
                    track: track.clone(),
                    cover_url: cover_url.clone(),
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
                        if active_menu_track.read().as_ref() == Some(&track_menu.id.uid_path()) {
                            active_menu_track.set(None);
                        } else {
                            active_menu_track.set(Some(track_menu.id.uid_path()));
                        }
                    },
                    on_add_to_playlist: move |_| {
                        selected_track_for_playlist.set(Some(track_add.id.uid_path()));
                        show_playlist_modal.set(true);
                        active_menu_track.set(None);
                    },
                    on_queue: move |_| {
                        ctrl.add_to_queue(vec![track_queue.clone()]);
                        active_menu_track.set(None);
                    },
                    on_close_menu: move |_| active_menu_track.set(None),
                    on_delete: move |_| active_menu_track.set(None),
                    hide_delete: true,
                    on_download: move |_| {
                        if !is_downloaded {
                            active_menu_track.set(None);
                            queue_downloads(
                                vec![(item_id_dl.clone(), track_title.clone(), track_artist.clone())],
                                config,
                                download_queue,
                            );
                        }
                    },
                    on_play: move |_| {
                        queue.set(queue_source.clone());
                        ctrl.play_track(idx);
                    },
                }
            }
        });

    rsx! {
        div {
            if *show_playlist_modal.read() {
                PlaylistModal {
                    is_jellyfin: true,
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
                            spawn(async move {
                                let Some(conn) =
                                    ::server::server_ops::ServerConn::resolve(&config.peek())
                                else {
                                    return;
                                };
                                let item_ids: Vec<String> = selected_paths
                                    .iter()
                                    .filter_map(|p| {
                                        ::server::server_ops::parse_item_id(p.to_str()?)
                                            .map(str::to_string)
                                    })
                                    .collect();
                                let _ = ::server::server_ops::add_tracks_to_playlist(
                                    &conn, &pid, &item_ids,
                                )
                                .await;
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
                            spawn(async move {
                                let Some(conn) =
                                    ::server::server_ops::ServerConn::resolve(&config.peek())
                                else {
                                    return;
                                };
                                let item_ids: Vec<String> = selected_paths
                                    .iter()
                                    .filter_map(|p| {
                                        ::server::server_ops::parse_item_id(p.to_str()?)
                                            .map(str::to_string)
                                    })
                                    .collect();
                                if !item_ids.is_empty() {
                                    let _ = ::server::server_ops::create_server_playlist(
                                        &conn,
                                        &playlist_name,
                                        &item_ids,
                                    )
                                    .await;
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

            if is_selection_mode() {
                SelectionBar {
                    count: selected_tracks.read().len(),
                    show_delete: false,
                    on_add_to_queue: move |_| {
                        let selected = selected_tracks.read().clone();
                        if selected.is_empty() {
                            return;
                        }
                        let tracks: Vec<_> = displayed_tracks_for_selection
                            .iter()
                            .filter(|(t, _)| selected.contains(&t.id.uid_path()))
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
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    },
                    on_cancel: move |_| {
                        is_selection_mode.set(false);
                        selected_tracks.write().clear();
                    }
                }
            }

            // Generic "Syncing with server" spinner for non-YT
            // services. YT has its own status row below with track
            // counter + refresh button — don't double-render.
            if *is_syncing.read()
                && config
                    .read()
                    .server
                    .as_ref()
                    .map(|s| s.service != MusicService::YtMusic)
                    .unwrap_or(true)
            {
                div {
                    class: "flex items-center gap-2 text-slate-400 text-sm mb-4",
                    i { class: "fa-solid fa-circle-notch fa-spin" }
                    span { "{i18n::t(\"syncing_with_server\")}" }
                }
            }

            // Sync status row — visible whenever we're syncing or when
            // YT Music is the active server (so the user always has a
            // refresh button to force a re-fetch). Counter ticks up as
            // pages stream in. Stays out of the way for non-YT services
            // because it's a YT-specific affordance.
            {
                let is_ytmusic = config
                    .read()
                    .server
                    .as_ref()
                    .map(|s| s.service == MusicService::YtMusic)
                    .unwrap_or(false);
                let synced = *synced_so_far.read();
                let syncing = *is_syncing.read();
                let total = displayed_tracks.len();
                if is_ytmusic {
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
                {
                    let yt_anon = config
                        .read()
                        .server
                        .as_ref()
                        .map(|s| {
                            s.service == config::MusicService::YtMusic && s.yt_anonymous
                        })
                        .unwrap_or(false);
                    rsx! {
                        div {
                            class: "flex flex-col items-center justify-center h-64 text-slate-500 text-center px-6",
                            if yt_anon {
                                i { class: "fa-solid fa-right-to-bracket text-4xl mb-4 opacity-50" }
                                p { class: "text-base", "{i18n::t(\"yt_anon_favorites\")}" }
                            } else {
                                i { class: "fa-regular fa-heart text-4xl mb-4 opacity-30" }
                                p { class: "text-base", "{i18n::t(\"no_favorites\")}" }
                                p { class: "text-sm mt-1 opacity-70",
                                    "{i18n::t(\"heart_track_to_add_server\")}"
                                }
                            }
                        }
                    }
                }
            } else if !is_empty {
                div {
                    class: "flex items-center gap-3 mb-4 px-2 text-sm font-medium text-slate-500 uppercase tracking-wider",
                    button {
                        class: if displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.id.uid_path())) {
                            "w-4 h-4 rounded border border-indigo-400 bg-indigo-500 text-white flex items-center justify-center transition-colors"
                        } else {
                            "w-4 h-4 rounded border border-white/20 bg-white/5 hover:border-white/50 transition-colors"
                        },
                        aria_label: i18n::t("select_all_tracks"),
                        onclick: move |_| {
                            let all_selected = !displayed_tracks.is_empty() && displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.id.uid_path()));
                            if all_selected {
                                selected_tracks.write().clear();
                                is_selection_mode.set(false);
                            } else {
                                selected_tracks.set(displayed_tracks.iter().map(|(track, _)| track.id.uid_path()).collect());
                                is_selection_mode.set(true);
                            }
                        },
                        if displayed_tracks.iter().all(|(track, _)| selected_tracks.read().contains(&track.id.uid_path())) {
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
                div {
                    class: if is_modern { "" } else { "space-y-1" },
                    {tracks_nodes}
                }
            }
        }
    }
}

pub use JellyfinFavorites as ServerFavorites;

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
