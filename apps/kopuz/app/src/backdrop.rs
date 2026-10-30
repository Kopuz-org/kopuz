//! What is behind the window: a wallpaper the user asked for, else the light the playing cover casts.

use gpui_kit::{AnyElement, App, Window};
use store::PlayerModel;

/// The window's backdrop, drawn off whatever is playing.
pub(crate) fn layer(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let artwork = PlayerModel::global(cx)
        .read(cx)
        .track()
        .and_then(|track| track.artwork.clone());
    components::backdrop::layer(artwork.as_ref(), 0, window, cx)
}
