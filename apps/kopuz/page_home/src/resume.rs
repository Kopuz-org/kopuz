//! Continue: the queue the last session was interrupted in, and the one button that picks it up.

use api::TrackInfo;
use components::ResumeCard;
use gpui_kit::{AnyElement, App, IntoElement as _};
use store::{PlayerModel, QueueModel};

/// The queue left behind, as the banner draws it.
#[derive(Clone)]
pub(crate) struct Resume {
    track: TrackInfo,
    left: usize,
    playing: bool,
}

impl Resume {
    /// Nothing when no queue was left behind.
    pub(crate) fn read(cx: &App) -> Option<Self> {
        let queue = QueueModel::global(cx).read(cx);
        let track = queue.current()?.clone();
        let left = queue
            .len()
            .saturating_sub(queue.position().unwrap_or(0) as usize + 1);
        let playing = PlayerModel::global(cx).read(cx).is_playing();
        Some(Self {
            track,
            left,
            playing,
        })
    }
}

/// The banner the page opens with.
pub(crate) fn banner(resume: &Resume) -> AnyElement {
    ResumeCard::new("home-continue", &resume.track)
        .left(resume.left)
        .playing(resume.playing)
        .on_toggle(|_, cx| PlayerModel::global(cx).update(cx, |player, cx| player.toggle(cx)))
        .into_any_element()
}
