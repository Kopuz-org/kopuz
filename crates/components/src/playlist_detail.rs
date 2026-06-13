use config::MusicService;
use db::Source;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_albums, use_playlists, use_tracks_by_keys};
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
use rfd::AsyncFileDialog;
use std::path::PathBuf;
use tracing::Instrument;

#[component]
#[tracing::instrument(name = "render.playlist_detail", skip_all)]
pub fn PlaylistDetail(
    playlist_id: String,
    config: Signal<config::AppConfig>,
    on_close: EventHandler<()>,
    on_download_all: Option<EventHandler<()>>,
    on_delete_all: Option<EventHandler<()>>,
    on_download_track: Option<EventHandler<usize>>,
    #[props(default = false)] is_downloading_all: bool,
) -> Element {
    let mut tracks = use_signal(Vec::<reader::models::Track>::new);
    let mut has_loaded_jellyfin_tracks = use_signal(|| false);
    let gens = hooks::db_reactivity::use_generations();
    let playlists_res = use_playlists();
    let source_local = use_memo(|| Source::Local);
    let albums_res = use_albums(source_local);

    let pid_for_seed = playlist_id.clone();
    let seed_refs = use_memo(move || {
        let store = playlists_res.read().clone().unwrap_or_default();
        // The store holds only the active source's playlists, so a found
        // playlist is a server one iff the active source is a server.
        let is_server = config.read().active_source.is_server();
        if let Some(p) = store.playlists.iter().find(|p| p.id == pid_for_seed) {
            (is_server, p.tracks.clone())
        } else {
            (false, Vec::new())
        }
    });
    let local_seed_refs = use_memo(move || {
        let (is_j, refs) = seed_refs.read().clone();
        if is_j { Vec::new() } else { refs }
    });
    let local_tracks_res = use_tracks_by_keys(source_local, local_seed_refs);

    // YT Music's InnerTube has no playlist-reorder mutation, so reorder
    // affordances need to be hidden / no-op on YT playlists. Doing the
    // optimistic local swap and then no-op'ing the server side leaves a
    // visual ghost that reverts on next sync — silent UX loss.
    let active_service_is_ytmusic = config
        .peek()
        .server
        .as_ref()
        .map(|s| s.service == MusicService::YtMusic)
        .unwrap_or(false);

    // Initial tracks WITHOUT any network round-trip: local playlists resolve
    // their refs through the tracks-by-keys hook; server playlists seed from
    // the cached server rows in the DB. For server playlists this is a SEED —
    // the network fetch below still runs once in the background and replaces
    // it. Previously the page sat empty until the whole remote walk finished
    // (837 liked songs = many sequential continuation requests).
    // One unconditional effect for both kinds, so the hook order can't change
    // if this component instance is re-rendered with the other playlist kind.
    use_effect(move || {
        let (is_j, refs) = seed_refs.read().clone();
        if !is_j {
            tracks.set(local_tracks_res.read().clone().unwrap_or_default());
        } else if !*has_loaded_jellyfin_tracks.read() && !refs.is_empty() {
            let db = consume_context::<db::Db>();
            let server_id = {
                let conf = config.peek();
                conf.active_source
                    .server_id()
                    .map(String::from)
                    .or_else(|| conf.server.as_ref().and_then(|s| s.id.clone()))
            };
            spawn(async move {
                let Some(sid) = server_id else { return };
                if let Ok(seed) = db.tracks_by_keys(&Source::Server(sid), &refs).await
                    && !seed.is_empty()
                    && !*has_loaded_jellyfin_tracks.peek()
                {
                    tracks.set(seed);
                }
            });
        }
    });

    let pid = playlist_id.clone();
    use_effect(move || {
        let is_j = seed_refs.read().0;
        if is_j && !*has_loaded_jellyfin_tracks.read() {
            let pid_clone = pid.clone();
            let load_span = tracing::info_span!("playlist.load_entries", playlist_id = %pid_clone);
            spawn(
                async move {
                    tracing::debug!("playlist entries load started");
                    let server_info = {
                        let conf = config.peek();
                        conf.server.as_ref().and_then(|server| {
                            if let (Some(token), Some(user_id)) =
                                (&server.access_token, &server.user_id)
                            {
                                Some((
                                    server.service,
                                    server.url.clone(),
                                    token.clone(),
                                    conf.device_id.clone(),
                                    user_id.clone(),
                                ))
                            } else {
                                None
                            }
                        })
                    };
                    if let Some((service, url, token, device_id, user_id)) = server_info {
                        match service {
                            MusicService::YtMusic => {
                                let yt = server::ytmusic::YouTubeMusicClient::with_cookies(
                                    token.clone(),
                                );
                                if let Ok(yt_tracks) = yt.get_playlist_entries(&pid_clone).await {
                                    tracing::debug!(
                                        count = yt_tracks.len(),
                                        "playlist entries loaded, setting tracks"
                                    );
                                    tracks.set(yt_tracks);
                                    has_loaded_jellyfin_tracks.set(true);
                                }
                            }
                            MusicService::Jellyfin => {
                                let remote = server::jellyfin::JellyfinClient::new(
                                    &url,
                                    Some(&token),
                                    &device_id,
                                    Some(&user_id),
                                );
                                if let Ok(items) = remote.get_playlist_items(&pid_clone).await {
                                    let mut new_tracks = Vec::new();
                                    for item in items {
                                        let duration_secs =
                                            item.run_time_ticks.unwrap_or(0) / 10_000_000;
                                        let cover = item
                                            .image_tags
                                            .as_ref()
                                            .and_then(|tags| tags.get("Primary").cloned());
                                        let bitrate_kbps = item.bitrate.unwrap_or(0) / 1000;
                                        let bitrate_u16 = bitrate_kbps.min(u16::MAX as u32) as u16;
                                        let artist_str = item
                                            .album_artist
                                            .clone()
                                            .or_else(|| item.artists.as_ref().map(|a| a.join(", ")))
                                            .unwrap_or_default();
                                        new_tracks.push(reader::models::Track {
                                            id: reader::models::TrackId::Server {
                                                service: MusicService::Jellyfin,
                                                item_id: item.id.clone(),
                                            },
                                            cover,
                                            album_id: item
                                                .album_id
                                                .map(|id| format!("jellyfin:{}", id))
                                                .unwrap_or_default(),
                                            title: item.name,
                                            artist: artist_str,
                                            album: item.album.unwrap_or_default(),
                                            duration: duration_secs,
                                            khz: item.sample_rate.unwrap_or(0),
                                            bitrate: bitrate_u16,
                                            track_number: item.index_number,
                                            disc_number: item.parent_index_number,
                                            musicbrainz_release_id: None,
                                            musicbrainz_recording_id: None,
                                            musicbrainz_track_id: None,
                                            playlist_item_id: item.playlist_item_id,
                                            artists: item.artists.unwrap_or_default(),
                                        });
                                    }
                                    tracing::debug!(
                                        count = new_tracks.len(),
                                        "playlist entries loaded, setting tracks"
                                    );
                                    tracks.set(new_tracks);
                                    has_loaded_jellyfin_tracks.set(true);
                                }
                            }
                            MusicService::Subsonic | MusicService::Custom => {
                                let remote =
                                    server::subsonic::SubsonicClient::new(&url, &user_id, &token);
                                if let Ok(items) = remote.get_playlist_entries(&pid_clone).await {
                                    let mut new_tracks = Vec::new();
                                    for item in items {
                                        let cover_tag = item
                                            .cover_art
                                            .as_ref()
                                            .and_then(|id| remote.cover_art_url(id, Some(512)).ok())
                                            .map(|url| {
                                                let mut hex = String::with_capacity(url.len() * 2);
                                                for b in url.as_bytes() {
                                                    hex.push_str(&format!("{:02x}", b));
                                                }
                                                format!("urlhex_{}", hex)
                                            });
                                        let album_id = item
                                            .album_id
                                            .as_ref()
                                            .map(|id| {
                                                if let Some(tag) = &cover_tag {
                                                    format!("jellyfin:{}:{}", id, tag)
                                                } else {
                                                    format!("jellyfin:{}:none", id)
                                                }
                                            })
                                            .unwrap_or_else(|| {
                                                format!("jellyfin:{}:none", item.id)
                                            });
                                        new_tracks.push(reader::models::Track {
                                            id: reader::models::TrackId::Server {
                                                service,
                                                item_id: item.id.clone(),
                                            },
                                            // "none" (not None) deliberately: it marks
                                            // an explicit no-cover so the resolver
                                            // doesn't fall through to a bogus remote
                                            // guess — same convention as subsonic_sync.
                                            cover: Some(
                                                cover_tag
                                                    .clone()
                                                    .unwrap_or_else(|| "none".to_string()),
                                            ),
                                            album_id,
                                            title: item.title,
                                            artist: item.artist.clone().unwrap_or_default(),
                                            album: item.album.unwrap_or_default(),
                                            duration: item.duration.unwrap_or(0),
                                            khz: item.sampling_rate.unwrap_or(0),
                                            bitrate: item.bit_rate.unwrap_or(0).min(u16::MAX as u32)
                                                as u16,
                                            track_number: item.track,
                                            disc_number: item.disc_number,
                                            musicbrainz_release_id: None,
                                            musicbrainz_recording_id: None,
                                            musicbrainz_track_id: None,
                                            playlist_item_id: None,
                                            artists: vec![item.artist.unwrap_or_default()],
                                        });
                                    }
                                    tracing::debug!(
                                        count = new_tracks.len(),
                                        "playlist entries loaded, setting tracks"
                                    );
                                    tracks.set(new_tracks);
                                    has_loaded_jellyfin_tracks.set(true);
                                }
                            }
                        }
                    }
                }
                .instrument(load_span),
            );
        }
    });

    let store_loading = playlists_res.read().is_none();
    let store = playlists_res.read().clone().unwrap_or_default();
    let (playlist_name, is_jellyfin, playlist_custom_cover, playlist_image_tag) =
        if let Some(p) = store.playlists.iter().find(|p| p.id == playlist_id) {
            let is_server = config.read().active_source.is_server();
            (
                p.name.clone(),
                is_server,
                p.cover_path.clone(),
                p.image_tag.clone(),
            )
        } else if store_loading {
            return rsx! { div {} };
        } else {
            return rsx! { div { "{i18n::t(\"playlist_not_found\")}" } };
        };

    let tracks_val = tracks.read().clone();

    let playlist_cover = if !is_jellyfin {
        playlist_custom_cover
            .as_ref()
            .and_then(|p| utils::format_artwork_url(Some(p)))
            .or_else(|| {
                tracks_val.first().and_then(|t| {
                    albums_res
                        .read()
                        .as_ref()
                        .and_then(|albums| albums.iter().find(|a| a.id == t.album_id))
                        .and_then(|a| utils::format_artwork_url(a.cover_path.as_ref()))
                })
            })
    } else if let Some(server) = &config.read().server {
        if let Some(path) = &playlist_custom_cover {
            utils::format_artwork_url(Some(path))
        } else if let Some(tag) = &playlist_image_tag {
            Some(std::sync::Arc::from(
                utils::jellyfin_image::jellyfin_image_url(
                    &server.url,
                    &playlist_id,
                    Some(tag.as_str()),
                    server.access_token.as_deref(),
                    512,
                    90,
                )
                .as_str(),
            ))
        } else {
            tracks_val.first().and_then(|t| {
                utils::jellyfin_image::resolve_track_cover(
                    t.cover.as_deref(),
                    &t.id.key(),
                    &t.album_id,
                    &server.url,
                    server.access_token.as_deref(),
                    512,
                    90,
                )
                .map(|s| std::sync::Arc::from(s.as_str()))
            })
        }
    } else {
        None
    };

    let pid_for_remove = playlist_id.clone();
    let pid_for_move_up = playlist_id.clone();
    let pid_for_move_down = playlist_id.clone();
    let pid_for_cover = playlist_id.clone();
    let name_for_cover = playlist_name.clone();
    let tag_for_cover = playlist_image_tag.clone();

    rsx! {
        crate::track_list_view::TrackListView {
            name: playlist_name.clone(),
            description: if is_jellyfin { i18n::t("server_playlist").to_string() } else { String::new() },
            cover_url: playlist_cover,
            back_label: i18n::t("back_to_playlists").to_string(),
            tracks: tracks_val,
            on_close,
            enable_metadata: !is_jellyfin,
            on_cover_click: move |_| {
                let _ = &pid_for_cover;
                let _ = &name_for_cover;
                let _ = &tag_for_cover;
                #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
                {
                    let pid = pid_for_cover.clone();
                    let pl_name = name_for_cover.clone();
                    let pl_tag = tag_for_cover.clone();
                    let db = consume_context::<db::Db>();
                    spawn(async move {
                        let file = AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await;
                        if let Some(file) = file {
                            let path = file.path().to_path_buf();
                            let src = if is_jellyfin {
                                // Pull the owned Jellyfin creds out of the config
                                // signal before awaiting, so no GenerationalRef is
                                // held across the upload.
                                let jellyfin = {
                                    let conf = config.peek();
                                    conf.server.as_ref().and_then(|server| {
                                        match (&server.access_token, &server.user_id) {
                                            (Some(token), Some(user_id))
                                                if server.service == MusicService::Jellyfin =>
                                            {
                                                Some((
                                                    server.url.clone(),
                                                    token.clone(),
                                                    user_id.clone(),
                                                    conf.device_id.clone(),
                                                ))
                                            }
                                            _ => None,
                                        }
                                    })
                                };
                                if let Some((url, token, user_id, device_id)) = jellyfin
                                    && let Ok(bytes) = std::fs::read(&path)
                                {
                                    let ext = path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("")
                                        .to_lowercase();
                                    let ct = if ext == "png" { "image/png" } else { "image/jpeg" };
                                    let remote = server::jellyfin::JellyfinClient::new(
                                        &url,
                                        Some(&token),
                                        &device_id,
                                        Some(&user_id),
                                    );
                                    let _ = remote.set_playlist_image(&pid, bytes, ct).await;
                                }
                                let sid = {
                                    let conf = config.peek();
                                    conf.active_source
                                        .server_id()
                                        .map(String::from)
                                        .or_else(|| conf.server.as_ref().and_then(|s| s.id.clone()))
                                        .unwrap_or_default()
                                };
                                Source::Server(sid)
                            } else {
                                Source::Local
                            };
                            let cover_str = path.to_string_lossy().into_owned();
                            if db
                                .upsert_playlist_meta(
                                    &src,
                                    &pid,
                                    &pl_name,
                                    Some(&cover_str),
                                    pl_tag.as_deref(),
                                )
                                .await
                                .is_ok()
                            {
                                gens.bump(Table::Playlists);
                            }
                        }
                    });
                }
            },
            on_delete_track: move |idx: usize| {
                if !is_jellyfin
                    && let Some(t) = tracks.read().get(idx).cloned() {
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Some(del_path) = t.id.local_path()
                            && std::fs::remove_file(del_path).is_ok()
                        {
                            let db = consume_context::<db::Db>();
                            let key = t.id.key().into_owned();
                            spawn(async move {
                                if db.delete_tracks(&Source::Local, &[key]).await.is_ok() {
                                    gens.bump(Table::Tracks);
                                }
                            });
                        }
                    }
            },
            on_selection_delete: move |paths: Vec<PathBuf>| {
                if !is_jellyfin {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
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
                    }
                }
            },
            on_remove_from_playlist: move |idx: usize| {
                if let Some(t) = tracks.read().get(idx).cloned() {
                    if !is_jellyfin {
                        let removed = t.id.uid_path().to_string_lossy().into_owned();
                        let pid = pid_for_remove.clone();
                        let db = consume_context::<db::Db>();
                        spawn(async move {
                            if db
                                .remove_playlist_tracks(&Source::Local, &pid, &[removed])
                                .await
                                .is_ok()
                            {
                                gens.bump(Table::Playlists);
                            }
                        });
                    } else {
                        let pid_clone = pid_for_remove.clone();
                        let entry_id_opt = t.playlist_item_id.clone();
                        let track_video_id = {
                            let k = t.id.key();
                            (!k.is_empty()).then(|| k.to_string())
                        };
                        let remove_idx = idx;
                        spawn(async move {
                            // Pull owned creds out of the config signal before any
                            // await so no GenerationalRef is held across the request.
                            let creds = {
                                let conf = config.peek();
                                conf.server.as_ref().and_then(|server| {
                                    match (&server.access_token, &server.user_id) {
                                        (Some(token), Some(user_id)) => Some((
                                            server.service,
                                            server.url.clone(),
                                            token.clone(),
                                            user_id.clone(),
                                            conf.device_id.clone(),
                                        )),
                                        _ => None,
                                    }
                                })
                            };
                            if let Some((service, url, token, user_id, device_id)) = creds {
                                let removed = match service {
                                    MusicService::YtMusic => {
                                        if let Some(vid) = track_video_id.as_deref() {
                                            let yt =
                                                server::ytmusic::YouTubeMusicClient::with_cookies(
                                                    token.clone(),
                                                );
                                            yt.remove_from_playlist(&pid_clone, vid).await.is_ok()
                                        } else {
                                            false
                                        }
                                    }
                                    MusicService::Jellyfin => {
                                        if let Some(entry_id) = entry_id_opt {
                                            let remote = server::jellyfin::JellyfinClient::new(
                                                &url,
                                                Some(&token),
                                                &device_id,
                                                Some(&user_id),
                                            );
                                            remote
                                                .remove_from_playlist(&pid_clone, &entry_id)
                                                .await
                                                .is_ok()
                                        } else {
                                            false
                                        }
                                    }
                                    MusicService::Subsonic | MusicService::Custom => {
                                        let remote = server::subsonic::SubsonicClient::new(
                                            &url, &user_id, &token,
                                        );
                                        remote
                                            .remove_from_playlist(&pid_clone, remove_idx)
                                            .await
                                            .is_ok()
                                    }
                                };
                                if removed {
                                    let mut tw = tracks.write();
                                    if remove_idx < tw.len() {
                                        tw.remove(remove_idx);
                                    }
                                }
                            }
                        });
                    }
                }
            },
            is_reorderable: !active_service_is_ytmusic,
            on_move_up: move |idx: usize| {
                if idx == 0 || active_service_is_ytmusic { return; }
                tracks.write().swap(idx - 1, idx);
                if !is_jellyfin {
                    let store = playlists_res.read().clone().unwrap_or_default();
                    if let Some(pl) =
                        store.playlists.iter().find(|p| p.id == pid_for_move_up)
                        && idx < pl.tracks.len()
                    {
                        let mut refs = pl.tracks.clone();
                        refs.swap(idx - 1, idx);
                        let pid = pid_for_move_up.clone();
                        let db = consume_context::<db::Db>();
                        spawn(async move {
                            if db
                                .set_playlist_tracks(&Source::Local, &pid, &refs)
                                .await
                                .is_ok()
                            {
                                gens.bump(Table::Playlists);
                            }
                        });
                    }
                } else {
                    let track_list = tracks.read().clone();
                    let pid = pid_for_move_up.clone();
                    spawn(async move {
                        let creds = {
                            let conf = config.peek();
                            conf.server.as_ref().and_then(|server| {
                                match (&server.access_token, &server.user_id) {
                                    (Some(token), Some(user_id)) => Some((
                                        server.service,
                                        server.url.clone(),
                                        token.clone(),
                                        user_id.clone(),
                                        conf.device_id.clone(),
                                    )),
                                    _ => None,
                                }
                            })
                        };
                        if let Some((service, url, token, user_id, device_id)) = creds {
                            let moved_item =
                                track_list.get(idx - 1).and_then(|t| t.playlist_item_id.clone());
                            match service {
                                MusicService::YtMusic => {}
                                MusicService::Jellyfin => {
                                    if let Some(item_id) = moved_item {
                                        let remote = server::jellyfin::JellyfinClient::new(
                                            &url,
                                            Some(&token),
                                            &device_id,
                                            Some(&user_id),
                                        );
                                        let _ = remote
                                            .move_playlist_item(&pid, &item_id, idx - 1)
                                            .await;
                                    }
                                }
                                MusicService::Subsonic | MusicService::Custom => {
                                    let remote =
                                        server::subsonic::SubsonicClient::new(&url, &user_id, &token);
                                    let ids: Vec<String> = track_list
                                        .iter()
                                        .filter_map(|t| {
                                            let s = t.id.uid();
                                            let parts: Vec<&str> = s.split(':').collect();
                                            if parts.len() >= 2 {
                                                Some(parts[1].to_string())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    let id_refs: Vec<&str> =
                                        ids.iter().map(|s| s.as_str()).collect();
                                    let _ = remote
                                        .reorder_playlist(&pid, &id_refs, id_refs.len())
                                        .await;
                                }
                            }
                        }
                    });
                }
            },
            on_move_down: move |idx: usize| {
                let len = tracks.read().len();
                if idx + 1 >= len || active_service_is_ytmusic { return; }
                tracks.write().swap(idx, idx + 1);
                if !is_jellyfin {
                    let store = playlists_res.read().clone().unwrap_or_default();
                    if let Some(pl) =
                        store.playlists.iter().find(|p| p.id == pid_for_move_down)
                        && idx + 1 < pl.tracks.len()
                    {
                        let mut refs = pl.tracks.clone();
                        refs.swap(idx, idx + 1);
                        let pid = pid_for_move_down.clone();
                        let db = consume_context::<db::Db>();
                        spawn(async move {
                            if db
                                .set_playlist_tracks(&Source::Local, &pid, &refs)
                                .await
                                .is_ok()
                            {
                                gens.bump(Table::Playlists);
                            }
                        });
                    }
                } else {
                    let track_list = tracks.read().clone();
                    let pid = pid_for_move_down.clone();
                    spawn(async move {
                        let creds = {
                            let conf = config.peek();
                            conf.server.as_ref().and_then(|server| {
                                match (&server.access_token, &server.user_id) {
                                    (Some(token), Some(user_id)) => Some((
                                        server.service,
                                        server.url.clone(),
                                        token.clone(),
                                        user_id.clone(),
                                        conf.device_id.clone(),
                                    )),
                                    _ => None,
                                }
                            })
                        };
                        if let Some((service, url, token, user_id, device_id)) = creds {
                            let moved_item =
                                track_list.get(idx + 1).and_then(|t| t.playlist_item_id.clone());
                            match service {
                                MusicService::YtMusic => {}
                                MusicService::Jellyfin => {
                                    if let Some(item_id) = moved_item {
                                        let remote = server::jellyfin::JellyfinClient::new(
                                            &url,
                                            Some(&token),
                                            &device_id,
                                            Some(&user_id),
                                        );
                                        let _ = remote
                                            .move_playlist_item(&pid, &item_id, idx + 1)
                                            .await;
                                    }
                                }
                                MusicService::Subsonic | MusicService::Custom => {
                                    let remote =
                                        server::subsonic::SubsonicClient::new(&url, &user_id, &token);
                                    let ids: Vec<String> = track_list
                                        .iter()
                                        .filter_map(|t| {
                                            let s = t.id.uid();
                                            let parts: Vec<&str> = s.split(':').collect();
                                            if parts.len() >= 2 {
                                                Some(parts[1].to_string())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    let id_refs: Vec<&str> =
                                        ids.iter().map(|s| s.as_str()).collect();
                                    let _ = remote
                                        .reorder_playlist(&pid, &id_refs, id_refs.len())
                                        .await;
                                }
                            }
                        }
                    });
                }
            },
            on_download_all: if is_jellyfin { on_download_all } else { None },
            on_download_track: if is_jellyfin { on_download_track } else { None },
            on_delete_all: if is_jellyfin { on_delete_all } else { None },
            is_downloading_all,
            show_delete_in_selection: !is_jellyfin,
        }
    }
}
