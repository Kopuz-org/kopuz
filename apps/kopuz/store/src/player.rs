use std::time::{Duration, Instant};

use api::{
    ApiEvent, BufferedRange, ExternalPlayback, Intent, LoopMode, Phase, PlayerCommand, PlayerState,
    PositionAnchor, TrackInfo, TrackKind,
};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;

// Fast enough for a seek bar's second labels; lyrics request their own frames.
const PLAYHEAD_TICK: Duration = Duration::from_millis(250);

/// The daemon's player, mirrored from `PlayerState` events with the position interpolated locally.
pub struct PlayerModel {
    state: PlayerState,
    // This process's instant at which the daemon clock read `state.now_ms`.
    observed_at: Instant,
    optimistic_playing: Option<bool>,
    revision: Option<u64>,
    // The unknown token already resynced for, so a stream of stale positions cannot spin on refetch.
    resynced_for: Option<u64>,
    // A seek asked for and not yet seen land; positions from before it are the old place.
    seeking: Option<Seek>,
    _fetch: Option<Task<()>>,
    _subscription: Subscription,
}

/// A seek the daemon has been told about but has not yet reported back from.
#[derive(Debug, Clone, Copy)]
struct Seek {
    to_ms: u64,
    /// The daemon clock when it was asked for: anything stamped earlier predates it.
    at_ms: u64,
    since: Instant,
}

/// How near the target a reported position has to be to count as the seek having landed.
const SEEK_LANDED_MS: u64 = 1_500;
/// How long a seek is waited for before the daemon is believed whatever it says.
const SEEK_GRACE: Duration = Duration::from_secs(2);

/// Whether a reported position should replace a seek that has not been seen to land.
fn landed(seek: &Seek, position_ms: u64, at_ms: u64, waited: Duration) -> bool {
    waited >= SEEK_GRACE
        || (at_ms >= seek.at_ms && position_ms.abs_diff(seek.to_ms) <= SEEK_LANDED_MS)
}

