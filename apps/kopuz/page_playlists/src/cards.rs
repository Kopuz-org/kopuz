//! The playlist card, and the menu it carries.

use api::PlaylistInfo;
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
