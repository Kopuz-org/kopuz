//! Source-agnostic Playlists page (issue #35). The chrome (header, add-playlist,
//! folder/playlist detail) is shared; the grid renders folders + local management
//! when the source organises playlists in folders ([`Capabilities::folders`]), or
//! a flat remote list with per-card downloads + sync otherwise. No `is_server()`
//! dispatch — every divergence gates on the resolved source's capabilities.

use components::dots_menu::{DotsMenu, MenuAction};
use components::folder_picker::FolderPickerModal;
use components::playlist_detail::PlaylistDetail;
use components::playlist_popups::AddPlaylistPopup;
use config::{AppConfig, MusicService, Source, UiStyle};
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_active_source, use_playlists, use_tracks_by_keys};
use tracing::Instrument;

use crate::server::download_manager::{
    DownloadQueue, DownloadStatus, delete_downloads, queue_downloads,
};

#[component]
#[tracing::instrument(name = "render.playlists_page", skip_all)]
pub fn PlaylistsPage(
    config: Signal<AppConfig>,
    mut selected_playlist_id: Signal<Option<String>>,
) -> Element {
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());

    let mut show_add_playlist = use_signal(|| false);
    let mut playlist_name = use_signal(String::new);
    let mut error = use_signal(|| Option::<String>::None);
    let mut saving = use_signal(|| false);
    let mut playlist_refresh_trigger = use_signal(|| 0u64);

    let gens = hooks::db_reactivity::use_generations();
    let playlists_res = use_playlists();
    let sel_server_refs = use_memo(move || {
        let store = playlists_res.read().clone().unwrap_or_default();
        selected_playlist_id
            .read()
            .as_ref()
            .and_then(|pid| store.playlists.iter().find(|p| p.id == *pid))
            .map(|p| p.tracks.clone())
            .unwrap_or_default()
    });
    let sel_server_tracks_res = use_tracks_by_keys(source, sel_server_refs);

    let handle_add_playlist = move |_| {
        if saving() {
            return;
        }
        let name = playlist_name();
        // A source that can't mutate playlists (a creds-less/offline server, or a
        // read-only source) gets the friendly message instead of a raw error.
        if caps().playlists == ::server::source::PlaylistOps::None {
            error.set(Some(i18n::t("error_server_not_configured").to_string()));
            return;
        }
        let s = active_source.peek().clone();
        error.set(None);
        saving.set(true);
        spawn(async move {
            let result = s.create_playlist(&name, &[]).await;
            saving.set(false);
            match result {
                Ok(_) => {
                    // A server create mirrors into the DB but a re-sync still
                    // reconciles remote-side details, so the sync path re-fetches.
                    if caps().sync {
                        playlist_refresh_trigger.with_mut(|v| *v += 1);
                    } else {
                        gens.bump(Table::Playlists);
                    }
                    show_add_playlist.set(false);
                    playlist_name.set(String::new());
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
                }
            }
        });
    };

    let download_queue = use_context::<Signal<DownloadQueue>>();

    let mut last_source = use_signal(|| config.read().active_source.clone());
    if *last_source.read() != config.read().active_source {
        selected_playlist_id.set(None);
        last_source.set(config.read().active_source.clone());
    }

    let is_modern = config.read().ui_style == UiStyle::Modern;

    rsx! {
        div { class: if cfg!(target_os = "android") { "px-4 pt-2 pb-28 absolute inset-0 flex flex-col" } else if is_modern { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },
            if let Some(pid) = selected_playlist_id.read().clone() {
                {
                    let pid_for_dl = pid.clone();
                    let is_downloading_all = {
                        let store = playlists_res.read().clone().unwrap_or_default();
                        let track_ids = store
                            .playlists
                            .iter()
                            .find(|p| p.id == pid)
                            .map(|p| p.tracks.clone())
                            .unwrap_or_default();
                        let q = download_queue.read();
                        track_ids.iter().any(|tid| {
                            q.items.iter().any(|i| {
                                &i.id == tid
                                    && matches!(
                                        i.status,
                                        DownloadStatus::Queued | DownloadStatus::Downloading
                                    )
                            })
                        })
                    };
                    let pid_for_del = pid.clone();
                    let pid_for_dl_track = pid.clone();
                    rsx! {
                        PlaylistDetail {
                            playlist_id: pid,
                            config,
                            on_close: move |_| selected_playlist_id.set(None),
                            is_downloading_all,
                            on_download_all: move |_| {
                                let requests: Vec<(String, String, String)> = {
                                    let store = playlists_res.read().clone().unwrap_or_default();
                                    let resolved = sel_server_tracks_res.read().clone().unwrap_or_default();
                                    store
                                        .playlists
                                        .iter()
                                        .find(|p| p.id == pid_for_dl)
                                        .map(|p| {
                                            p.tracks
                                                .iter()
                                                .map(|tid| {
                                                    let meta = resolved
                                                        .iter()
                                                        .find(|t| t.id.key().as_ref() == tid.as_str());
                                                    (
                                                        tid.clone(),
                                                        meta.map(|t| t.title.clone()).unwrap_or_default(),
                                                        meta.map(|t| t.artist.clone()).unwrap_or_default(),
                                                    )
                                                })
                                                .collect()
                                        })
                                        .unwrap_or_default()
                                };
                                if requests.is_empty() {
                                    return;
                                }
                                queue_downloads(requests, config, download_queue);
                            },
                            on_delete_all: move |_| {
                                let ids: Vec<String> = {
                                    let store = playlists_res.read().clone().unwrap_or_default();
                                    store
                                        .playlists
                                        .iter()
                                        .find(|p| p.id == pid_for_del)
                                        .map(|p| p.tracks.clone())
                                        .unwrap_or_default()
                                };
                                if !ids.is_empty() {
                                    delete_downloads(ids, config, download_queue);
                                }
                            },
                            on_download_track: move |idx: usize| {
                                let store = playlists_res.read().clone().unwrap_or_default();
                                let resolved = sel_server_tracks_res.read().clone().unwrap_or_default();
                                let mut track_id = String::new();
                                let mut track_title = String::new();
                                let mut track_artist = String::new();
                                if let Some(p) = store.playlists.iter().find(|p| p.id == pid_for_dl_track)
                                    && let Some(tid) = p.tracks.get(idx)
                                {
                                    track_id = tid.clone();
                                    if let Some(meta) =
                                        resolved.iter().find(|t| t.id.key().as_ref() == tid.as_str())
                                    {
                                        track_title = meta.title.clone();
                                        track_artist = meta.artist.clone();
                                    }
                                }
                                if !track_id.is_empty() {
                                    let is_downloaded = config
                                        .read()
                                        .offline_tracks
                                        .get(&track_id)
                                        .map(|p| std::path::Path::new(p).exists())
                                        .unwrap_or(false);
                                    if is_downloaded {
                                        delete_downloads(vec![track_id], config, download_queue);
                                    } else {
                                        queue_downloads(
                                            vec![(track_id, track_title, track_artist)],
                                            config,
                                            download_queue,
                                        );
                                    }
                                }
                            },
                        }
                    }
                }
            } else {
                div { class: if is_modern { "flex items-center justify-between mb-6" } else { "flex items-center justify-between mb-8" },
                    if is_modern {
                        div {
                            p {
                                class: "text-[10px] font-bold tracking-widest uppercase mb-0.5",
                                style: "color: rgba(255,255,255,0.35);",
                                "{i18n::t(\"library\")}"
                            }
                            h1 { class: "text-2xl font-bold text-white", "{i18n::t(\"playlists\")}" }
                        }
                    } else {
                        h1 { class: "text-3xl font-bold text-white", "{i18n::t(\"playlists\")}" }
                    }
                    div { class: "flex items-center gap-1",
                        if caps().folders {
                            button {
                                class: "text-white/60 flex items-center hover:text-white transition-colors p-3 rounded-full hover:bg-white/10",
                                title: i18n::t("new_folder").to_string(),
                                onclick: move |_| {
                                    let new_id = uuid::Uuid::new_v4().to_string();
                                    let name = i18n::t("new_folder").to_string();
                                    let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                                    spawn(async move {
                                        if local
                                            .create_folder(&new_id, &name)
                                            .await
                                            .is_ok()
                                        {
                                            gens.bump(Table::Folders);
                                        }
                                    });
                                },
                                i { class: "fa-solid fa-folder-plus" }
                            }
                        }
                        button {
                            class: "text-white/60 flex items-center hover:text-white transition-colors p-3 rounded-full hover:bg-white/10",
                            title: i18n::t("add_playlist").to_string(),
                            aria_label: i18n::t("add_playlist").to_string(),
                            onclick: move |_| {
                                error.set(None);
                                show_add_playlist.set(true);
                            },
                            i { class: "fa-solid fa-add" }
                        }
                    }
                }
                if show_add_playlist() {
                    AddPlaylistPopup {
                        playlist_name,
                        error,
                        on_close: move |_| {
                            error.set(None);
                            show_add_playlist.set(false);
                        },
                        on_save: handle_add_playlist,
                        show_add_folder: caps().folders,
                        on_add_folder: move |folder_path: String| {
                            let folder_path_buf = std::path::PathBuf::from(&folder_path);
                            let folder_name = folder_path_buf
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_else(|| folder_path.clone());
                            let prefix = if folder_path.ends_with(std::path::MAIN_SEPARATOR) {
                                folder_path
                            } else {
                                format!("{folder_path}{}", std::path::MAIN_SEPARATOR)
                            };
                            let read_db = consume_context::<hooks::ReadDb>();
                            let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                            spawn(async move {
                                let tracks = read_db.folder_tracks(&prefix).await.unwrap_or_default();
                                let refs: Vec<String> = tracks
                                    .iter()
                                    .map(|track| track.id.key().into_owned())
                                    .collect();
                                if local.create_playlist(&folder_name, &refs).await.is_ok() {
                                    gens.bump(Table::Playlists);
                                }
                            });
                            error.set(None);
                            playlist_name.set(String::new());
                        },
                    }
                }

                PlaylistsGrid {
                    config,
                    selected_playlist_id,
                    refresh_trigger: playlist_refresh_trigger,
                }
            }
        }
    }
}

