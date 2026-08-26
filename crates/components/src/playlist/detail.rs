use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_playlists, use_tracks_by_keys};
#[cfg(not(target_os = "android"))]
use rfd::AsyncFileDialog;
use std::path::PathBuf;

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
    let mut has_loaded_remote = use_signal(|| false);
    let gens = hooks::db_reactivity::use_generations();
    let active_source = use_context::<Signal<::server::source::ActiveSource>>();
    let playlists_res = use_playlists();
    let cover_for = hooks::use_db_queries::use_cover_resolver(512);

    // Seed = the stored playlist's track refs, resolved from the ACTIVE source's
    // partition (the store only holds the active source's playlists).
    let pid_for_seed = playlist_id.clone();
    let seed_refs = use_memo(move || {
        let store = playlists_res.read().clone().unwrap_or_default();
        store
            .playlists
            .iter()
            .find(|p| p.id == pid_for_seed)
            .map(|p| p.tracks.clone())
            .unwrap_or_default()
    });
    let active_partition = use_memo(move || config.read().active_source.clone());
    let seed_tracks_res = use_tracks_by_keys(active_partition, seed_refs);

    // Affordances are capability-driven, not source-kind-driven: tag-edit and
    // delete-from-disk are local-only, downloads server-only, reorder per the
    // playlists cap (YT's InnerTube has no reorder mutation). Reading the caps is
    // also more correct than `is_server` — e.g. a creds-less offline server has
    // downloads=false.
    let caps = active_source.read().capabilities();
    let can_reorder = caps.playlists == ::server::source::PlaylistOps::Reorder;

    // Initial tracks with no network round-trip: resolve the playlist's refs from
    // the active source's cached/local rows. A server's live entries (below)
    // replace this once they arrive; local has no remote entries, so this stands.
    use_effect(move || {
        if !*has_loaded_remote.read() {
            tracks.set(seed_tracks_res.read().clone().unwrap_or_default());
        }
    });

    let api = use_context::<std::sync::Arc<dyn api::KopuzApi>>();
    let pid = playlist_id.clone();
    let remote_entries = caps.sync;
    use_effect(move || {
        if *has_loaded_remote.read() || !remote_entries {
            return;
        }
        let pid_clone = pid.clone();
        let api = api.clone();
        has_loaded_remote.set(true);
        spawn(async move {
            match api
                .refresh_playlist(api::PlaylistTracksRequest {
                    id: pid_clone.clone(),
                    page: api::Page {
                        offset: 0,
                        limit: u32::MAX,
                    },
                })
                .await
            {
                Ok(page) => tracks.set(
                    page.items
                        .into_iter()
                        .map(hooks::use_db_queries::track_from_api)
                        .collect(),
                ),
                Err(error) => {
                    tracing::warn!(playlist_id = %pid_clone, %error, "playlist refresh failed");
                }
            }
        });
    });

    let store_loading = playlists_res.read().is_none();
    let store = playlists_res.read().clone().unwrap_or_default();
    let (playlist_name, playlist_custom_cover, playlist_image_tag) =
        if let Some(p) = store.playlists.iter().find(|p| p.id == playlist_id) {
            (p.name.clone(), p.cover_path.clone(), p.image_tag.clone())
        } else if store_loading {
            return rsx! { div {} };
        } else {
            return rsx! { div { "{i18n::t(\"playlist_not_found\")}" } };
        };

    let tracks_val = tracks.read().clone();

    // A custom (locally-picked) cover wins; then a server playlist's remote image
    // tag; then the first track's cover via the source-agnostic seam.
    let playlist_cover = server::cover::playlist(
        &config.read(),
        &playlist_id,
        playlist_custom_cover.as_deref(),
        playlist_image_tag.as_deref(),
        512,
    )
    .or_else(|| tracks_val.first().and_then(&cover_for));

    let start_radio = crate::radio_actions::playlist_radio_handler(playlist_id.clone());

    let pid_for_remove = playlist_id.clone();
    let pid_for_move_up = playlist_id.clone();
    let pid_for_move_down = playlist_id.clone();
    let pid_for_cover = playlist_id.clone();

    rsx! {
        crate::track_list_view::TrackListView {
            name: playlist_name.clone(),
            description: String::new(),
            cover_url: playlist_cover,
            tracks: tracks_val,
            on_close,
            on_start_radio: start_radio,
            enable_metadata: caps.edit_tags,
            on_cover_click: move |_| {
                let _ = &pid_for_cover;
                #[cfg(not(target_os = "android"))]
                {
                    let pid = pid_for_cover.clone();
                    spawn(async move {
                        let file = AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await;
                        if let Some(file) = file {
                            let content_type = match file
                                .path()
                                .extension()
                                .and_then(|value| value.to_str())
                                .map(str::to_ascii_lowercase)
                                .as_deref()
                            {
                                Some("png") => "image/png",
                                Some("webp") => "image/webp",
                                _ => "image/jpeg",
                            };
                            let data = file.read().await;
                            let api = consume_context::<std::sync::Arc<dyn api::KopuzApi>>();
                            if api
                                .upload_artwork(api::ArtworkUpload {
                                    target: Some(api::ArtworkTarget::Playlist { id: pid }),
                                    content_type: content_type.to_string(),
                                    data,
                                })
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
                if caps.delete_from_disk
                    && let Some(t) = tracks.read().get(idx).cloned()
                {
                    let api = consume_context::<std::sync::Arc<dyn api::KopuzApi>>();
                    let key = t.id.key().into_owned();
                    spawn(async move {
                        if api.delete_tracks(vec![key], true).await.is_ok() {
                            gens.bump(Table::Tracks);
                        }
                    });
                }
            },
            on_selection_delete: move |paths: Vec<PathBuf>| {
                if caps.delete_from_disk {
                    let keys: Vec<String> = paths
                        .into_iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect();
                    if !keys.is_empty() {
                        let api = consume_context::<std::sync::Arc<dyn api::KopuzApi>>();
                        spawn(async move {
                            if api.delete_tracks(keys, true).await.is_ok() {
                                gens.bump(Table::Tracks);
                            }
                        });
                    }
                }
            },
            on_remove_from_playlist: move |idx: usize| {
                if let Some(t) = tracks.read().get(idx).cloned() {
                    let pid = pid_for_remove.clone();
                    let api = consume_context::<std::sync::Arc<dyn api::KopuzApi>>();
                    let key = t.id.key().into_owned();
                    spawn(async move {
                        if api.remove_playlist_tracks(pid, vec![key]).await.is_ok() {
                            let mut tw = tracks.write();
                            if idx < tw.len() {
                                tw.remove(idx);
                            }
                            gens.bump(Table::Playlists);
                        }
                    });
                }
            },
            is_reorderable: can_reorder,
            on_move_up: move |idx: usize| {
                if idx == 0 || !can_reorder {
                    return;
                }
                tracks.write().swap(idx - 1, idx);
                let mut refs = {
                    let store = playlists_res.read();
                    let Some(pl) = store
                        .as_ref()
                        .and_then(|s| s.playlists.iter().find(|p| p.id == pid_for_move_up))
                    else {
                        return;
                    };
                    if idx >= pl.tracks.len() {
                        return;
                    }
                    pl.tracks.clone()
                };
                refs.swap(idx - 1, idx);
                let pid = pid_for_move_up.clone();
                let api = consume_context::<std::sync::Arc<dyn api::KopuzApi>>();
                spawn(async move {
                    if api.reorder_playlist_tracks(pid, refs).await.is_ok()
                    {
                        gens.bump(Table::Playlists);
                    }
                });
            },
            on_move_down: move |idx: usize| {
                let len = tracks.read().len();
                if idx + 1 >= len || !can_reorder {
                    return;
                }
                tracks.write().swap(idx, idx + 1);
                let mut refs = {
                    let store = playlists_res.read();
                    let Some(pl) = store
                        .as_ref()
                        .and_then(|s| s.playlists.iter().find(|p| p.id == pid_for_move_down))
                    else {
                        return;
                    };
                    if idx + 1 >= pl.tracks.len() {
                        return;
                    }
                    pl.tracks.clone()
                };
                refs.swap(idx, idx + 1);
                let pid = pid_for_move_down.clone();
                let api = consume_context::<std::sync::Arc<dyn api::KopuzApi>>();
                spawn(async move {
                    if api.reorder_playlist_tracks(pid, refs).await.is_ok()
                    {
                        gens.bump(Table::Playlists);
                    }
                });
            },
            on_download_all: if caps.downloads { on_download_all } else { None },
            on_download_track: if caps.downloads { on_download_track } else { None },
            on_delete_all: if caps.downloads { on_delete_all } else { None },
            is_downloading_all,
            show_delete_in_selection: caps.delete_from_disk,
        }
    }
}
