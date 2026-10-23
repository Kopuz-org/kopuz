//! The transport, wired to the player: the paint is `components::Transport`.

use components::{Transport, TransportMetrics};
use gpui_kit::App;
use store::PlayerModel;

use crate::NowPlaying;

impl NowPlaying {
    /// The transport row. `full` draws the optional side toggles the preferences allow.
    pub(crate) fn transport(&self, metrics: TransportMetrics, full: bool, cx: &App) -> Transport {
        let (playing, shuffle, loop_mode, loaded) = {
            let player = PlayerModel::global(cx);
            let player = player.read(cx);
            (
                player.is_playing(),
                player.shuffle(),
                player.loop_mode(),
                player.track().is_some(),
            )
        };
        let wanted = &store::layout(cx).player;

        Transport::new("transport")
            .metrics(metrics)
            .sides(full)
            .balanced(full && self.presentation == crate::Presentation::Sheet)
            .playing(playing)
            .loaded(loaded)
            .shuffle(wanted.shuffle.then_some(shuffle))
            .repeat(wanted.repeat.then_some(loop_mode))
            .on_toggle(|_, cx| PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx)))
            .on_previous(|_, cx| {
                PlayerModel::global(cx).update(cx, |player, cx| player.previous(cx))
            })
            .on_next(|_, cx| PlayerModel::global(cx).update(cx, |player, cx| player.next(cx)))
            .on_shuffle(move |_, cx| {
                PlayerModel::global(cx).update(cx, |player, cx| player.set_shuffle(!shuffle, cx))
            })
            .on_repeat(|_, cx| {
                PlayerModel::global(cx).update(cx, |player, cx| player.cycle_loop_mode(cx))
            })
    }
}
