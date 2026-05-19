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
        None => return rsx! { div { "{i18n::t(\"album_not_found\")}" } },
    };

    let album_title = album.title.clone();
    let album_artist = album.artist.clone();
    let cover_url = utils::format_artwork_url(album.cover_path.as_ref());

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

    rsx! {
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
            on_delete_track: move |idx: usize| {
                if let Some(t) = tracks_for_delete.get(idx) {
                    #[cfg(not(target_arch = "wasm32"))]
                    if std::fs::remove_file(&t.path).is_ok() {
                        library.write().remove_track(&t.path);
                        let lib_path = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
                            .map(|d| d.config_dir().join("library.json"))
                            .unwrap_or_else(|| PathBuf::from("./config/library.json"));
                        let _ = library.read().save(&lib_path);
                    }
                }
            },
            on_selection_delete: move |paths: Vec<PathBuf>| {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    for path in &paths {
                        if std::fs::remove_file(path).is_ok() {
                            library.write().remove_track(path);
                        }
                    }
                    let lib_path = directories::ProjectDirs::from("com", "temidaradev", "kopuz")
                        .map(|d| d.config_dir().join("library.json"))
                        .unwrap_or_else(|| PathBuf::from("./config/library.json"));
                    let _ = library.read().save(&lib_path);
                }
            },
        }
    }
}
