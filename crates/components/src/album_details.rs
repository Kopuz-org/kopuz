use db::Source;
use dioxus::prelude::*;
use hooks::db_reactivity::Table;
use hooks::use_db_queries::{use_album, use_all_tracks};
use reader::Library;
use std::path::PathBuf;

#[component]
pub fn AlbumDetails(
    album_id: String,
    library: Signal<Library>,
    playlist_store: Signal<reader::PlaylistStore>,
    on_close: EventHandler<()>,
) -> Element {
    let gens = hooks::db_reactivity::use_generations();
    let source = use_memo(|| Source::Local);
    let album_id_memo = use_memo(use_reactive!(|album_id| album_id));
    let album_res = use_album(source, album_id_memo);
    let filter = use_memo(move || db::TrackFilter::album(Source::Local, album_id_memo()));
    let tracks_res = use_all_tracks(filter);

    let album_loading = album_res.read().is_none();
    let album = match album_res.read().clone().flatten() {
        Some(a) => a,
        None => {
            if album_loading {
                return rsx! { div {} };
            }
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

    let mut tracks: Vec<_> = tracks_res.read().clone().unwrap_or_default();

    tracks.sort_by(|a, b| {
        a.disc_number.cmp(&b.disc_number).then_with(|| {
            a.track_number
                .cmp(&b.track_number)
                .then_with(|| a.title.cmp(&b.title))
        })
    });

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
                    let db = consume_context::<db::Db>();
                    spawn(async move {
                        let old_cover = delete_cover;
                        if db
                            .update_album_cover(&Source::Local, &aid, None, false)
                            .await
                            .is_ok()
                        {
                            gens.bump(Table::Albums);
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
                    let db = consume_context::<db::Db>();
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
                                let saved_str = saved.to_string_lossy().into_owned();
                                if db
                                    .update_album_cover(
                                        &Source::Local,
                                        &aid,
                                        Some(&saved_str),
                                        true,
                                    )
                                    .await
                                    .is_ok()
                                {
                                    gens.bump(Table::Albums);
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
                },
            }
        }
    }
}
