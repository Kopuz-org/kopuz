use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::TestAppContext;
use store::Playhead;

use super::*;

pub(crate) fn boot(cx: &mut TestAppContext) {
    // The link talks to its tokio runtime from another thread, which the test scheduler refuses by default.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        i18n::init("en");
        link::init(link::Address::Socket("/nonexistent/kopuz.sock".into()), cx)
            .expect("a link that has not connected yet");
        store::init(cx);
        theme::apply_named("default", None, cx);
    });
}

pub(crate) fn counted<T: 'static>(
    entity: &Entity<T>,
    cx: &mut gpui_kit::VisualTestContext,
) -> Rc<Cell<usize>> {
    let count = Rc::new(Cell::new(0));
    let seen = count.clone();
    cx.update(|_, cx| {
        cx.observe(entity, move |_, _| seen.set(seen.get() + 1))
            .detach()
    });
    count
}

#[gpui_kit::test]
fn a_playhead_tick_redraws_the_seek_row_and_leaves_the_player_alone(cx: &mut TestAppContext) {
    boot(cx);
    let (player, cx) = cx.add_window_view(NowPlaying::bar);
    // Let the first failed connection attempt land before counting anything.
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.run_until_parked();
    let seek = player.read_with(cx, |player, _| player.seek.clone().expect("a seek row"));
    let player_draws = counted(&player, cx);
    let seek_draws = counted(&seek, cx);

    for _ in 0..3 {
        cx.update(|_, cx| Playhead::global(cx).update(cx, |_, cx| cx.notify()));
        cx.run_until_parked();
    }

    assert_eq!(seek_draws.get(), 3);
    assert_eq!(player_draws.get(), 0);
}
