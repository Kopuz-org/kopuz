use dioxus::prelude::*;
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
    let playlists_res = use_playlists();
    let cover_for = hooks::use_db_queries::use_cover_resolver(512);
    // Still needed by the cover upload and the delete-from-disk paths, which
    // write through the source until the mutation API covers them.

    // The playlist's track refs, resolved from the library. One query, live:
    // the daemon's refresh invalidates as each page lands, so this is both the
    // instant cached view and the progressive one.
    let pid_for_refs = playlist_id.clone();
    let track_refs = use_memo(move || {
        let store = playlists_res.read().clone().unwrap_or_default();
        store
            .playlists
            .iter()
            .find(|playlist| playlist.id == pid_for_refs)
            .map(|playlist| playlist.tracks.clone())
            .unwrap_or_default()
    });
    let active_partition = use_memo(move || config.read().active_source.clone());
    let tracks_res = use_tracks_by_keys(active_partition, track_refs);

    // Affordances are capability-driven, not source-kind-driven: tag-edit and
    // delete-from-disk are local-only, downloads server-only, reorder per the
    // playlists cap (YT's InnerTube has no reorder mutation). Reading the caps is
    // also more correct than `is_server` — e.g. a creds-less offline server has
    // downloads=false.
    let caps = *hooks::sources::use_capabilities().read();
    let can_reorder = caps.playlists == api::PlaylistCapability::Reorder;

    // A server playlist's contents are refreshed by the daemon, a page at a
    // time, and every page invalidates -- so the list fills in as it arrives
    // through the query hook above, without this component owning a copy of
    // it or knowing the walk exists. The daemon gates the staleness.
    let pid_for_refresh = playlist_id.clone();
    use_effect(move || {
        if !caps.sync {
            return;
        }
        let api = hooks::consume_api();
        let id = pid_for_refresh.clone();
        spawn(async move {
            if let Err(error) = api.refresh_playlist(id).await {
                tracing::debug!(%error, "playlist refresh failed");
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

    let tracks_val = tracks_res.read().clone().unwrap_or_default();
    let track_count = tracks_val.len();
    let tracks_for_delete = tracks_val.clone();

    // A custom (locally-picked) cover wins; then a server playlist's remote image
    // tag; then the first track's cover via the source-agnostic seam.
    let playlist_cover = playlist_custom_cover
        .as_ref()
        .and_then(|p| utils::format_artwork_url(Some(p)))
        .or_else(|| {
            let tag = playlist_image_tag.as_ref()?;
            let conf = config.read();
            let server = conf.server.as_ref()?;
            server::cover::resolve(
                &conf,
                reader::CoverRef::remote_item(server.service, &playlist_id, Some(tag.as_str())),
                512,
            )
        })
        .or_else(|| tracks_val.first().and_then(&cover_for));

    let start_radio = crate::radio_actions::playlist_radio_handler(playlist_id.clone());

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
            tracks: tracks_val,
            on_close,
            on_start_radio: start_radio,
            enable_metadata: caps.edit_tags,
            on_cover_click: move |_| {
                let _ = &pid_for_cover;
                let _ = &name_for_cover;
                let _ = &tag_for_cover;
                #[cfg(not(target_os = "android"))]
                {
                    let pid = pid_for_cover.clone();
                    // The daemon decides what "set a cover" means -- Jellyfin
                    // pushes the image upstream, everyone else records it --
                    // so the bytes go across and the policy stays there.
                    spawn(async move {
                        let Some(file) = AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await
                        else {
                            return;
                        };
                        let path = file.path().to_path_buf();
                        let Ok(bytes) = tokio::fs::read(&path).await else {
                            return;
                        };
                        hooks::library_actions::upload_artwork(
                            api::ArtworkTarget::Playlist(pid),
                            hooks::library_actions::content_type_for(&path),
                            bytes,
                        );
                    });
                }
            },
            on_delete_track: move |idx: usize| {
                if let Some(track) = tracks_for_delete.get(idx) {
                    hooks::library_actions::delete_tracks(
                        vec![track.id.key().into_owned()],
                        caps.delete_from_disk,
                    );
                }
            },
            on_selection_delete: move |paths: Vec<PathBuf>| {
                let keys: Vec<String> = paths
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                hooks::library_actions::delete_tracks(keys, caps.delete_from_disk);
            },
            // No optimistic edit: the daemon invalidates as it writes, and the
            // query hook's coalescing window is shorter than the round trip
            // that produced it.
            on_remove_from_playlist: move |idx: usize| {
                hooks::playlist_actions::remove_track(pid_for_remove.clone(), idx);
            },
            is_reorderable: can_reorder,
            on_move_up: move |idx: usize| {
                if idx > 0 && can_reorder {
                    hooks::playlist_actions::reorder(pid_for_move_up.clone(), idx, idx - 1);
                }
            },
            on_move_down: move |idx: usize| {
                if can_reorder && idx + 1 < track_count {
                    hooks::playlist_actions::reorder(pid_for_move_down.clone(), idx, idx + 1);
                }
            },
            on_download_all: if caps.downloads { on_download_all } else { None },
            on_download_track: if caps.downloads { on_download_track } else { None },
            on_delete_all: if caps.downloads { on_delete_all } else { None },
            is_downloading_all,
            show_delete_in_selection: caps.delete_from_disk,
        }
    }
}
