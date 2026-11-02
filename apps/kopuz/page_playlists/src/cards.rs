//! The playlist and folder cards, and the menus they carry.

use api::{PlaylistFolderInfo, PlaylistInfo};
use components::format::t_with;
use components::{Frame, Tile, icons};
use gpui_kit::{Pixels, px};
use store::{Pictured, Route, Target};

/// The `w-full h-32` picture inside a card's `p-4`.
const COVER: Pixels = px(128.);

/// The card for one playlist; whether it sits in a folder is the catalog's answer, not a flag.
pub(crate) fn playlist_card(playlist: &PlaylistInfo) -> Tile {
    Tile::new(playlist.id.clone(), playlist.name.clone())
        .subtitle(t_with(
            "playlist_track_count",
            &[("count", playlist.track_keys.len().into())],
        ))
        .artwork(playlist.artwork.as_ref())
        .pictured(Pictured::Playlist(playlist.name.clone().into()))
        .placeholder(icons::LIST)
        .size(COVER)
        .frame(Frame::Card)
        .route(Route::Playlist(playlist.id.clone().into()))
        .menu(Some(Target::Playlist {
            id: playlist.id.clone().into(),
            name: playlist.name.clone().into(),
        }))
}

/// The card for one folder; its cover is the first playlist's.
pub(crate) fn folder_card(folder: &PlaylistFolderInfo, playlists: &[PlaylistInfo]) -> Tile {
    let cover = folder
        .playlist_ids
        .first()
        .and_then(|id| playlists.iter().find(|playlist| &playlist.id == id))
        .and_then(|playlist| playlist.artwork.as_ref());
    // TODO(i18n): the Dioxus card spells this count out in English too.
    Tile::new(folder.id.clone(), folder.name.clone())
        .subtitle(format!("{} playlists", folder.playlist_ids.len()))
        .artwork(cover)
        .placeholder(icons::FOLDER)
        .size(COVER)
        .frame(Frame::Card)
        .route(Route::PlaylistFolder(folder.id.clone().into()))
        .menu(Some(Target::Folder {
            id: folder.id.clone().into(),
            name: folder.name.clone().into(),
        }))
}
