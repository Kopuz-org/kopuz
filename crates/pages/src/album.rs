use config::{AppConfig, MusicSource};
use db::Source;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_albums, use_all_tracks, use_tracks_window};

use crate::local::album::LocalAlbum;
use crate::server::album::{ServerAlbum, ServerAlbumDetails};

#[component]
pub fn Album(
    config: Signal<AppConfig>,
    album_id: Signal<String>,
    mut queue: Signal<Vec<reader::models::Track>>,
    mut current_queue_index: Signal<usize>,
) -> Element {
    let is_server = config.read().active_source == MusicSource::Server;

    let open_album_menu = use_signal(|| None::<String>);
    let mut show_album_playlist_modal = use_signal(|| false);
    let pending_album_id_for_playlist = use_signal(|| None::<String>);

    let mut has_fetched_jellyfin = use_signal(|| false);

    let gens = hooks::db_reactivity::use_generations();
    let active_server_id = use_memo(move || {
        let c = config.read();
        c.active_server_id
            .clone()
            .or_else(|| c.server.as_ref().and_then(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let server_source = use_memo(move || Source::Server(active_server_id()));
    let server_filter = use_memo(move || db::TrackFilter::new(Source::Server(active_server_id())));
    let probe_page = use_memo(|| db::Page {
        offset: 0,
        limit: 1,
    });
    let server_tracks_win = use_tracks_window(server_filter, probe_page);
    let server_albums_res = use_albums(server_source);
    let pending_filter = use_memo(move || {
        let aid = pending_album_id_for_playlist
            .read()
            .clone()
            .unwrap_or_default();
        let source = if config.read().active_source == MusicSource::Server {
            Source::Server(active_server_id())
        } else {
            Source::Local
        };
        db::TrackFilter::album(source, aid)
    });
    let pending_tracks_res = use_all_tracks(pending_filter);

    let mut fetch_jellyfin = move || {
        has_fetched_jellyfin.set(true);
        spawn(async move {
            let _ = crate::server::subsonic_sync::sync_server_library(config, false).await;
        });
    };

    use_effect(move || {
        if is_server && !*has_fetched_jellyfin.read() {
            if let (Some(total), Some(albums)) = (
                *server_tracks_win.total.read(),
                server_albums_res.read().clone(),
            ) {
                if total == 0 || albums.is_empty() {
                    fetch_jellyfin();
                } else {
                    has_fetched_jellyfin.set(true);
                }
            }
        }
    });

    rsx! {
        div {
            class: if cfg!(target_os = "android") { "px-4 pt-2 pb-28 absolute inset-0 flex flex-col" } else { "p-8 pb-24 absolute inset-0 flex flex-col" },

            if album_id.read().is_empty() {
                div {
                    if !cfg!(target_os = "android") {
                        h1 { class: "text-3xl font-bold text-white mb-6", "{i18n::t(\"all_albums\")}" }
                    }

                    if is_server {
                        ServerAlbum {
                            config,
                            album_id,
                            queue,
                            open_album_menu,
                            show_album_playlist_modal,
                            pending_album_id_for_playlist,
                        }
                    } else {
                        LocalAlbum {
                            album_id,
                            queue,
                            open_album_menu,
                            show_album_playlist_modal,
                            pending_album_id_for_playlist,
                        }
                    }

                    if *show_album_playlist_modal.read() {
                        components::playlist_modal::PlaylistModal {
                            is_jellyfin: is_server,
                            on_close: move |_| show_album_playlist_modal.set(false),
                            on_add_to_playlist: move |playlist_id: String| {
                                if pending_album_id_for_playlist.read().is_some() {
                                    let tracks: Vec<_> = pending_tracks_res
                                        .read()
                                        .clone()
                                        .unwrap_or_default()
                                        .iter()
                                        .map(|t| t.id.uid_path())
                                        .collect();
                                    if is_server {
                                        let pid = playlist_id.clone();
                                        let paths = tracks.clone();
                                        let server_vals = {
                                            let conf = config.peek();
                                            conf.server
                                                .as_ref()
                                                .and_then(|s| {
                                                    if let (Some(tok), Some(uid)) = (
                                                        &s.access_token,
                                                        &s.user_id,
                                                    ) {
                                                        Some((
                                                            s.service,
                                                            s.url.clone(),
                                                            tok.clone(),
                                                            uid.clone(),
                                                            conf.device_id.clone(),
                                                        ))
                                                    } else {
                                                        None
                                                    }
                                                })
                                        };
                                        if let Some((service, url, token, user_id, device_id)) = server_vals {
                                            let sid = active_server_id();
                                            spawn(async move {
                                                let conn = ::server::server_ops::ServerConn {
                                                    service,
                                                    url,
                                                    token,
                                                    user_id,
                                                    device_id,
                                                };
                                                let item_ids: Vec<String> = paths
                                                    .iter()
                                                    .filter_map(|p| {
                                                        ::server::server_ops::parse_item_id(p.to_str()?)
                                                            .map(str::to_string)
                                                    })
                                                    .collect();
                                                let added = ::server::server_ops::add_tracks_to_playlist(
                                                    &conn, &pid, &item_ids,
                                                )
                                                .await;
                                                if !added.is_empty() {
                                                    let db = consume_context::<db::Db>();
                                                    let store = db.load_playlists().await.unwrap_or_default();
                                                    if let Some(pl) = store
                                                        .jellyfin_playlists
                                                        .iter()
                                                        .find(|p| p.id == pid)
                                                    {
                                                        let mut refs = pl.tracks.clone();
                                                        for id in added {
                                                            if !refs.contains(&id) {
                                                                refs.push(id);
                                                            }
                                                        }
                                                        if db
                                                            .set_playlist_tracks(
                                                                &Source::Server(sid),
                                                                &pid,
                                                                &refs,
                                                            )
                                                            .await
                                                            .is_ok()
                                                        {
                                                            gens.bump(Table::Playlists);
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    } else {
                                        let db = consume_context::<db::Db>();
                                        spawn(async move {
                                            let store = db.load_playlists().await.unwrap_or_default();
                                            if let Some(playlist) =
                                                store.playlists.iter().find(|p| p.id == playlist_id)
                                            {
                                                let mut paths = playlist.tracks.clone();
                                                for path in tracks {
                                                    if !paths.contains(&path) {
                                                        paths.push(path);
                                                    }
                                                }
                                                let refs: Vec<String> = paths
                                                    .iter()
                                                    .map(|p| p.to_string_lossy().into_owned())
                                                    .collect();
                                                if db
                                                    .set_playlist_tracks(&Source::Local, &playlist_id, &refs)
                                                    .await
                                                    .is_ok()
                                                {
                                                    gens.bump(Table::Playlists);
                                                }
                                            }
                                        });
                                    }
                                }
                                show_album_playlist_modal.set(false);
                            },
                            on_create_playlist: move |name: String| {
                                if pending_album_id_for_playlist.read().is_some() {
                                    let tracks: Vec<_> = pending_tracks_res
                                        .read()
                                        .clone()
                                        .unwrap_or_default()
                                        .iter()
                                        .map(|t| t.id.uid_path())
                                        .collect();
                                    if is_server {
                                        let playlist_name = name.clone();
                                        let paths = tracks.clone();
                                        let server_vals = {
                                            let conf = config.peek();
                                            conf.server
                                                .as_ref()
                                                .and_then(|s| {
                                                    if let (Some(tok), Some(uid)) = (
                                                        &s.access_token,
                                                        &s.user_id,
                                                    ) {
                                                        Some((
                                                            s.service,
                                                            s.url.clone(),
                                                            tok.clone(),
                                                            uid.clone(),
                                                            conf.device_id.clone(),
                                                        ))
                                                    } else {
                                                        None
                                                    }
                                                })
                                        };
                                        if let Some((service, url, token, user_id, device_id)) = server_vals {
                                            let sid = active_server_id();
                                            spawn(async move {
                                                let conn = ::server::server_ops::ServerConn {
                                                    service,
                                                    url,
                                                    token,
                                                    user_id,
                                                    device_id,
                                                };
                                                let item_ids: Vec<String> = paths
                                                    .iter()
                                                    .filter_map(|p| {
                                                        ::server::server_ops::parse_item_id(p.to_str()?)
                                                            .map(str::to_string)
                                                    })
                                                    .collect();
                                                let result = ::server::server_ops::create_server_playlist(
                                                    &conn,
                                                    &playlist_name,
                                                    &item_ids,
                                                )
                                                .await;
                                                if let Ok(new_id) = result {
                                                    let db = consume_context::<db::Db>();
                                                    let source = Source::Server(sid);
                                                    if db
                                                        .upsert_playlist_meta(
                                                            &source,
                                                            &new_id,
                                                            &playlist_name,
                                                            None,
                                                            None,
                                                        )
                                                        .await
                                                        .is_ok()
                                                        && db
                                                            .set_playlist_tracks(&source, &new_id, &item_ids)
                                                            .await
                                                            .is_ok()
                                                    {
                                                        gens.bump(Table::Playlists);
                                                    }
                                                }
                                            });
                                        }
                                    } else {
                                        let refs: Vec<String> = tracks
                                            .iter()
                                            .map(|p| p.to_string_lossy().into_owned())
                                            .collect();
                                        let id = uuid::Uuid::new_v4().to_string();
                                        let db = consume_context::<db::Db>();
                                        spawn(async move {
                                            if db
                                                .upsert_playlist_meta(&Source::Local, &id, &name, None, None)
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
                                    }
                                }
                                show_album_playlist_modal.set(false);
                            },
                        }
                    }
                }
            } else {
                if is_server {
                    ServerAlbumDetails {
                        album_jellyfin_id: album_id.read().clone(),
                        config,
                        queue,
                        on_close: move |_| album_id.set(String::new()),
                    }
                } else {
                    components::album_details::AlbumDetails {
                        album_id: album_id.read().clone(),
                        on_close: move |_| album_id.set(String::new()),
                    }
                }
            }
        }
    }
}
