use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};
use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use config::{AppConfig, MusicService};
use db::Source;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_playlists, use_tracks_by_keys};
use tracing::Instrument;

#[component]
#[tracing::instrument(name = "render.jellyfin_playlists", skip_all)]
pub fn JellyfinPlaylists(
    config: Signal<AppConfig>,
    mut selected_playlist_id: Signal<Option<String>>,
    #[props(default)] refresh_trigger: Signal<u64>,
) -> Element {
    let is_offline = use_context::<Signal<bool>>();
    let mut last_fetch_key = use_signal(|| None::<String>);
    let mut fetch_request_id = use_signal(|| 0u64);
    let mut yt_refresh_nonce: Signal<u64> = use_signal(|| 0);
    let mut yt_is_syncing = use_signal(|| false);
    let mut yt_synced_so_far: Signal<usize> = use_signal(|| 0);
    let download_queue = use_context::<Signal<DownloadQueue>>();

    let gens = hooks::db_reactivity::use_generations();
    let active_server_id = use_memo(move || {
        let c = config.read();
        c.active_server_id
            .clone()
            .or_else(|| c.server.as_ref().and_then(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let playlists_server_id =
        use_memo(move || Some(active_server_id()).filter(|id| !id.is_empty()));
    let playlists_res = use_playlists(playlists_server_id);
    let server_source = use_memo(move || Source::Server(active_server_id()));
    let cover_keys = use_memo(move || -> Vec<String> {
        let store = playlists_res.read().clone().unwrap_or_default();
        store
            .jellyfin_playlists
            .iter()
            .filter_map(|p| p.tracks.first().cloned())
            .collect()
    });
    let cover_tracks_res = use_tracks_by_keys(server_source, cover_keys);

    use_effect(move || {
        let yt_nonce = *yt_refresh_nonce.read();
        let trigger = *refresh_trigger.read();

        let fetch_context = {
            let conf = config.peek();
            conf.server.as_ref().and_then(|server| {
                if let (Some(token), Some(user_id)) = (&server.access_token, &server.user_id) {
                    Some((
                        server.service,
                        server.url.clone(),
                        token.clone(),
                        user_id.clone(),
                        conf.device_id.clone(),
                    ))
                } else {
                    None
                }
            })
        };
        let is_ytmusic = fetch_context
            .as_ref()
            .map(|(s, _, _, _, _)| *s == MusicService::YtMusic)
            .unwrap_or(false);

        // Build a "server identity" key (without trigger) to detect server changes
        let server_key = fetch_context
            .as_ref()
            .map(|(service, url, _, user_id, _)| format!("{service:?}|{url}|{user_id}"));

        // Build the full fetch key that also includes the trigger
        let fetch_key = fetch_context
            .as_ref()
            .map(|(service, url, token, user_id, _)| {
                format!("{service:?}|{url}|{user_id}|{token}|{trigger}")
            });

        let db = consume_context::<db::Db>();
        let sid = active_server_id();
        spawn(async move {
            if is_ytmusic && yt_nonce == 0 && trigger == 0 {
                let already_synced = db
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

            let has_cached = !db
                .load_playlists(Some(sid.as_str()))
                .await
                .unwrap_or_default()
                .jellyfin_playlists
                .is_empty();
            let last_key = last_fetch_key.peek().clone();

            // Extract the server-identity part of the last fetch key (everything before the last |)
            let last_server_key = last_key.as_ref().and_then(|k| {
                let parts: Vec<&str> = k.splitn(5, '|').collect();
                if parts.len() >= 3 {
                    Some(parts[..3].join("|").to_string())
                } else {
                    None
                }
            });

            // Skip if same key (already fetched this exact state)
            if last_key.as_ref() == fetch_key.as_ref() {
                return;
            }

            // If server identity is the same and we have cached data, only re-fetch on explicit trigger
            if server_key == last_server_key && has_cached && trigger == 0 {
                // Update the key so we don't keep hitting this branch, but don't fetch
                last_fetch_key.set(fetch_key.clone());
                return;
            }

            last_fetch_key.set(fetch_key.clone());

            let request_id = *fetch_request_id.peek() + 1;
            fetch_request_id.set(request_id);

            let Some((service, url, token, user_id, device_id)) = fetch_context else {
                return;
            };
            let source = Source::Server(sid.clone());
            let mut server_playlists = Vec::new();

            match service {
                MusicService::Jellyfin => {
                    let remote =
                        JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                    if let Ok(playlists) = remote.get_playlists().await {
                        for p in playlists {
                            let image_tag = p
                                .image_tags
                                .as_ref()
                                .and_then(|tags| tags.get("Primary"))
                                .cloned();
                            if let Ok(items) = remote.get_playlist_items(&p.id).await {
                                let tracks: Vec<String> =
                                    items.into_iter().map(|item| item.id).collect();
                                server_playlists.push(reader::models::JellyfinPlaylist {
                                    id: p.id.clone(),
                                    name: p.name.clone(),
                                    tracks,
                                    image_tag,
                                    cover_path: None,
                                });
                            } else {
                                server_playlists.push(reader::models::JellyfinPlaylist {
                                    id: p.id.clone(),
                                    name: p.name.clone(),
                                    tracks: vec![],
                                    image_tag,
                                    cover_path: None,
                                });
                            }
                        }
                    }
                }
                MusicService::Subsonic | MusicService::Custom => {
                    let remote = SubsonicClient::new(&url, &user_id, &token);
                    if let Ok(playlists) = remote.get_playlists().await {
                        for p in playlists {
                            let tracks = remote
                                .get_playlist_entries(&p.id)
                                .await
                                .unwrap_or_default()
                                .into_iter()
                                .map(|song| song.id)
                                .collect();
                            server_playlists.push(reader::models::JellyfinPlaylist {
                                id: p.id,
                                name: p.name,
                                tracks,
                                image_tag: None,
                                cover_path: None,
                            });
                        }
                    }
                }
                MusicService::YtMusic => {
                    tracing::debug!("YT playlists sync starting");
                    yt_is_syncing.set(true);
                    yt_synced_so_far.set(0);
                    let yt =
                        ::server::ytmusic::YouTubeMusicClient::with_cookies(token.clone());
                    let list_result = yt.list_playlists().await;
                    if *fetch_request_id.read() != request_id {
                        return;
                    }
                    match list_result.as_ref() {
                        Ok(v) => tracing::debug!(entries = v.len(), "YT list_playlists"),
                        Err(e) => tracing::warn!(error = %e, "YT list_playlists failed"),
                    }
                    let summaries = match list_result {
                        Ok(s) => s,
                        Err(_) => {
                            yt_is_syncing.set(false);
                            return;
                        }
                    };
                    let total = summaries.len();
                    yt_synced_so_far.set(0);

                    {
                        let existing = db
                            .load_playlists(Some(sid.as_str()))
                            .await
                            .unwrap_or_default()
                            .jellyfin_playlists;
                        for s in &summaries {
                            let image_tag = s
                                .thumbnail_url
                                .as_ref()
                                .map(|u| utils::jellyfin_image::encode_cover_url(u));
                            let existing_cover_path = existing
                                .iter()
                                .find(|e| e.id == s.id)
                                .and_then(|e| e.cover_path.clone())
                                .map(|p| p.to_string_lossy().into_owned());
                            let _ = db
                                .upsert_playlist_meta(
                                    &source,
                                    &s.id,
                                    &s.title,
                                    existing_cover_path.as_deref(),
                                    image_tag.as_deref(),
                                )
                                .await;
                        }
                        gens.bump(Table::Playlists);
                    }

                    let mut seen_paths: std::collections::HashSet<std::path::PathBuf> =
                        std::collections::HashSet::new();
                    for (i, summary) in summaries.into_iter().enumerate() {
                        if *fetch_request_id.peek() != request_id {
                            return;
                        }
                        yt_synced_so_far.set(i + 1);
                        let tracks = match yt.get_playlist_entries(&summary.id).await {
                            Ok(t) => t,
                            Err(e) => {
                                tracing::warn!(playlist = %summary.id, error = %e, "YT playlist entries failed");
                                Vec::new()
                            }
                        };
                        let track_ids: Vec<String> = tracks
                            .iter()
                            .filter_map(|t| {
                                let k = t.id.key();
                                (!k.is_empty()).then(|| k.to_string())
                            })
                            .collect();
                        if db
                            .set_playlist_tracks(&source, &summary.id, &track_ids)
                            .await
                            .is_ok()
                        {
                            gens.bump_coalesced(Table::Playlists);
                        }
                        let new_tracks: Vec<reader::models::Track> = tracks
                            .into_iter()
                            .filter(|t| seen_paths.insert(t.id.uid_path()))
                            .collect();
                        for chunk in new_tracks.chunks(100) {
                            let _ = db.upsert_tracks(&source, chunk).await;
                        }
                        gens.bump_coalesced(Table::Tracks);
                    }

                    if *fetch_request_id.peek() != request_id {
                        return;
                    }
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
                    stamps["last_yt_playlists_sync_at"] = serde_json::json!(now);
                    let _ = db
                        .meta_put("yt_sync", "timestamps", &stamps.to_string())
                        .await;
                    gens.bump(Table::Tracks);
                    gens.bump(Table::Playlists);
                    yt_is_syncing.set(false);
                    yt_synced_so_far.set(total);
                    return;
                }
            }

            if *fetch_request_id.peek() != request_id {
                return;
            }

            let existing = db
                .load_playlists(Some(sid.as_str()))
                .await
                .unwrap_or_default()
                .jellyfin_playlists;
            // Preserve any locally-set cover_path when replacing server data
            for p in &mut server_playlists {
                if let Some(existing) = existing.iter().find(|e| e.id == p.id) {
                    p.cover_path = existing.cover_path.clone();
                }
            }
            for p in &server_playlists {
                let cover = p
                    .cover_path
                    .as_ref()
                    .map(|c| c.to_string_lossy().into_owned());
                if db
                    .upsert_playlist_meta(
                        &source,
                        &p.id,
                        &p.name,
                        cover.as_deref(),
                        p.image_tag.as_deref(),
                    )
                    .await
                    .is_ok()
                {
                    let _ = db.set_playlist_tracks(&source, &p.id, &p.tracks).await;
                }
            }
            // Full-replace semantics: drop server playlists that no longer exist.
            for stale in existing
                .iter()
                .filter(|e| !server_playlists.iter().any(|p| p.id == e.id))
            {
                let _ = db.delete_playlist(&source, &stale.id).await;
            }
            gens.bump(Table::Playlists);
        }.instrument(tracing::info_span!("playlists.fetch")));
    });

    let jellyfin_playlists = use_memo(move || {
        let store = playlists_res.read().clone().unwrap_or_default();
        let offline = *is_offline.read();
        let conf = config.read();
        if offline {
            store
                .jellyfin_playlists
                .iter()
                .filter(|p| {
                    !p.tracks.is_empty()
                        && p.tracks.iter().all(|tid| {
                            if let Some(path_str) = conf.offline_tracks.get(tid) {
                                std::path::Path::new(path_str).exists()
                            } else {
                                false
                            }
                        })
                })
                .cloned()
                .collect()
        } else {
            store.jellyfin_playlists.clone()
        }
    });

    let playlists = jellyfin_playlists.read().clone();
    let is_ytmusic_active = config
        .read()
        .server
        .as_ref()
        .map(|s| s.service == MusicService::YtMusic)
        .unwrap_or(false);

    rsx! {
        div {
            if is_ytmusic_active {
                {
                    let syncing = *yt_is_syncing.read();
                    let done = *yt_synced_so_far.read();
                    // Total is the number of tiles seeded into the
                    // store; while syncing this is the target count,
                    // after syncing it's the final count.
                    let total = playlists.len();
                    let remaining = total.saturating_sub(done);
                    rsx! {
                        div {
                            class: "flex items-center justify-between gap-3 mb-3 px-2 text-xs text-slate-400",
                            div {
                                class: "flex items-center gap-2",
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
                {
                    // Anonymous YT has no library playlists by design —
                    // show a sign-in prompt rather than the generic
                    // "no playlists found" empty state.
                    let yt_anon = config
                        .read()
                        .server
                        .as_ref()
                        .map(|s| {
                            s.service == config::MusicService::YtMusic && s.yt_anonymous
                        })
                        .unwrap_or(false);
                    rsx! {
                        div { class: "flex flex-col items-center justify-center h-64 text-slate-500 text-center px-6",
                            if yt_anon {
                                i { class: "fa-solid fa-right-to-bracket text-4xl mb-4 opacity-50" }
                                p { "{i18n::t(\"yt_anon_playlists\")}" }
                            } else {
                                i { class: "fa-regular fa-folder-open text-4xl mb-4 opacity-50" }
                                p { "{i18n::t(\"no_playlists_found\")}" }
                            }
                        }
                    }
                }
            } else {
                div { class: "grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-6",
                    {playlists.into_iter().map(|playlist| {
                        let cover_url = {
                            let conf = config.peek();
                            if let Some(server) = &conf.server {
                                if let Some(path) = &playlist.cover_path {
                                    utils::format_artwork_url(Some(path))
                                } else if let Some(tag) = &playlist.image_tag {
                                    utils::map_cover_url(Some(utils::jellyfin_image::jellyfin_image_url(
                                        &server.url,
                                        &playlist.id,
                                        Some(tag.as_str()),
                                        server.access_token.as_deref(),
                                        384,
                                        80,
                                    )))
                                } else if let Some(first_track_id) = playlist.tracks.first() {
                                    cover_tracks_res
                                        .read()
                                        .clone()
                                        .unwrap_or_default()
                                        .iter()
                                        .find(|t| t.id.key().contains(first_track_id.as_str()))
                                        .and_then(|t| {
                                            utils::map_cover_url(utils::jellyfin_image::resolve_track_cover(
                                                t.cover.as_deref(),
                                                &t.id.key(),
                                                &t.album_id,
                                                &server.url,
                                                server.access_token.as_deref(),
                                                384,
                                                80,
                                            ))
                                        })
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        let playlist_id_nav = playlist.id.clone();
                        let is_dl = {
                            let q = download_queue.read();
                            playlist.tracks.iter().any(|tid| q.items.iter().any(|i| &i.id == tid && matches!(i.status, DownloadStatus::Queued | DownloadStatus::Downloading)))
                        };

                        let all_downloaded = !playlist.tracks.is_empty() && playlist.tracks.iter().all(|tid| {
                            if let Some(path_str) = config.read().offline_tracks.get(tid) {
                                std::path::Path::new(path_str).exists()
                            } else {
                                false
                            }
                        });

                        rsx! {
                            div {
                                key: "{playlist.id}",
                                class: "bg-white/5 border border-white/5 rounded-2xl p-6 hover:bg-white/10 transition-all cursor-pointer group relative",
                                onclick: move |_| selected_playlist_id.set(Some(playlist_id_nav.clone())),
                                div {
                                    class: "mb-4 w-full aspect-square rounded-xl flex items-center justify-center overflow-hidden transition-all bg-white/5",
                                    if let Some(url) = cover_url {
                                        img {
                                            src: "{url}",
                                            class: "w-full h-full object-cover",
                                            decoding: "async", loading: "lazy"
                                        }
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

                                button {
                                    class: "absolute top-4 right-4 w-8 h-8 rounded-full bg-black/40 border border-white/10 flex items-center justify-center text-white/60 hover:text-white hover:border-white/30 transition-colors opacity-0 group-hover:opacity-100",
                                    title: if all_downloaded { "Remove downloads" } else { "Download playlist for offline playback" },
                                    disabled: is_dl,
                                    onclick: move |evt| {
                                        evt.stop_propagation();
                                        if all_downloaded {
                                            let ids_only = playlist.tracks.clone();
                                            crate::server::download_manager::delete_downloads(ids_only, config, download_queue);
                                        } else {
                                            let ids = playlist.tracks.clone();
                                            let s = server_source.peek().clone();
                                            let db = consume_context::<db::Db>();
                                            spawn(async move {
                                                let meta = db.tracks_by_keys(&s, &ids).await.unwrap_or_default();
                                                let requests: Vec<(String, String, String)> = ids
                                                    .iter()
                                                    .map(|tid| {
                                                        let m = meta.iter().find(|t| t.id.key().as_ref() == tid.as_str());
                                                        (
                                                            tid.clone(),
                                                            m.map(|t| t.title.clone()).unwrap_or_default(),
                                                            m.map(|t| t.artist.clone()).unwrap_or_default(),
                                                        )
                                                    })
                                                    .collect();
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
                    })}
                }
            }
        }
    }
}

pub use JellyfinPlaylists as ServerPlaylists;
