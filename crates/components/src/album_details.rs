use dioxus::prelude::*;
use reader::Library;
use std::path::PathBuf;

#[component]
pub fn AlbumDetails(
    album_id: String,
    library: Signal<Library>,
    playlist_store: Signal<reader::PlaylistStore>,
    on_close: EventHandler<()>,
) -> Element {
    let lib = library.read();
    let album = match lib.albums.iter().find(|a| a.id == album_id) {
        Some(a) => a,
        None => {
            return rsx! {
                div { "{i18n::t(\"album_not_found\")}" }
            };
        }
    };

    let album_title = album.title.clone();
    let album_artist = album.artist.clone();
    let cover_url = utils::format_artwork_url(album.cover_path.as_ref());
    let current_cover = album.cover_path.clone();
    let cover_cache = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
        .map(|d| d.cache_dir().join("covers"))
        .unwrap_or_else(|| PathBuf::from("./cache/covers"));

    let mut tracks: Vec<_> = lib
        .tracks
        .iter()
        .filter(|t| t.album_id == album_id)
        .cloned()
        .collect();

    tracks.sort_by(|a, b| {
        a.disc_number.cmp(&b.disc_number).then_with(|| {
            a.track_number
                .cmp(&b.track_number)
                .then_with(|| a.title.cmp(&b.title))
        })
    });

    drop(lib);

    let tracks_for_delete = tracks.clone();
    let aid = album_id.clone();
    let cover_reset_action = if current_cover.is_some() {
        let aid = aid.clone();
        let delete_cover = current_cover.clone();
        let cover_cache = cover_cache.clone();
        Some(rsx! {
            button {
                class: "inline-flex items-center justify-center h-9 w-9 rounded-full text-sm font-medium transition-colors border border-white/12 hover:bg-white/10",
                style: "color: var(--color-white); opacity: 0.6;",
                aria_label: i18n::t("remove_cover").to_string(),
                title: i18n::t("remove_cover").to_string(),
                onclick: move |_| {
                    let aid = aid.clone();
                    let delete_cover = delete_cover.clone();
                    let cover_cache = cover_cache.clone();
                    spawn(async move {
                        let old_cover = delete_cover;
                        {
                            let mut lib = library.write();
                            if let Some(album) = lib.albums.iter_mut().find(|a| a.id == aid) {
                                album.cover_path = None;
                                album.manual_cover = false;
                            }
                        }

                        if let Some(path) = old_cover
                            && path.starts_with(&cover_cache)
                        {
                            let _ = tokio::fs::remove_file(&path).await;
                        }
                    });
                },
                i { class: "fa-solid fa-trash text-xs" }
            }
        })
    } else {
        None
    };

    rsx! {
        div { class: "absolute inset-0 flex flex-col overflow-hidden p-8",
            crate::track_list_view::TrackListView {
                name: album_title,
                description: album_artist,
                cover_url,
                is_album: true,
                back_label: i18n::t("back_to_albums").to_string(),
                tracks,
                library,
                playlist_store,
                on_close,
                enable_metadata: true,
                on_cover_click: move |_| {
                    let aid = aid.clone();
                    let _ = &aid;
                    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
                    spawn(async move {
                        let file = rfd::AsyncFileDialog::new()
                            .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
                            .pick_file()
                            .await;
                        if let Some(file) = file {
                            let path = file.path().to_path_buf();
                            let data = match tokio::fs::read(&path).await {
                                Ok(d) => d,
                                Err(_) => return,
                            };
                            let cover_cache = directories::ProjectDirs::from(
                                    "com",
                                    "temidaradev",
                                    "kopuz",
                                )
                                .map(|d| d.cache_dir().join("covers"))
                                .unwrap_or_else(|| PathBuf::from("./cache/covers"));
                            if let Ok(saved) = reader::utils::save_cover(
                                &aid,
                                &data,
                                path.extension().and_then(|e| e.to_str()),
                                &cover_cache,
                            ) {
                                let mut lib = library.write();
                                if let Some(album) =
                                    lib.albums.iter_mut().find(|a| a.id == aid)
                                {
                                    album.cover_path = Some(saved.clone());
                                    album.manual_cover = true;
                                }
                            }
                        }
                    });
                },
                actions: cover_reset_action,
                on_delete_track: move |idx: usize| {
                    if let Some(t) = tracks_for_delete.get(idx) {
                        #[cfg(not(target_arch = "wasm32"))]
                        if let Some(track_path) = t.id.local_path()
                            && std::fs::remove_file(track_path).is_ok()
                        {
                            library.write().remove_track(&t.id);
                        }
                    }
                },
                on_selection_delete: move |paths: Vec<PathBuf>| {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        for path in &paths {
                            if std::fs::remove_file(path).is_ok() {
                                library
                                    .write()
                                    .remove_track(&reader::models::TrackId::Local(path.clone()));
                            }
                        }
                    }
                },
            }
        }
    }
}
