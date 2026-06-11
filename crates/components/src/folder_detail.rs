use dioxus::prelude::*;
use reader::{Library, PlaylistStore, models::Track};
use std::path::PathBuf;

#[component]
pub fn FolderDetail(
    folder_path: String,
    library: Signal<Library>,
    mut playlist_store: Signal<PlaylistStore>,
    config: Signal<config::AppConfig>,
    on_close: EventHandler<()>,
) -> Element {
    let folder_path_buf = PathBuf::from(&folder_path);
    let folder_name = folder_path_buf
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| folder_path.clone());

    let lib = library.read();
    let mut folder_tracks: Vec<Track> = lib
        .tracks
        .iter()
        .filter(|t| t.id.local_path().is_some_and(|p| p.starts_with(&folder_path_buf)))
        .cloned()
        .collect();
    folder_tracks.sort_by(|a, b| {
        a.disc_number
            .cmp(&b.disc_number)
            .then(a.track_number.cmp(&b.track_number))
            .then(a.title.cmp(&b.title))
    });

    let cover_url = folder_tracks.first().and_then(|t| {
        lib.albums
            .iter()
            .find(|a| a.id == t.album_id)
            .and_then(|a| utils::format_artwork_url(a.cover_path.as_ref()))
    });

    drop(lib);

    let _ = config;

    rsx! {
        crate::track_list_view::TrackListView {
            name: folder_name,
            description: i18n::t("folder_playlist").to_string(),
            cover_url,
            back_label: i18n::t("back_to_playlists").to_string(),
            tracks: folder_tracks,
            library,
            playlist_store,
            on_close,
            enable_metadata: true,
        }
    }
}