/// The playlists grid: folders + local management when the source organises into
/// folders, else a flat remote list with downloads + remote sync. One component,
/// gated on [`Capabilities`].
#[component]
fn PlaylistsGrid(
    config: Signal<AppConfig>,
    mut selected_playlist_id: Signal<Option<String>>,
    refresh_trigger: Signal<u64>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let source = use_active_source();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let caps = use_memo(move || active_source.read().capabilities());
    let is_offline = use_context::<Signal<bool>>();
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let playlists_res = use_playlists();
    // First track of each playlist — the cover-of-last-resort for a playlist with
    // no explicit cover / image tag (resolved through the source cover seam).
    let first_keys = use_memo(move || {
        playlists_res
            .read()
            .clone()
            .unwrap_or_default()
            .playlists
            .iter()
            .filter_map(|p| p.tracks.first().cloned())
            .collect::<Vec<String>>()
    });
    let first_tracks_res = use_tracks_by_keys(source, first_keys);

    // Local folder-management state (mutated inside `folders_layout`'s handlers).
    let active_menu = use_signal(|| Option::<String>::None);
    let open_folder_id = use_signal(|| Option::<String>::None);
    let move_target_id = use_signal(|| Option::<String>::None);
    let rename_playlist_id = use_signal(|| Option::<String>::None);
    let rename_playlist_name = use_signal(String::new);
    let rename_folder_id = use_signal(|| Option::<String>::None);
    let rename_folder_name = use_signal(String::new);

    // Remote-sync state.
    let mut last_fetch_key = use_signal(|| None::<String>);
    let mut fetch_request_id = use_signal(|| 0u64);
    let mut yt_refresh_nonce: Signal<u64> = use_signal(|| 0);
    let mut yt_is_syncing = use_signal(|| false);
    let mut yt_synced_so_far: Signal<usize> = use_signal(|| 0);

    let active_server_id =
        use_memo(move || source().server_id().map(String::from).unwrap_or_default());

    // Remote playlist fetch — servers only (gated on `sync`). Diffs into the DB;
    // the grid reads the DB via `use_playlists`.
    use_effect(move || {
        if !caps().sync {
            return;
        }
        let yt_nonce = *yt_refresh_nonce.read();
        let trigger = *refresh_trigger.read();
        // YT auto-syncs only once (a stamp guards re-runs); other servers re-fetch
        // on a server/identity change. `discover` marks the active source as YT.
        let is_ytmusic = caps().discover;

        // Dedup key from the active server's identity (+ trigger), so a re-render
        // with the same server doesn't re-fetch.
        let (server_key, fetch_key) = {
            let conf = config.peek();
            match conf.server.as_ref() {
                Some(s) => {
                    let sk = format!(
                        "{:?}|{}|{}",
                        s.service,
                        s.url,
                        s.user_id.as_deref().unwrap_or_default()
                    );
                    let fk = format!(
                        "{sk}|{}|{trigger}",
                        s.access_token.as_deref().unwrap_or_default()
                    );
                    (Some(sk), Some(fk))
                }
                None => (None, None),
            }
        };

        let source = active_source.peek().clone();
        let read_db = consume_context::<hooks::ReadDb>();
        let sid = active_server_id();
        spawn(
            async move {
                if is_ytmusic && yt_nonce == 0 && trigger == 0 {
                    let already_synced = read_db
                        .meta_get("yt_sync", "timestamps")
                        .await
                        .ok()
                        .flatten()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .and_then(|v| v.get("last_yt_playlists_sync_at").and_then(|v| v.as_u64()))
                        .is_some();
                    if already_synced {
                        return;
                    }
                }

                let source_db = Source::Server(sid.clone());
                let existing = read_db
                    .load_playlists(&source_db)
                    .await
                    .unwrap_or_default()
                    .playlists;
                let has_cached = !existing.is_empty();
                let last_key = last_fetch_key.peek().clone();
                let last_server_key = last_key.as_ref().and_then(|k| {
                    let parts: Vec<&str> = k.splitn(5, '|').collect();
                    if parts.len() >= 3 {
                        Some(parts[..3].join("|").to_string())
                    } else {
                        None
                    }
                });
                if last_key.as_ref() == fetch_key.as_ref() {
                    return;
                }
                if server_key == last_server_key && has_cached && trigger == 0 {
                    last_fetch_key.set(fetch_key.clone());
                    return;
                }
                last_fetch_key.set(fetch_key.clone());

                let request_id = *fetch_request_id.peek() + 1;
                fetch_request_id.set(request_id);

                yt_is_syncing.set(true);
                yt_synced_so_far.set(0);

                // Listing first (tiles appear immediately), entries per playlist
                // after — all through the facade, so this loop is service-agnostic.
                let metas = match source.fetch_playlists().await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!(error = %e, "fetch_playlists failed");
                        yt_is_syncing.set(false);
                        return;
                    }
                };
                if *fetch_request_id.peek() != request_id {
                    return;
                }
                let total = metas.len();
                for m in &metas {
                    let existing_cover = existing
                        .iter()
                        .find(|e| e.id == m.id)
                        .and_then(|e| e.cover_path.clone())
                        .map(|p| p.to_string_lossy().into_owned());
                    let _ = source
                        .upsert_playlist_meta(
                            &m.id,
                            &m.name,
                            existing_cover.as_deref(),
                            m.image_tag.as_deref(),
                        )
                        .await;
                }
                gens.bump(Table::Playlists);

                let mut seen_paths: std::collections::HashSet<reader::TrackId> =
                    std::collections::HashSet::new();
                for (i, m) in metas.iter().enumerate() {
                    if *fetch_request_id.peek() != request_id {
                        return;
                    }
                    yt_synced_so_far.set(i + 1);
                    let entries = source
                        .fetch_playlist_entries(&m.id)
                        .await
                        .unwrap_or_default();
                    let track_ids: Vec<String> = entries
                        .iter()
                        .filter_map(|t| {
                            let k = t.id.key();
                            (!k.is_empty()).then(|| k.to_string())
                        })
                        .collect();
                    if source.set_playlist_tracks(&m.id, &track_ids).await.is_ok() {
                        gens.bump_coalesced(Table::Playlists);
                    }
                    let new_tracks: Vec<reader::models::Track> = entries
                        .into_iter()
                        .filter(|t| seen_paths.insert(t.id.clone()))
                        .collect();
                    for chunk in new_tracks.chunks(100) {
                        let _ = source.upsert_tracks(chunk).await;
                    }
                    gens.bump_coalesced(Table::Tracks);
                }

                if *fetch_request_id.peek() != request_id {
                    return;
                }
                // Full-replace: drop playlists no longer present remotely.
                for stale in existing
                    .iter()
                    .filter(|e| !metas.iter().any(|m| m.id == e.id))
                {
                    let _ = source.delete_playlist(&stale.id).await;
                }
                if is_ytmusic {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let mut stamps: serde_json::Value = read_db
                        .meta_get("yt_sync", "timestamps")
                        .await
                        .ok()
                        .flatten()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_else(|| serde_json::json!({}));
                    stamps["last_yt_playlists_sync_at"] = serde_json::json!(now);
                    let _ = source
                        .set_meta("yt_sync", "timestamps", &stamps.to_string())
                        .await;
                }
                gens.bump(Table::Tracks);
                gens.bump(Table::Playlists);
                yt_is_syncing.set(false);
                yt_synced_so_far.set(total);
            }
            .instrument(tracing::info_span!("playlists.fetch")),
        );
    });

    let store = playlists_res.read().clone().unwrap_or_default();
    let first_tracks = first_tracks_res.read().clone().unwrap_or_default();

    // A playlist's cover, source-uniform: an explicit cover, then a server image
    // tag, then the first track's cover (all resolved through the source layer).
    let cover_for = |playlist: &reader::models::Playlist| -> Option<utils::CoverUrl> {
        let conf = config.read();
        if let Some(url) = ::server::cover::from_path(&conf, playlist.cover_path.as_deref(), 384) {
            return Some(url);
        }
        if let Some(tag) = &playlist.image_tag
            && let Some(server) = &conf.server
        {
            return utils::map_cover_url(Some(utils::jellyfin_image::jellyfin_image_url(
                &server.url,
                &playlist.id,
                Some(tag.as_str()),
                server.access_token.as_deref(),
                384,
                80,
            )));
        }
        let first_ref = playlist.tracks.first()?;
        let track = first_tracks
            .iter()
            .find(|t| t.id.key().as_ref() == first_ref.as_str())?;
        ::server::cover::track(&conf, track, 384)
    };

    if caps().folders {
        return folders_layout(FoldersCtx {
            selected_playlist_id,
            store,
            cover_for: &cover_for,
            active_menu,
            open_folder_id,
            move_target_id,
            rename_playlist_id,
            rename_playlist_name,
            rename_folder_id,
            rename_folder_name,
            gens,
        });
    }

    // ---- Server (flat remote list) layout ----------------------------------
    let offline = caps().downloads && *is_offline.read();
    let conf = config.read();
    let playlists: Vec<reader::models::Playlist> = if offline {
        store
            .playlists
            .iter()
            .filter(|p| {
                !p.tracks.is_empty()
                    && p.tracks.iter().all(|tid| {
                        conf.offline_tracks
                            .get(tid)
                            .map(|path| std::path::Path::new(path).exists())
                            .unwrap_or(false)
                    })
            })
            .cloned()
            .collect()
    } else {
        store.playlists.clone()
    };
    drop(conf);
    let is_yt = caps().discover;
    let yt_anon = config
        .read()
        .server
        .as_ref()
        .map(|s| s.service == MusicService::YtMusic && s.yt_anonymous)
        .unwrap_or(false);

    rsx! {
        div {
            if is_yt {
                {
                    let syncing = *yt_is_syncing.read();
                    let done = *yt_synced_so_far.read();
                    let total = playlists.len();
                    let remaining = total.saturating_sub(done);
                    rsx! {
                        div { class: "flex items-center justify-between gap-3 mb-3 px-2 text-xs text-slate-400",
                            div { class: "flex items-center gap-2",
                                if syncing {
                                    i { class: "fa-solid fa-arrows-rotate fa-spin text-indigo-300" }
                                    span { "Loading tracks — {done} / {total} playlists ({remaining} left)" }
                                } else if total > 0 {
                                    i { class: "fa-solid fa-check text-emerald-400" }
                                    span { "{total} playlists synced" }
                                }
                            }
                            button {
                                class: "px-3 py-1 rounded bg-white/5 hover:bg-white/10 text-white/80 transition-colors disabled:opacity-50",
                                disabled: syncing,
                                onclick: move |_| {
                                    let next = *yt_refresh_nonce.peek() + 1;
                                    yt_refresh_nonce.set(next);
                                },
                                i { class: "fa-solid fa-arrows-rotate mr-1" }
                                "Refresh"
                            }
                        }
                    }
                }
            }

            if playlists.is_empty() {
                div { class: "flex flex-col items-center justify-center h-64 text-slate-500 text-center px-6",
                    if yt_anon {
                        i { class: "fa-solid fa-right-to-bracket text-4xl mb-4 opacity-50" }
                        p { "{i18n::t(\"yt_anon_playlists\")}" }
                    } else {
                        i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                        p { "{i18n::t(\"no_playlists_found\")}" }
                    }
                }
            } else {
                div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                    {playlists.into_iter().map(|playlist| {
                        let cover_url = cover_for(&playlist);
                        let playlist_id_nav = playlist.id.clone();
                        let is_dl = {
                            let q = download_queue.read();
                            playlist.tracks.iter().any(|tid| q.items.iter().any(|i| &i.id == tid && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading)))
                        };
                        let all_downloaded = !playlist.tracks.is_empty() && playlist.tracks.iter().all(|tid| {
                            config.read().offline_tracks.get(tid).map(|p| std::path::Path::new(p).exists()).unwrap_or(false)
                        });
                        rsx! {
                            div {
                                key: "{playlist.id}",
                                class: "bg-white/5 border border-white/5 rounded-2xl p-6 hover:bg-white/10 transition-all cursor-pointer group relative",
                                onclick: move |_| selected_playlist_id.set(Some(playlist_id_nav.clone())),
                                div { class: "mb-4 w-full aspect-square rounded-xl flex items-center justify-center overflow-hidden transition-all bg-white/5",
                                    if let Some(url) = cover_url {
                                        img { src: "{url}", class: "w-full h-full object-cover", decoding: "async", loading: "lazy" }
                                    } else {
                                        div {
                                            class: "w-full h-full flex items-center justify-center",
                                            style: "background: color-mix(in srgb, var(--color-indigo-500), transparent 80%); color: var(--color-indigo-400)",
                                            i { class: "fa-solid fa-server text-2xl" }
                                        }
                                    }
                                }
                                h3 { class: "text-xl font-bold text-white mb-1 truncate", "{playlist.name}" }
                                p { class: "text-sm text-slate-400", "Server • {playlist.tracks.len()} tracks" }
                                if caps().downloads {
                                    button {
                                        class: "absolute top-4 right-4 w-8 h-8 rounded-full bg-black/40 border border-white/10 flex items-center justify-center text-white/60 hover:text-white hover:border-white/30 transition-colors opacity-0 group-hover:opacity-100",
                                        title: if all_downloaded { "Remove downloads" } else { "Download playlist for offline playback" },
                                        disabled: is_dl,
                                        onclick: move |evt| {
                                            evt.stop_propagation();
                                            if all_downloaded {
                                                delete_downloads(playlist.tracks.clone(), config, download_queue);
                                            } else {
                                                let ids = playlist.tracks.clone();
                                                let s = source.peek().clone();
                                                let read_db = consume_context::<hooks::ReadDb>();
                                                spawn(async move {
                                                    let meta = read_db.tracks_by_keys(&s, &ids).await.unwrap_or_default();
                                                    let requests: Vec<(String, String, String)> = ids.iter().map(|tid| {
                                                        let m = meta.iter().find(|t| t.id.key().as_ref() == tid.as_str());
                                                        (tid.clone(), m.map(|t| t.title.clone()).unwrap_or_default(), m.map(|t| t.artist.clone()).unwrap_or_default())
                                                    }).collect();
                                                    queue_downloads(requests, config, download_queue);
                                                });
                                            }
                                        },
                                        if is_dl {
                                            i { class: "fa-solid fa-spinner fa-spin text-xs" }
                                        } else if all_downloaded {
                                            i { class: "fa-solid fa-trash text-xs" }
                                        } else {
                                            i { class: "fa-solid fa-download text-xs" }
                                        }
                                    }
                                }
                            }
                        }
                    })}
                }
            }
        }
    }
}