impl PlayerModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let connection = Link::connection(cx);
        let subscription = cx.subscribe(&connection, |this, _, event: &ApiEvent, cx| {
            this.apply_event(event, cx)
        });
        Self {
            state: PlayerState::default(),
            observed_at: Instant::now(),
            optimistic_playing: None,
            revision: None,
            resynced_for: None,
            seeking: None,
            _fetch: None,
            _subscription: subscription,
        }
    }

    fn apply_event(&mut self, event: &ApiEvent, cx: &mut Context<Self>) {
        match event {
            ApiEvent::PlayerState(state) => self.apply_state((**state).clone(), cx),
            ApiEvent::PlayerPosition {
                token,
                position_ms,
                at_ms,
                playing,
            } => {
                // Only a position for the track we are showing may move our clock anchor.
                if Some(*token) == self.visible_token() {
                    // A seek in flight means the daemon is still reporting where it was.
                    match self.seeking {
                        Some(seek)
                            if !landed(&seek, *position_ms, *at_ms, seek.since.elapsed()) => {}
                        _ => {
                            self.seeking = None;
                            self.observed_at = Instant::now();
                            self.state.now_ms = *at_ms;
                            self.state.position = Some(PositionAnchor {
                                ms: *position_ms,
                                at_ms: *at_ms,
                                playing: *playing,
                            });
                        }
                    }
                } else if self.resynced_for != Some(*token) {
                    self.resynced_for = Some(*token);
                    self.refetch(cx);
                }
                crate::notify(cx);
            }
            ApiEvent::PlayerBuffered { token, ranges } if Some(*token) == self.token() => {
                self.state.buffered = ranges.clone();
                crate::notify(cx);
            }
            ApiEvent::QueueChanged { .. } => self.refetch(cx),
            ApiEvent::Resync => {
                self.revision = None;
                self.resynced_for = None;
                self.refetch(cx);
            }
            _ => {}
        }
    }

    fn refetch(&mut self, cx: &mut Context<Self>) {
        if self._fetch.is_some() {
            return;
        }
        let call = Link::global(cx).call(|api| async move { api.player_state().await });
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let state = call.await;
            this.update(cx, |this, cx| {
                this._fetch = None;
                if let Ok(state) = state {
                    this.apply_state(state, cx);
                }
            })
            .ok();
        }));
    }

    fn apply_state(&mut self, state: PlayerState, cx: &mut Context<Self>) {
        if !adopt_state(&mut self.state, &mut self.revision, state) {
            return;
        }
        self.observed_at = Instant::now();
        self.optimistic_playing = None;
        // A whole state is the daemon's own answer, so there is nothing left to wait for.
        self.seeking = None;
        crate::notify(cx);
    }

    fn token(&self) -> Option<u64> {
        match self.state.intent {
            Intent::Stopped => None,
            Intent::Loading { token, .. } | Intent::Committed { token } => Some(token),
        }
    }

    /// The token whose position the daemon anchors: the outgoing track's while a crossfade runs.
    fn visible_token(&self) -> Option<u64> {
        match &self.state.fading {
            Some(fading) => Some(fading.from_token),
            None => self.token(),
        }
    }

    fn daemon_now_ms(&self) -> u64 {
        self.state.now_ms + self.observed_at.elapsed().as_millis() as u64
    }

    pub fn state(&self) -> &PlayerState {
        &self.state
    }

    /// What to show as playing: the outgoing track during a crossfade, else the current one.
    pub fn track(&self) -> Option<&TrackInfo> {
        match &self.state.fading {
            Some(fading) => Some(&fading.track),
            None => self.state.track.as_ref(),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.optimistic_playing.unwrap_or(match self.state.intent {
            Intent::Loading { .. } => true,
            Intent::Committed { .. } => self.state.phase == Phase::Playing,
            Intent::Stopped => false,
        })
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.state.intent, Intent::Loading { .. })
    }

    pub fn is_radio(&self) -> bool {
        self.track()
            .is_some_and(|track| track.kind == TrackKind::Radio)
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.track().and_then(|track| track.duration_ms)
    }

    pub fn position_ms(&self) -> u64 {
        let Some(anchor) = self.state.position else {
            return 0;
        };
        let position = if anchor.playing {
            anchor.ms + self.daemon_now_ms().saturating_sub(anchor.at_ms)
        } else {
            anchor.ms
        };
        self.duration_ms()
            .map_or(position, |duration| position.min(duration))
    }

    pub fn buffered(&self) -> &[BufferedRange] {
        &self.state.buffered
    }

    pub fn volume(&self) -> f32 {
        self.state.volume
    }

    pub fn shuffle(&self) -> bool {
        self.state.queue.shuffle
    }

    pub fn loop_mode(&self) -> LoopMode {
        self.state.queue.loop_mode
    }

    /// The container the daemon says the queue was built from, for "playing from".
    pub fn queue_context(&self) -> Option<&api::QueueContext> {
        self.state.queue.context.as_ref()
    }

    pub fn error(&self) -> Option<&api::ErrorBody> {
        self.state.error.as_ref()
    }

    pub fn external(&self) -> Option<&ExternalPlayback> {
        self.state.external.as_ref()
    }

    pub fn output_latency_ms(&self) -> Option<u64> {
        self.state.output_latency_ms
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.optimistic_playing = Some(!self.is_playing());
        crate::notify(cx);
        self.command(PlayerCommand::Toggle, cx);
    }

    pub fn play(&mut self, cx: &mut Context<Self>) {
        self.optimistic_playing = Some(true);
        crate::notify(cx);
        self.command(PlayerCommand::Play, cx);
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.optimistic_playing = Some(false);
        crate::notify(cx);
        self.command(PlayerCommand::Pause, cx);
    }

    pub fn next(&mut self, cx: &mut Context<Self>) {
        self.command(PlayerCommand::Next, cx);
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        self.command(PlayerCommand::Previous, cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.command(PlayerCommand::Stop, cx);
    }

    /// Send a transport command and, if it is refused, stop pretending it landed.
    fn command(&mut self, command: PlayerCommand, cx: &mut Context<Self>) {
        let call =
            Link::global(cx).call(move |api| async move { api.player_command(command).await });
        cx.spawn(async move |this, cx| {
            let Err(error) = call.await else { return };
            this.update(cx, |this, cx| {
                this.optimistic_playing = None;
                crate::notify(cx);
            })
            .ok();
            cx.update(|cx| crate::Notices::failed(&error, cx));
        })
        .detach();
    }

    pub fn seek(&mut self, position_ms: u64, cx: &mut Context<Self>) {
        let at_ms = self.daemon_now_ms();
        self.seeking = Some(Seek {
            to_ms: position_ms,
            at_ms,
            since: Instant::now(),
        });
        self.state.position = Some(PositionAnchor {
            ms: position_ms,
            at_ms,
            playing: self.is_playing(),
        });
        crate::notify(cx);
        send(PlayerCommand::Seek { position_ms }, cx);
    }

    /// `volume` is 0.0..=1.0; the daemon persists it, and setting a level unmutes.
    pub fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        self.state.volume = volume.clamp(0.0, 1.0);
        self.state.muted = false;
        crate::notify(cx);
        send(
            PlayerCommand::SetVolume {
                volume: self.state.volume,
            },
            cx,
        );
    }

    /// Silenced while the level is kept, so unmuting returns to it.
    pub fn is_muted(&self) -> bool {
        self.state.muted
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        let muted = !self.state.muted;
        self.state.muted = muted;
        crate::notify(cx);
        send(PlayerCommand::SetMuted { muted }, cx);
    }

    /// Nudge by `step`, for a wheel over a volume control.
    pub fn nudge_volume(&mut self, step: f32, cx: &mut Context<Self>) {
        self.set_volume(self.volume() + step, cx);
    }

    pub fn set_shuffle(&mut self, shuffle: bool, cx: &mut Context<Self>) {
        self.state.queue.shuffle = shuffle;
        crate::notify(cx);
        send(
            PlayerCommand::SetMode {
                shuffle: Some(shuffle),
                loop_mode: None,
            },
            cx,
        );
    }

    pub fn cycle_loop_mode(&mut self, cx: &mut Context<Self>) {
        let loop_mode = self.state.queue.loop_mode.next();
        self.state.queue.loop_mode = loop_mode;
        crate::notify(cx);
        send(
            PlayerCommand::SetMode {
                shuffle: None,
                loop_mode: Some(loop_mode),
            },
            cx,
        );
    }
}

