//! The level control, wired to the player: the paint is `components::VolumeControl`.

use components::VolumeControl;
use gpui_kit::Context;
use store::{ConfigModel, PlayerModel};

use crate::NowPlaying;

impl NowPlaying {
    /// The mute glyph and the level beside it, sharing one wheel.
    pub(crate) fn volume_control(&self, slider: bool, cx: &Context<Self>) -> VolumeControl {
        let player = PlayerModel::global(cx).read(cx);
        let (level, muted) = (player.volume(), player.is_muted());
        let step = ConfigModel::global(cx)
            .read(cx)
            .config()
            .playback
            .volume_scroll_step
            .max(0.);
        let state = self.volume.clone();
        VolumeControl::new("volume", &self.volume)
            .level(level)
            .muted(muted)
            .slider(slider)
            .wide(self.presentation == crate::Presentation::Sheet)
            .held(self.volume_held)
            .scale(self.scale(cx))
            .scroll_step(step)
            .on_mute(|_, cx| {
                PlayerModel::global(cx).update(cx, |player, cx| player.toggle_mute(cx))
            })
            .on_step(move |next, window, cx| {
                state.update(cx, |state, cx| state.set_value(next, window, cx));
                PlayerModel::global(cx).update(cx, |player, cx| player.set_volume(next, cx));
            })
    }
}