/// Borrowed bundle for the folder-tree layout (keeps the function signature sane).
struct FoldersCtx<'a> {
    selected_playlist_id: Signal<Option<String>>,
    store: reader::PlaylistStore,
    cover_for: &'a dyn Fn(&reader::models::Playlist) -> Option<utils::CoverUrl>,
    active_menu: Signal<Option<String>>,
    open_folder_id: Signal<Option<String>>,
    move_target_id: Signal<Option<String>>,
    rename_playlist_id: Signal<Option<String>>,
    rename_playlist_name: Signal<String>,
    rename_folder_id: Signal<Option<String>>,
    rename_folder_name: Signal<String>,
    gens: hooks::db_reactivity::Generations,
}

fn folders_layout(ctx: FoldersCtx<'_>) -> Element {
    let FoldersCtx {
        mut selected_playlist_id,
        store,
        cover_for,
        mut active_menu,
        mut open_folder_id,
        mut move_target_id,
        mut rename_playlist_id,
        mut rename_playlist_name,
        mut rename_folder_id,
        mut rename_folder_name,
        gens,
    } = ctx;

    let folders = store.folders.clone();
    let all_playlists = store.playlists.clone();
    // A dedicated clone the rename modal's `on_save` closure can own (it preserves
    // the playlist's existing cover when renaming).
    let rename_lookup = all_playlists.clone();
    let root_playlists: Vec<_> = all_playlists
        .iter()
        .filter(|p| !folders.iter().any(|f| f.playlist_ids.contains(&p.id)))
        .cloned()
        .collect();
    let open_folder = open_folder_id
        .read()
        .as_ref()
        .and_then(|id| folders.iter().find(|f| f.id == *id).cloned());
    let folder_playlists: Vec<_> = if let Some(ref folder) = open_folder {
        folder
            .playlist_ids
            .iter()
            .filter_map(|pid| all_playlists.iter().find(|p| p.id == *pid).cloned())
            .collect()
    } else {
        vec![]
    };

    let delete_playlist_text = i18n::t("delete_playlist").to_string();
    let rename_playlist_text = i18n::t("rename_playlist").to_string();
    let rename_folder_text = i18n::t("rename_folder").to_string();
    let move_text = i18n::t("move_to_folder").to_string();
    let remove_folder_text = i18n::t("remove_from_folder").to_string();
    let delete_folder_text = i18n::t("delete_folder").to_string();

    let playlist_actions = vec![
        MenuAction::new(move_text.as_str(), "fa-solid fa-folder-open"),
        MenuAction::new(rename_playlist_text.as_str(), "fa-solid fa-pen"),
        MenuAction::new(delete_playlist_text.as_str(), "fa-solid fa-trash").destructive(),
    ];
    let folder_playlist_actions = vec![
        MenuAction::new(move_text.as_str(), "fa-solid fa-folder-open"),
        MenuAction::new(remove_folder_text.as_str(), "fa-solid fa-folder-minus"),
        MenuAction::new(rename_playlist_text.as_str(), "fa-solid fa-pen"),
        MenuAction::new(delete_playlist_text.as_str(), "fa-solid fa-trash").destructive(),
    ];
    let folder_actions = vec![
        MenuAction::new(rename_folder_text.as_str(), "fa-solid fa-pen"),
        MenuAction::new(delete_folder_text.as_str(), "fa-solid fa-trash").destructive(),
    ];

    let render_card = |playlist: &reader::models::Playlist, in_folder: bool| {
        let cover_url = cover_for(playlist);
        let pid = playlist.id.clone();
        let pid_click = playlist.id.clone();
        let pid_menu = playlist.id.clone();
        let pid_action = playlist.id.clone();
        let name_for_rename = playlist.name.clone();
        let name = playlist.name.clone();
        let count = playlist.tracks.len();
        let is_menu_open = active_menu.read().as_deref() == Some(playlist.id.as_str());
        let actions = if in_folder {
            folder_playlist_actions.clone()
        } else {
            playlist_actions.clone()
        };
        rsx! {
            div {
                key: "{pid}",
                class: "bg-white/5 border border-white/5 rounded-2xl p-4 hover:bg-white/10 transition-all cursor-pointer group relative",
                onclick: move |_| selected_playlist_id.set(Some(pid_click.clone())),
                div { class: "mb-4 w-full h-32 rounded-xl flex items-center justify-center overflow-hidden transition-all bg-white/5",
                    if let Some(url) = cover_url {
                        img {
                            src: "{url}",
                            class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500",
                            decoding: "async",
                            loading: "lazy",
                        }
                    } else {
                        div {
                            class: "w-full h-full flex items-center justify-center",
                            style: "background: color-mix(in srgb, var(--color-indigo-500), transparent 80%); color: var(--color-indigo-400)",
                            i { class: "fa-solid fa-list-ul text-2xl" }
                        }
                    }
                }
                div { class: "flex items-start justify-between gap-2",
                    div { class: "min-w-0 flex-1",
                        h3 { class: "text-xl font-bold text-white mb-1 truncate", "{name}" }
                        {
                            let track_text = i18n::t_with("playlist_track_count", &[("count", count.to_string())]);
                            rsx! { p { class: "text-sm text-slate-400", "{track_text}" } }
                        }
                    }
                    div { onclick: move |evt| evt.stop_propagation(),
                        DotsMenu {
                            actions,
                            is_open: is_menu_open,
                            on_open: move |_| active_menu.set(Some(pid_menu.clone())),
                            on_close: move |_| active_menu.set(None),
                            button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                            anchor: "right".to_string(),
                            on_action: move |idx: usize| {
                                active_menu.set(None);
                                if in_folder {
                                    match idx {
                                        0 => move_target_id.set(Some(pid_action.clone())),
                                        1 => {
                                            let pid = pid_action.clone();
                                            let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                                            spawn(async move {
                                                if local
                                                    .set_playlist_folder(&pid, None)
                                                    .await
                                                    .is_ok()
                                                {
                                                    gens.bump(Table::Folders);
                                                }
                                            });
                                        }
                                        2 => {
                                            rename_playlist_id.set(Some(pid_action.clone()));
                                            rename_playlist_name.set(name_for_rename.clone());
                                        }
                                        _ => {
                                            let pid = pid_action.clone();
                                            let source = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                                            spawn(async move {
                                                if source.delete_playlist(&pid).await.is_ok()
                                                    && source.set_playlist_folder(&pid, None).await.is_ok()
                                                {
                                                    gens.bump(Table::Playlists);
                                                    gens.bump(Table::Folders);
                                                }
                                            });
                                        }
                                    }
                                } else {
                                    match idx {
                                        0 => move_target_id.set(Some(pid_action.clone())),
                                        1 => {
                                            rename_playlist_id.set(Some(pid_action.clone()));
                                            rename_playlist_name.set(name_for_rename.clone());
                                        }
                                        _ => {
                                            let pid = pid_action.clone();
                                            let s = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                                            spawn(async move {
                                                if s.delete_playlist(&pid).await.is_ok() {
                                                    gens.bump(Table::Playlists);
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
    };

    rsx! {
        div {
            if let Some(target_id) = move_target_id.read().clone() {
                FolderPickerModal {
                    playlist_id: target_id,
                    on_close: move |_| move_target_id.set(None),
                }
            }
            if let Some(rename_id) = rename_playlist_id.read().clone() {
                RenameTextModal {
                    title: rename_playlist_text.clone(),
                    value: rename_playlist_name,
                    on_close: move |_| {
                        rename_playlist_id.set(None);
                        rename_playlist_name.set(String::new());
                    },
                    on_save: move |_| {
                        let name = rename_playlist_name.read().trim().to_string();
                        if name.is_empty() {
                            return;
                        }
                        if let Some(playlist) = rename_lookup.iter().find(|playlist| playlist.id == rename_id) {
                            let id = rename_id.clone();
                            let cover = playlist
                                .cover_path
                                .as_ref()
                                .map(|p| p.to_string_lossy().into_owned());
                            let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                            spawn(async move {
                                if local
                                    .upsert_playlist_meta(&id, &name, cover.as_deref(), None)
                                    .await
                                    .is_ok()
                                {
                                    gens.bump(Table::Playlists);
                                }
                            });
                        }
                        rename_playlist_id.set(None);
                        rename_playlist_name.set(String::new());
                    },
                }
            }
            if let Some(rename_id) = rename_folder_id.read().clone() {
                RenameTextModal {
                    title: rename_folder_text.clone(),
                    value: rename_folder_name,
                    on_close: move |_| {
                        rename_folder_id.set(None);
                        rename_folder_name.set(String::new());
                    },
                    on_save: move |_| {
                        let name = rename_folder_name.read().trim().to_string();
                        if name.is_empty() {
                            return;
                        }
                        let rename_id = rename_id.clone();
                        let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                        spawn(async move {
                            if local
                                .rename_folder(&rename_id, &name)
                                .await
                                .is_ok()
                            {
                                gens.bump(Table::Folders);
                            }
                        });
                        rename_folder_id.set(None);
                        rename_folder_name.set(String::new());
                    },
                }
            }

            if let Some(ref folder) = open_folder {
                div {
                    div { class: "flex items-center gap-3 mb-8",
                        button {
                            class: "flex items-center gap-2 text-slate-400 hover:text-white transition-colors",
                            onclick: move |_| open_folder_id.set(None),
                            i { class: "fa-solid fa-arrow-left" }
                            "{i18n::t(\"back_to_playlists\")}"
                        }
                        span { class: "text-white/30", "/" }
                        span { class: "text-white font-semibold", "{folder.name}" }
                    }
                    if folder_playlists.is_empty() {
                        div { class: "flex flex-col items-center justify-center h-48 text-slate-500",
                            i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                            p { "{i18n::t(\"no_playlists_yet\")}" }
                        }
                    } else {
                        div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                            {folder_playlists.iter().map(|p| render_card(p, true))}
                        }
                    }
                }
            } else {
                div {
                    if folders.is_empty() && root_playlists.is_empty() {
                        div { class: "flex flex-col items-center justify-center h-64 text-slate-500",
                            i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                            p { "{i18n::t(\"no_playlists_yet\")}" }
                        }
                    } else {
                        if !folders.is_empty() {
                            div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6 mb-8",
                                {folders.iter().map(|folder| {
                                    let fid = folder.id.clone();
                                    let fid_open = folder.id.clone();
                                    let fid_menu = folder.id.clone();
                                    let fid_del = folder.id.clone();
                                    let fid_rename = folder.id.clone();
                                    let fname = folder.name.clone();
                                    let fname_rename = folder.name.clone();
                                    let count = folder.playlist_ids.len();
                                    let is_menu_open = active_menu.read().as_deref() == Some(folder.id.as_str());
                                    let cover_url = folder
                                        .playlist_ids
                                        .first()
                                        .and_then(|pid| all_playlists.iter().find(|p| p.id == *pid))
                                        .and_then(cover_for);
                                    let folder_actions = folder_actions.clone();
                                    rsx! {
                                        div {
                                            key: "{fid}",
                                            class: "bg-white/5 border border-white/5 rounded-2xl p-4 hover:bg-white/10 transition-all cursor-pointer group relative",
                                            onclick: move |_| open_folder_id.set(Some(fid_open.clone())),
                                            div { class: "mb-4 w-full h-32 rounded-xl flex items-center justify-center overflow-hidden transition-all bg-white/5",
                                                if let Some(url) = cover_url {
                                                    img {
                                                        src: "{url}",
                                                        class: "w-full h-full object-cover group-hover:scale-105 transition-transform duration-500",
                                                        decoding: "async",
                                                        loading: "lazy",
                                                    }
                                                } else {
                                                    div {
                                                        class: "w-full h-full flex items-center justify-center",
                                                        style: "background: color-mix(in srgb, var(--color-amber-500), transparent 80%); color: var(--color-amber-400)",
                                                        i { class: "fa-solid fa-folder text-2xl" }
                                                    }
                                                }
                                            }
                                            div { class: "flex items-start justify-between gap-2",
                                                div { class: "min-w-0 flex-1",
                                                    h3 { class: "text-xl font-bold text-white mb-1 truncate", "{fname}" }
                                                    p { class: "text-sm text-slate-400", "{count} playlists" }
                                                }
                                                div { onclick: move |evt| evt.stop_propagation(),
                                                    DotsMenu {
                                                        actions: folder_actions,
                                                        is_open: is_menu_open,
                                                        on_open: move |_| active_menu.set(Some(fid_menu.clone())),
                                                        on_close: move |_| active_menu.set(None),
                                                        button_class: "opacity-0 group-hover:opacity-100 focus:opacity-100".to_string(),
                                                        anchor: "right".to_string(),
                                                        on_action: move |idx: usize| {
                                                            active_menu.set(None);
                                                            if idx == 0 {
                                                                rename_folder_id.set(Some(fid_rename.clone()));
                                                                rename_folder_name.set(fname_rename.clone());
                                                            } else {
                                                                let fid = fid_del.clone();
                                                                let local = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                                                                spawn(async move {
                                                                    if local
                                                                        .delete_folder(&fid)
                                                                        .await
                                                                        .is_ok()
                                                                    {
                                                                        gens.bump(Table::Folders);
                                                                    }
                                                                });
                                                            }
                                                        },
                                                    }
                                                }
                                            }
                                        }
                                    }
                                })}
                            }
                        }
                        if !root_playlists.is_empty() {
                            if !folders.is_empty() {
                                h2 { class: "text-sm font-semibold text-white/40 uppercase tracking-widest mb-4",
                                    "{i18n::t(\"playlists\")}"
                                }
                            }
                            div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                                {root_playlists.iter().map(|p| render_card(p, false))}
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RenameTextModal(
    title: String,
    value: Signal<String>,
    on_close: EventHandler<()>,
    on_save: EventHandler<()>,
) -> Element {
    rsx! {
        div {
            class: "fixed inset-0 bg-black/70 flex items-center justify-center z-50",
            onclick: move |_| on_close.call(()),
            div {
                class: "bg-neutral-900 border border-white/10 rounded-2xl p-6 w-80 shadow-2xl",
                onclick: move |evt| evt.stop_propagation(),
                h2 { class: "text-lg font-bold text-white mb-4", "{title}" }
                input {
                    class: "w-full bg-white/5 border border-white/10 rounded-lg px-3 py-2 text-sm text-white placeholder-slate-500 focus:outline-none focus:border-indigo-500 mb-4",
                    value: "{value()}",
                    oninput: move |evt| value.set(evt.value()),
                    onkeydown: move |evt| {
                        evt.stop_propagation();
                        if evt.key() == Key::Enter {
                            on_save.call(());
                        }
                    },
                }
                div { class: "flex justify-end gap-2",
                    button {
                        class: "px-3 py-2 rounded-lg text-sm text-slate-400 hover:text-white hover:bg-white/10 transition-colors",
                        onclick: move |_| on_close.call(()),
                        "{i18n::t(\"cancel\")}"
                    }
                    button {
                        class: "px-3 py-2 bg-white text-black rounded-lg text-sm font-medium hover:bg-slate-200 transition-colors disabled:opacity-50 disabled:cursor-not-allowed",
                        disabled: value.read().trim().is_empty(),
                        onclick: move |_| on_save.call(()),
                        "{i18n::t(\"save\")}"
                    }
                }
            }
        }
    }
}
