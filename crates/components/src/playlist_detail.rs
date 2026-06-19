use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_playlists, use_tracks_by_keys};
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

    let pid = playlist_id.clone();
    use_effect(move || {
        if *has_loaded_remote.read() {
            return;
        }
        let pid_clone = pid.clone();
        let load_span = tracing::info_span!("playlist.load_entries", playlist_id = %pid_clone);
        // The source fetches its own entries — servers hit the network; local
        // returns the empty default (its tracks come from the seed above). The
        // non-empty guard keeps local from being wiped, so this stays agnostic.
        let source = active_source.peek().clone();
        spawn(
            async move {
                if let Ok(entries) = source.fetch_playlist_entries(&pid_clone).await
                    && !entries.is_empty()
                    && !*has_loaded_remote.peek()
                {
                    tracing::debug!(count = entries.len(), "playlist entries loaded");
                    tracks.set(entries);
                    has_loaded_remote.set(true);
                }
            }
            .instrument(load_span),
        );
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
    let playlist_cover = playlist_custom_cover
        .as_ref()
        .and_then(|p| utils::format_artwork_url(Some(p)))
        .or_else(|| {
            let tag = playlist_image_tag.as_ref()?;
            let conf = config.read();
            let server = conf.server.as_ref()?;
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
        })
        .or_else(|| tracks_val.first().and_then(&cover_for));

    let pid_for_remove = playlist_id.clone();
    let pid_for_move_up = playlist_id.clone();
    let pid_for_move_down = playlist_id.clone();
    let pid_for_cover = playlist_id.clone();
    let name_for_cover = playlist_name.clone();
    let tag_for_cover = playlist_image_tag.clone();

    rsx! {
        crate::track_list_view::TrackListView {
            name: playlist_name.clone(),
            description: String::new(),
            cover_url: playlist_cover,
            back_label: i18n::t("back_to_playlists").to_string(),
            tracks: tracks_val,
            on_close,
            enable_metadata: caps.edit_tags,
            on_cover_click: move |_| {
                let _ = &pid_for_cover;
                let _ = &name_for_cover;
                let _ = &tag_for_cover;
                #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
                {
                    let pid = pid_for_cover.clone();
                    let pl_name = name_for_cover.clone();
                    let pl_tag = tag_for_cover.clone();
                    let source = active_source.peek().clone();
                    spawn(async move {
                        let file = AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await;
                        if let Some(file) = file {
                            let path = file.path().to_path_buf();
                            // The source decides what "set a cover" means — Jellyfin
                            // pushes the image upstream, everyone else just records
                            // the local path.
                            if source
                                .set_playlist_cover(&pid, &pl_name, &path, pl_tag.as_deref())
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
                    && let Some(t) = tracks.read().get(idx).cloned() {
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Some(del_path) = t.id.local_path()
                            && std::fs::remove_file(del_path).is_ok()
                        {
                            let source = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                            let key = t.id.key().into_owned();
                            spawn(async move {
                                if source.delete_tracks(&[key]).await.is_ok() {
                                    gens.bump(Table::Tracks);
                                }
                            });
                        }
                    }
            },
            on_selection_delete: move |paths: Vec<PathBuf>| {
                if caps.delete_from_disk {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        let mut keys = Vec::new();
                        for path in &paths {
                            if std::fs::remove_file(path).is_ok() {
                                keys.push(path.to_string_lossy().into_owned());
                            }
                        }
                        if !keys.is_empty() {
                            let source = consume_context::<Signal<::server::source::ActiveSource>>().peek().clone();
                            spawn(async move {
                                if source.delete_tracks(&keys).await.is_ok() {
                                    gens.bump(Table::Tracks);
                                }
                            });
                        }
                    }
                }
            },
            on_remove_from_playlist: move |idx: usize| {
                if let Some(t) = tracks.read().get(idx).cloned() {
                    let pid = pid_for_remove.clone();
                    let source = active_source.peek().clone();
                    spawn(async move {
                        if source.remove_from_playlist(&pid, &t, idx).await.is_ok() {
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
                let Some(moved) = tracks.read().get(idx - 1).cloned() else {
                    return;
                };
                let pid = pid_for_move_up.clone();
                let source = active_source.peek().clone();
                spawn(async move {
                    if source
                        .reorder_playlist(&pid, &refs, &moved, idx - 1)
                        .await
                        .is_ok()
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
                let Some(moved) = tracks.read().get(idx + 1).cloned() else {
                    return;
                };
                let pid = pid_for_move_down.clone();
                let source = active_source.peek().clone();
                spawn(async move {
                    if source
                        .reorder_playlist(&pid, &refs, &moved, idx + 1)
                        .await
                        .is_ok()
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
