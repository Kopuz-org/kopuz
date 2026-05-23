use ::server::jellyfin::JellyfinClient;
use ::server::subsonic::SubsonicClient;
use components::folder_detail::FolderDetail;
use components::playlist_detail::PlaylistDetail;
use components::playlist_popups::AddPlaylistPopup;
use config::{AppConfig, MusicService, MusicSource, UiStyle};
use dioxus::prelude::*;
use reader::{Library, PlaylistStore};

use crate::local::playlists::LocalPlaylists;
use crate::server::download_manager::{DownloadQueue, DownloadStatus, queue_downloads};
use crate::server::playlists::ServerPlaylists;

#[component]
pub fn PlaylistsPage(
    playlist_store: Signal<PlaylistStore>,
    library: Signal<Library>,
    config: Signal<AppConfig>,
    mut selected_playlist_id: Signal<Option<String>>,
) -> Element {
    let is_server = config.read().active_source == MusicSource::Server;

    let mut selected_folder = use_signal(|| Option::<String>::None);
    let mut show_add_playlist = use_signal(|| false);
    let mut playlist_name = use_signal(|| String::new());
    let mut error = use_signal(|| Option::<String>::None);
    let mut saving = use_signal(|| false);
    let mut playlist_refresh_trigger = use_signal(|| 0u64);

    let handle_add_playlist = move |_| {
        if saving() {
            return;
        }
        let name = playlist_name();
        if is_server {
            let server_vals = {
                let conf = config.peek();
                conf.server.as_ref().and_then(|s| {
                    if let (Some(tok), Some(uid)) = (&s.access_token, &s.user_id) {
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
                error.set(None);
                saving.set(true);
                spawn(async move {
                    let result = match service {
                        MusicService::Jellyfin => {
                            let remote =
                                JellyfinClient::new(&url, Some(&token), &device_id, Some(&user_id));
                            remote.create_playlist(&name, &[]).await
                        }
                        MusicService::Subsonic | MusicService::Custom => {
                            let remote = SubsonicClient::new(&url, &user_id, &token);
                            remote.create_playlist(&name, &[]).await
                        }
                    };
                    saving.set(false);
                    match result {
                        Ok(_) => {
                            playlist_refresh_trigger.with_mut(|v| *v += 1);
                            show_add_playlist.set(false);
                            playlist_name.set(String::new());
                        }
                        Err(e) => {
                            error.set(Some(e));
                        }
                    }
                });
            } else {
                error.set(Some(i18n::t("error_server_not_configured").to_string()));
            }
        } else {
            let mut store = playlist_store.write();
            store.playlists.push(reader::models::Playlist {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                tracks: Vec::new(),
                cover_path: None,
            });
            show_add_playlist.set(false);
            playlist_name.set(String::new());
        }
    };

    let download_queue = use_context::<Signal<DownloadQueue>>();

    let mut last_source = use_signal(|| config.read().active_source.clone());
    if *last_source.read() != config.read().active_source {
        selected_playlist_id.set(None);
        last_source.set(config.read().active_source.clone());
    }

    let is_modern = config.read().ui_style == UiStyle::Modern;

    rsx! {
        div { class: if is_modern { "px-6 pt-6 absolute inset-0 flex flex-col" } else { "px-8 pt-8 absolute inset-0 flex flex-col" },
            if let Some(folder_path) = selected_folder.read().clone() {
                FolderDetail {
                    folder_path,
                    library,
                    playlist_store,
                    config,
                    on_close: move |_| selected_folder.set(None),
                }
            } else if let Some(pid) = selected_playlist_id.read().clone() {
                {
                    let pid_for_dl = pid.clone();
                    let is_downloading_all = {
                        let store = playlist_store.read();
                        let track_ids = store
                            .jellyfin_playlists
                            .iter()
                            .find(|p| p.id == pid)
                            .map(|p| p.tracks.clone())
                            .unwrap_or_default();
                        let q = download_queue.read();
                        track_ids
                            .iter()
                            .any(|tid| {
                                q
                                    .items
                                    .iter()
                                    .any(|i| {
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
                            playlist_store,
                            library,
                            config,
                            on_close: move |_| selected_playlist_id.set(None),
                            is_downloading_all,
                            on_download_all: move |_| {
                                let requests: Vec<(String, String, String)> = {
                                    let store = playlist_store.read();
                                    let lib = library.read();
                                    store
                                        .jellyfin_playlists
                                        .iter()
                                        .find(|p| p.id == pid_for_dl)
                                        .map(|p| {
                                            p
                                                .tracks
                                                .iter()
                                                .map(|tid| {
                                                    let meta = lib
                                                        .jellyfin_tracks
                                                        .iter()
                                                        .find(|t| t.path.to_string_lossy().contains(tid.as_str()));
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
                            on_delete_all: {
                                move |_| {
                                    let ids: Vec<String> = {
                                        let store = playlist_store.read();
                                        store
                                            .jellyfin_playlists
                                            .iter()
                                            .find(|p| p.id == pid_for_del)
                                            .map(|p| p.tracks.clone())
                                            .unwrap_or_default()
                                    };
                                    if !ids.is_empty() {
                                        crate::server::download_manager::delete_downloads(
                                            ids,
                                            config,
                                            download_queue,
                                        );
                                    }
                                }
                            },
                            on_download_track: {
                                move |idx: usize| {
                                    let store = playlist_store.read();
                                    let lib = library.read();
                                    let mut track_id = String::new();
                                    let mut track_title = String::new();
                                    let mut track_artist = String::new();

                                    if let Some(p) = store
                                        .jellyfin_playlists
                                        .iter()
                                        .find(|p| p.id == pid_for_dl_track)
                                    {
                                        if let Some(tid) = p.tracks.get(idx) {
                                            track_id = tid.clone();
                                            if let Some(meta) = lib
                                                .jellyfin_tracks
                                                .iter()
                                                .find(|t| t.path.to_string_lossy().contains(tid.as_str()))
                                            {
                                                track_title = meta.title.clone();
                                                track_artist = meta.artist.clone();
                                            }
                                        }
                                    }
                                    if !track_id.is_empty() {
                                        let is_downloaded = if let Some(path_str) = config
                                            .read()
                                            .offline_tracks
                                            .get(&track_id)
                                        {
                                            std::path::Path::new(path_str).exists()
                                        } else {
                                            false
                                        };
                                        if is_downloaded {
                                            crate::server::download_manager::delete_downloads(
                                                vec![track_id],
                                                config,
                                                download_queue,
                                            );
                                        } else {
                                            crate::server::download_manager::queue_downloads(
                                                vec![(track_id, track_title, track_artist)],
                                                config,
                                                download_queue,
                                            );
                                        }
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
                        if !is_server {
                            button {
                                class: "text-white/60 flex items-center hover:text-white transition-colors p-3 rounded-full hover:bg-white/10",
                                title: i18n::t("new_folder").to_string(),
                                onclick: move |_| {
                                    let new_id = uuid::Uuid::new_v4().to_string();
                                    playlist_store
                                        .write()
                                        .folders
                                        .push(reader::PlaylistFolder {
                                            id: new_id,
                                            name: i18n::t("new_folder").to_string(),
                                            playlist_ids: vec![],
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
                        show_add_folder: !is_server,
                        on_add_folder: move |folder_path: String| {
                            let folder_path_buf = std::path::PathBuf::from(&folder_path);
                            let folder_name = folder_path_buf
                                .file_name()
                                .map(|name| name.to_string_lossy().to_string())
                                .unwrap_or_else(|| folder_path.clone());
                            let tracks = library
                                .read()
                                .tracks
                                .iter()
                                .filter(|track| track.path.starts_with(&folder_path_buf))
                                .map(|track| track.path.clone())
                                .collect();

                            playlist_store
                                .write()
                                .playlists
                                .push(reader::models::Playlist {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    name: folder_name,
                                    tracks,
                                    cover_path: None,
                                });
                            error.set(None);
                            playlist_name.set(String::new());
                        },
                    }
                }

                if is_server {
                    ServerPlaylists {
                        playlist_store,
                        library,
                        config,
                        selected_playlist_id,
                        refresh_trigger: playlist_refresh_trigger,
                    }
                } else {
                    LocalPlaylists {
                        playlist_store,
                        library,
                        config,
                        selected_playlist_id,
                        on_select_folder: move |path| selected_folder.set(Some(path)),
                    }
                }
            }
        }
    }
}
