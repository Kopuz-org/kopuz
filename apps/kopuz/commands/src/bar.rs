use gpui_kit::App;
use store::{DownloadsModel, PlayerModel, actions};

/// Play the keys from the top.
pub fn play_all(keys: &[String], cx: &mut App) {
    actions::play_all(keys.to_vec(), cx);
}

pub fn set_shuffle(on: bool, cx: &mut App) {
    PlayerModel::global(cx).update(cx, |player, cx| player.set_shuffle(on, cx));
}

/// Shuffle on, then play the keys.
pub fn shuffle_play(keys: &[String], cx: &mut App) {
    set_shuffle(true, cx);
    play_all(keys, cx);
}

/// Download the keys or give their offline copies back.
pub fn set_offline(keys: &[String], offline: bool, cx: &mut App) {
    DownloadsModel::global(cx).update(cx, |downloads, cx| {
        downloads.set_offline(keys.to_vec(), offline, cx)
    });
}
