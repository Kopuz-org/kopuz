use std::time::{Duration, Instant};

use gpui_kit::{App, Context, Entity, Subscription, Task};

use crate::PlayerModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepAfter {
    Minutes(u64),
    EndOfTrack,
}

pub struct SleepTimer {
    armed: Option<SleepAfter>,
    deadline: Option<Instant>,
    track: Option<String>,
    _timer: Option<Task<()>>,
    _player: Subscription,
}

impl SleepTimer {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let player = PlayerModel::global(cx);
        Self {
            armed: None,
            deadline: None,
            track: None,
            _timer: None,
            _player: cx.observe(&player, |this, player, cx| {
                if this.armed != Some(SleepAfter::EndOfTrack) {
                    return;
                }
                let now = player.read(cx).track().map(|track| track.key.clone());
                match (&this.track, now) {
                    // Armed with nothing loaded: the first track to start is the one to finish.
                    (None, now) => this.track = now,
                    (Some(armed), Some(now)) if *armed == now => {}
                    _ => this.fire(cx),
                }
            }),
        }
    }

    pub fn armed(&self) -> Option<SleepAfter> {
        self.armed
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    pub fn set(&mut self, after: Option<SleepAfter>, cx: &mut Context<Self>) {
        self.armed = after;
        self.deadline = None;
        self._timer = None;
        self.track = PlayerModel::global(cx)
            .read(cx)
            .track()
            .map(|track| track.key.clone());
        if let Some(SleepAfter::Minutes(minutes)) = after {
            let wait = Duration::from_secs(minutes * 60);
            self.deadline = Some(Instant::now() + wait);
            self._timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                this.update(cx, |this, cx| this.fire(cx)).ok();
            }));
        }
        crate::notify(cx);
    }

    fn fire(&mut self, cx: &mut Context<Self>) {
        self.armed = None;
        self.deadline = None;
        self._timer = None;
        PlayerModel::global(cx).update(cx, |player, cx| player.pause(cx));
        crate::notify(cx);
    }
}
