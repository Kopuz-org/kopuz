use config::Source;
use dioxus::prelude::*;
use hooks::use_db_queries::{use_albums, use_folder_tracks};
use reader::models::Track;
use std::path::PathBuf;

#[component]
pub fn FolderDetail(
    folder_path: String,
    config: Signal<config::AppConfig>,
    on_close: EventHandler<()>,
) -> Element {
    let folder_path_buf = PathBuf::from(&folder_path);
    let folder_name = folder_path_buf
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| folder_path.clone());

    let source = use_memo(|| Source::Local);
    let prefix = use_memo(use_reactive!(|folder_path| {
        if folder_path.ends_with(std::path::MAIN_SEPARATOR) {
            folder_path
        } else {
            format!("{folder_path}{}", std::path::MAIN_SEPARATOR)
        }
    }));
    let tracks_res = use_folder_tracks(prefix);
    let albums_res = use_albums(source);

    let mut folder_tracks: Vec<Track> = tracks_res.read().clone().unwrap_or_default();
    folder_tracks.sort_by(|a, b| {
        a.disc_number
            .cmp(&b.disc_number)
            .then(a.track_number.cmp(&b.track_number))
            .then(a.title.cmp(&b.title))
    });

    let cover_url = folder_tracks.first().and_then(|t| {
        albums_res
            .read()
            .as_ref()
            .and_then(|albums| albums.iter().find(|a| a.id == t.album_id))
            .and_then(|a| utils::format_artwork_url(a.cover_path.as_ref()))
    });

    let _ = config;

    rsx! {
        crate::track_list_view::TrackListView {
            name: folder_name,
            description: i18n::t("folder_playlist").to_string(),
            cover_url,
            back_label: i18n::t("back_to_playlists").to_string(),
            tracks: folder_tracks,
            on_close,
            enable_metadata: true,
        }
    }
}
