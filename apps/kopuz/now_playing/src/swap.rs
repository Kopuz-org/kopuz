//! A track change: the old cover and title fading out under the new ones rather than cutting.

use components::motion::{self, Fade};
use std::time::{Duration, Instant};

use gpui_kit::{App, Window};

/// Longer than any frame takes, so a swap first drawn this late was not on screen when the track changed.
const UNSEEN: Duration = Duration::from_millis(250);

/// What a surface draws on a frame of a swap: the leaving track, and how far each part of the new one is in.
#[derive(Debug, Clone)]
pub(crate) struct SwapFrame {
    pub leaving: Option<api::TrackInfo>,
    pub title: f32,
    pub cover: f32,
}

impl SwapFrame {
    const SETTLED: Self = Self {
        leaving: None,
        title: 1.,
        cover: 1.,
    };
}

/// The playing track as one surface last drew it, and the one it is taking over from.
#[derive(Default)]
pub(crate) struct TrackSwap {
    drawn: Option<api::TrackInfo>,
    leaving: Option<api::TrackInfo>,
    title: Fade,
    cover: Fade,
    // The new cover waits for its picture, so the old one stays up rather than giving way to an empty frame.
    cover_turned: bool,
    // When the track changed; a surface first drawn well after it, such as a sheet opened later, just shows the new one.
    changed: Option<Instant>,
}

impl TrackSwap {
    /// Note the track now playing; a different one than last time starts the swap.
    pub fn follow(&mut self, track: Option<&api::TrackInfo>) {
        let same = self.drawn.as_ref().map(|drawn| &drawn.key) == track.map(|track| &track.key);
        let first = self.drawn.is_none() && self.leaving.is_none();
        if same || first {
            self.drawn = track.cloned();
            return;
        }
        self.leaving = self.drawn.take();
        self.drawn = track.cloned();
        self.title = Fade::hidden();
        self.cover = Fade::hidden();
        self.cover_turned = false;
        self.changed = Some(Instant::now());
    }

    /// Whether this is the swap's first frame and it came too late to show; a missed swap is dropped.
    fn missed(&mut self) -> bool {
        let missed = self.changed.take().is_some_and(|at| at.elapsed() > UNSEEN);
        if missed {
            self.leaving = None;
        }
        missed
    }

    /// This frame of the swap; `ready` says whether the new cover has a picture to show at the size drawn.
    pub fn frame(&mut self, ready: bool, window: &mut Window, cx: &App) -> SwapFrame {
        if self.leaving.is_none() {
            return SwapFrame::SETTLED;
        }
        if self.missed() {
            return SwapFrame::SETTLED;
        }
        let title = self.title.sample(true, motion::MEDIUM, window, cx);
        self.cover_turned |= ready;
        let cover = match self.cover_turned {
            true => self.cover.sample(true, motion::MEDIUM, window, cx),
            false => 0.,
        };
        if title >= 1. && cover >= 1. {
            self.leaving = None;
            return SwapFrame::SETTLED;
        }
        SwapFrame {
            leaving: self.leaving.clone(),
            title,
            cover,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str) -> api::TrackInfo {
        api::TrackInfo {
            key: key.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_first_track_has_nothing_to_swap_from() {
        let mut swap = TrackSwap::default();
        swap.follow(Some(&track("a")));
        assert!(swap.leaving.is_none());
    }

    #[test]
    fn a_new_track_keeps_the_old_one_to_fade_out() {
        let mut swap = TrackSwap::default();
        swap.follow(Some(&track("a")));
        swap.follow(Some(&track("b")));
        assert_eq!(swap.leaving.as_ref().map(|t| t.key.as_str()), Some("a"));
        assert_eq!(swap.drawn.as_ref().map(|t| t.key.as_str()), Some("b"));
        assert!(!swap.cover_turned);
    }

    #[test]
    fn the_same_track_again_is_no_swap() {
        let mut swap = TrackSwap::default();
        swap.follow(Some(&track("a")));
        swap.follow(Some(&track("a")));
        assert!(swap.leaving.is_none());
    }

    #[test]
    fn a_change_first_drawn_long_after_shows_only_the_new_track() {
        let mut swap = TrackSwap::default();
        swap.follow(Some(&track("a")));
        swap.follow(Some(&track("b")));
        swap.changed = Instant::now().checked_sub(UNSEEN * 4);
        assert!(swap.missed());
        assert!(swap.leaving.is_none());
    }
}