/// A delayed snapshot must not restore the previous song after a newer stream event.
fn adopt_state(
    current: &mut PlayerState,
    revision: &mut Option<u64>,
    incoming: PlayerState,
) -> bool {
    if revision.is_some_and(|rev| incoming.rev < rev) {
        return false;
    }
    *revision = Some(incoming.rev);
    *current = incoming;
    true
}

fn send(command: PlayerCommand, cx: &App) {
    Link::global(cx).fire(cx, "player command", move |api| async move {
        api.player_command(command).await
    });
}

/// Notifies a few times a second while playback runs; observe it (not [`PlayerModel`]) for anything that tracks the position.
pub struct Playhead {
    playing: bool,
    visible: bool,
    _ticker: Option<Task<()>>,
    _visibility: Option<Subscription>,
    _subscription: Subscription,
}

impl Playhead {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let player = PlayerModel::global(cx);
        let subscription = cx.observe(&player, |this, player, cx| {
            this.playing = player.read(cx).is_playing();
            this.retick(cx);
            crate::notify(cx);
        });
        Self {
            playing: false,
            visible: true,
            _ticker: None,
            _visibility: None,
            _subscription: subscription,
        }
    }

    /// The tick runs only while something is moving and someone can see it.
    fn retick(&mut self, cx: &mut Context<Self>) {
        match (self.playing && self.visible, self._ticker.is_some()) {
            (true, false) => self._ticker = Some(Self::tick(cx)),
            (false, true) => self._ticker = None,
            _ => {}
        }
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.retick(cx);
        // Back on screen the position is up to a tick behind: catch it up rather than wait.
        if visible {
            crate::notify(cx);
        }
    }

    fn tick(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PLAYHEAD_TICK).await;
                let unwatched = this.update(cx, |this, cx| {
                    if this.visible {
                        crate::notify(cx);
                    }
                    this._visibility.is_none()
                });
                match unwatched {
                    Ok(true) => Self::watch_window(&this, cx),
                    Ok(false) => {}
                    Err(_) => return,
                }
            }
        })
    }

    /// Installed on the first tick that finds a window, since the models are built before there is one.
    fn watch_window(this: &gpui_kit::WeakEntity<Self>, cx: &mut gpui_kit::AsyncApp) {
        let Some(playhead) = this.upgrade() else {
            return;
        };
        cx.update(|cx| {
            let Some(handle) = cx.windows().first().copied() else {
                return;
            };
            handle
                .update(cx, |_, window, cx| {
                    playhead.update(cx, |this, cx| {
                        this.visible = window.is_visible();
                        tracing::debug!(visible = this.visible, "playhead watching the window");
                        this._visibility = Some(cx.observe_window_visibility(
                            window,
                            |this, visibility, _, cx| {
                                this.set_visible(visibility.is_visible(), cx);
                            },
                        ));
                    });
                })
                .ok();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seek_at(to_ms: u64, at_ms: u64) -> Seek {
        Seek {
            to_ms,
            at_ms,
            since: Instant::now(),
        }
    }

    #[test]
    fn a_position_from_before_the_seek_is_where_it_used_to_be() {
        let seek = seek_at(180_000, 5_000);
        assert!(!landed(&seek, 12_000, 4_900, Duration::ZERO));
    }

    #[test]
    fn a_position_at_the_target_is_the_seek_landing() {
        let seek = seek_at(180_000, 5_000);
        assert!(landed(&seek, 180_400, 5_100, Duration::ZERO));
        assert!(landed(&seek, 178_600, 5_100, Duration::ZERO));
    }

    #[test]
    fn a_later_stamp_somewhere_else_entirely_is_still_not_the_seek() {
        let seek = seek_at(180_000, 5_000);
        assert!(!landed(&seek, 12_000, 6_000, Duration::ZERO));
    }

    #[test]
    fn a_seek_that_never_lands_gives_the_daemon_the_last_word() {
        let seek = seek_at(180_000, 5_000);
        assert!(landed(&seek, 12_000, 4_900, SEEK_GRACE));
    }

    fn state(rev: u64, title: &str) -> PlayerState {
        PlayerState {
            rev,
            track: Some(TrackInfo {
                title: title.into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn an_old_snapshot_cannot_replace_the_new_song() {
        let mut current = state(12, "new song");
        let mut revision = Some(12);
        assert!(!adopt_state(
            &mut current,
            &mut revision,
            state(11, "old song")
        ));
        assert_eq!(current.track.as_ref().unwrap().title, "new song");
        assert!(adopt_state(
            &mut current,
            &mut revision,
            state(13, "next song")
        ));
        assert_eq!(current.track.as_ref().unwrap().title, "next song");
    }

    #[test]
    fn reconnect_accepts_a_restarted_daemons_lower_revision() {
        let mut current = state(120, "previous daemon");
        let mut revision = None;
        assert!(adopt_state(
            &mut current,
            &mut revision,
            state(1, "restarted daemon")
        ));
        assert_eq!(current.track.as_ref().unwrap().title, "restarted daemon");
    }
}
