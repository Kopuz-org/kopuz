//! Tells the integration plugins that listen to playback what it does, read
//! off the session's player state. Each plugin has a queue of its own and hears
//! every event in order, so a slow one holds up only itself.

use std::collections::HashMap;
use std::sync::Arc;

use api::{ApiEvent, Intent, Phase, PlayerState, TrackInfo};
use plugin_host::manifest::Listen;
use plugin_host::{Integration, wit};
use tokio::sync::broadcast;

use crate::plugin_integrations::PluginIntegrations;
use crate::session::SessionHandle;

/// A jump this far from where the position should be is a seek, not drift.
const SEEK_SLACK_MS: u64 = 1_500;

/// Within this of either end, a jump from the end back to the start is the
/// same track playing again.
const LOOP_SLACK_MS: u64 = 2_000;

/// Events a plugin has not taken yet; past this the newest are dropped.
const QUEUE: usize = 64;

#[derive(Debug, Clone)]
struct Anchor {
    ms: u64,
    at_ms: u64,
    playing: bool,
    rate: f32,
}

impl Anchor {
    fn of(state: &PlayerState) -> Option<Self> {
        state.position.map(|position| Self {
            ms: position.ms,
            at_ms: position.at_ms,
            playing: position.playing,
            rate: state.rate(),
        })
    }

    /// Where playback should be at `at_ms`, had nothing moved it.
    fn expected_at(&self, at_ms: u64) -> u64 {
        if !self.playing {
            return self.ms;
        }
        let elapsed = at_ms.saturating_sub(self.at_ms) as f64 * f64::from(self.rate);
        self.ms + elapsed as u64
    }
}

/// What playback was doing as of the last state, to tell a change from a repeat.
#[derive(Debug, Default)]
pub(crate) struct Follower {
    track: Option<TrackInfo>,
    playing: bool,
    anchor: Option<Anchor>,
    /// The track the last position event was for. A player state can go out
    /// before its track's position does, still carrying the last track's.
    position_token: Option<u64>,
    /// The committed track's token, as of the last player state.
    token: Option<u64>,
    rate: f32,
}

fn now_playing(track: &TrackInfo) -> wit::NowPlaying {
    wit::NowPlaying {
        key: track.key.clone(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        duration_ms: track.duration_ms,
        musicbrainz_recording_id: track.musicbrainz_recording_id.clone(),
        musicbrainz_release_id: track.musicbrainz_release_id.clone(),
    }
}

impl Follower {
    /// What changed since the last state.
    pub(crate) fn follow(&mut self, state: &PlayerState) -> Vec<wit::PlaybackEvent> {
        let active = matches!(state.phase, Phase::Playing | Phase::Paused);
        // Between two tracks, while the next one loads: no stop.
        if !active && matches!(state.intent, Intent::Loading { .. }) && self.track.is_some() {
            return Vec::new();
        }
        let token = match state.intent {
            Intent::Committed { token } => Some(token),
            _ => None,
        };
        let Some(track) = state.track.as_ref().filter(|_| active) else {
            let stopped = self.track.take().is_some();
            *self = Self::default();
            return match stopped {
                true => vec![wit::PlaybackEvent::Stopped],
                false => Vec::new(),
            };
        };
        let playing = state.phase == Phase::Playing;
        // An anchor left over from the track before is no position for this one.
        let stale = matches!((token, self.position_token), (Some(now), Some(seen)) if now != seen);
        let anchor = Anchor::of(state).filter(|_| !stale);
        let position = anchor.as_ref().map_or(0, |anchor| anchor.ms);
        let same = self
            .track
            .as_ref()
            .is_some_and(|held| held.key == track.key);
        let mut events = Vec::new();
        if !same || self.looped(track, anchor.as_ref()) {
            events.extend(self.started(track, position, playing));
        } else if playing != self.playing {
            events.push(match playing {
                true => wit::PlaybackEvent::Resumed(position),
                false => wit::PlaybackEvent::Paused(position),
            });
        } else if self.seeked(anchor.as_ref()) {
            events.push(wit::PlaybackEvent::Seeked(position));
        }
        self.track = Some(track.clone());
        self.playing = playing;
        self.anchor = anchor;
        self.token = token;
        self.rate = state.rate();
        events
    }

    /// A position the session published for the track with `token`: a seek
    /// when it is off the clock, or the first good position of a track whose
    /// state went out before it.
    pub(crate) fn position(
        &mut self,
        token: u64,
        ms: u64,
        at_ms: u64,
        playing: bool,
    ) -> Vec<wit::PlaybackEvent> {
        self.position_token = Some(token);
        if self.track.is_none() || self.token.is_some_and(|current| current != token) {
            return Vec::new();
        }
        let now = Anchor {
            ms,
            at_ms,
            playing,
            rate: if self.rate > 0. { self.rate } else { 1. },
        };
        let moved = match &self.anchor {
            Some(_) => self.seeked(Some(&now)),
            // The track was said to start at 0; a restore picks up further in.
            None => ms > SEEK_SLACK_MS,
        };
        self.anchor = Some(now);
        match moved {
            true => vec![wit::PlaybackEvent::Seeked(ms)],
            false => Vec::new(),
        }
    }

    /// What a plugin that starts listening now needs to catch up.
    pub(crate) fn current(&self) -> Vec<wit::PlaybackEvent> {
        let Some(track) = &self.track else {
            return Vec::new();
        };
        let position = self.anchor.as_ref().map_or(0, |anchor| anchor.ms);
        self.started(track, position, self.playing)
    }

    fn started(
        &self,
        track: &TrackInfo,
        position_ms: u64,
        playing: bool,
    ) -> Vec<wit::PlaybackEvent> {
        let mut events = vec![wit::PlaybackEvent::Started(wit::Started {
            track: now_playing(track),
            position_ms,
        })];
        if !playing {
            events.push(wit::PlaybackEvent::Paused(position_ms));
        }
        events
    }

    fn seeked(&self, anchor: Option<&Anchor>) -> bool {
        let (Some(before), Some(now)) = (self.anchor.as_ref(), anchor) else {
            return false;
        };
        before.expected_at(now.at_ms).abs_diff(now.ms) > SEEK_SLACK_MS
    }

    /// The same track back at its start from its end, as a repeated one is.
    fn looped(&self, track: &TrackInfo, anchor: Option<&Anchor>) -> bool {
        let (Some(before), Some(now), Some(duration)) =
            (self.anchor.as_ref(), anchor, track.duration_ms)
        else {
            return false;
        };
        let expected = before.expected_at(now.at_ms);
        expected + LOOP_SLACK_MS >= duration && now.ms <= LOOP_SLACK_MS
    }
}

/// One plugin's events of one kind, delivered in order by a task of its own.
/// A plugin that falls behind loses its oldest waiting events, never the
/// newest, so what it hears last is how things stand now.
pub(crate) struct Queue<E> {
    pub(crate) integration: Arc<Integration>,
    waiting: Arc<std::sync::Mutex<std::collections::VecDeque<E>>>,
    ready: Arc<tokio::sync::Notify>,
    /// Cleared when the queue goes, so its worker ends with it.
    open: Arc<std::sync::atomic::AtomicBool>,
    capacity: usize,
    what: &'static str,
}

impl<E: Send + 'static> Queue<E> {
    pub(crate) fn open<F, Fut>(
        integration: Arc<Integration>,
        capacity: usize,
        what: &'static str,
        deliver: F,
    ) -> Self
    where
        F: Fn(Arc<Integration>, E) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), plugin_host::PluginError>> + Send,
    {
        let waiting = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        let ready = Arc::new(tokio::sync::Notify::new());
        let open = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let worker = Arc::clone(&integration);
        let (taken, woken, kept) = (Arc::clone(&waiting), Arc::clone(&ready), Arc::clone(&open));
        tokio::spawn(async move {
            loop {
                let next = taken
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .pop_front();
                let Some(event) = next else {
                    // Replaced or turned off: no one queues for it any more.
                    if worker.is_closed() || !kept.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    woken.notified().await;
                    continue;
                };
                if worker.is_closed() {
                    break;
                }
                if let Err(error) = deliver(Arc::clone(&worker), event).await {
                    tracing::warn!(plugin = %worker.plugin().id(), %error, what, "an integration was not told");
                }
            }
        });
        Self {
            integration,
            waiting,
            ready,
            open,
            capacity: capacity.max(1),
            what,
        }
    }

    /// Queue `event`, dropping the oldest waiting one when the plugin is this far behind.
    pub(crate) fn send(&self, event: E) {
        let mut waiting = self
            .waiting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if waiting.len() >= self.capacity {
            waiting.pop_front();
            tracing::debug!(plugin = %self.integration.plugin().id(), what = self.what, "an integration is behind; its oldest event was dropped");
        }
        waiting.push_back(event);
        drop(waiting);
        self.ready.notify_one();
    }
}

impl<E> Drop for Queue<E> {
    /// Wake the worker so it sees its integration closed and ends, rather than waiting forever.
    fn drop(&mut self) {
        self.open.store(false, std::sync::atomic::Ordering::Release);
        self.ready.notify_one();
    }
}

/// The running integrations that listen to `what`, with their queues kept in
/// step: one turned off, reopened or swapped for a new build gets a new queue.
pub(crate) fn listening<E>(
    plugins: &PluginIntegrations,
    queues: &mut HashMap<String, Queue<E>>,
    what: Listen,
) -> Vec<Arc<Integration>> {
    let listening: Vec<Arc<Integration>> = plugins
        .running()
        .into_iter()
        .filter(|integration| listens(integration, what))
        .collect();
    queues.retain(|id, queue| {
        listening.iter().any(|integration| {
            integration.plugin().id() == id && Arc::ptr_eq(integration, &queue.integration)
        })
    });
    listening
}

fn listens(integration: &Integration, what: Listen) -> bool {
    integration
        .plugin()
        .manifest()
        .integration
        .as_ref()
        .is_some_and(|spec| spec.listens_to(what))
}

fn playback_queue(integration: Arc<Integration>) -> Queue<wit::PlaybackEvent> {
    Queue::open(
        integration,
        QUEUE,
        "playback",
        |integration, event| async move { integration.on_playback(&event).await },
    )
}

/// Follow the session's player state for as long as it runs.
pub fn spawn(
    session: &SessionHandle,
    plugins: Arc<PluginIntegrations>,
) -> tokio::task::JoinHandle<()> {
    let mut events = session.subscribe();
    let lagged = session.clone();
    tokio::spawn(async move {
        let mut follower = Follower::default();
        let mut queues: HashMap<String, Queue<wit::PlaybackEvent>> = HashMap::new();
        loop {
            let happened = match events.recv().await {
                Ok(ApiEvent::PlayerState(state)) => follower.follow(&state),
                Ok(ApiEvent::PlayerPosition {
                    token,
                    position_ms,
                    at_ms,
                    playing,
                }) => follower.position(token, position_ms, at_ms, playing),
                // Behind on events: what playback is doing now says what was missed.
                Err(broadcast::error::RecvError::Lagged(_)) => follower.follow(&lagged.state()),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            if happened.is_empty() {
                continue;
            }
            for integration in listening(&plugins, &mut queues, Listen::Playback) {
                let id = integration.plugin().id().to_string();
                let queue = queues.entry(id).or_insert_with(|| {
                    // One that just started listening hears what it missed of this track first.
                    let queue = playback_queue(integration);
                    let caught_up = &happened;
                    for event in follower.current() {
                        if !caught_up.contains(&event) {
                            queue.send(event);
                        }
                    }
                    queue
                });
                for event in &happened {
                    queue.send(event.clone());
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::PositionAnchor;

    fn track(key: &str) -> TrackInfo {
        TrackInfo {
            key: key.into(),
            title: format!("{key} title"),
            artist: "Ada".into(),
            album: "First".into(),
            duration_ms: Some(200_000),
            ..Default::default()
        }
    }

    fn state(phase: Phase, key: Option<&str>, ms: u64, at_ms: u64) -> PlayerState {
        PlayerState {
            phase,
            track: key.map(track),
            position: Some(PositionAnchor {
                ms,
                at_ms,
                playing: phase == Phase::Playing,
            }),
            speed: 1.0,
            ..Default::default()
        }
    }

    fn started_at(events: &[wit::PlaybackEvent]) -> Vec<(String, u64)> {
        events
            .iter()
            .filter_map(|event| match event {
                wit::PlaybackEvent::Started(started) => {
                    Some((started.track.key.clone(), started.position_ms))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_track_starts_pauses_resumes_and_stops() {
        let mut follower = Follower::default();
        let first = follower.follow(&state(Phase::Playing, Some("a"), 0, 1_000));
        assert_eq!(started_at(&first), [("a".to_string(), 0)]);
        assert_eq!(first.len(), 1);

        // A repeat of the same state, as progress ticks are, says nothing.
        assert!(
            follower
                .follow(&state(Phase::Playing, Some("a"), 5_000, 6_000))
                .is_empty()
        );

        assert_eq!(
            follower.follow(&state(Phase::Paused, Some("a"), 9_000, 10_000)),
            [wit::PlaybackEvent::Paused(9_000)]
        );
        assert_eq!(
            follower.follow(&state(Phase::Playing, Some("a"), 9_000, 60_000)),
            [wit::PlaybackEvent::Resumed(9_000)]
        );
        assert_eq!(
            follower.follow(&state(Phase::Idle, None, 0, 70_000)),
            [wit::PlaybackEvent::Stopped]
        );
        assert!(
            follower
                .follow(&state(Phase::Idle, None, 0, 80_000))
                .is_empty()
        );
    }

    #[test]
    fn a_jump_off_the_clock_is_a_seek_and_drift_is_not() {
        let mut follower = Follower::default();
        follower.follow(&state(Phase::Playing, Some("a"), 0, 0));
        assert!(
            follower
                .follow(&state(Phase::Playing, Some("a"), 10_900, 10_000))
                .is_empty()
        );
        assert_eq!(
            follower.follow(&state(Phase::Playing, Some("a"), 90_000, 11_000)),
            [wit::PlaybackEvent::Seeked(90_000)]
        );
        // While paused the position should not move at all.
        follower.follow(&state(Phase::Paused, Some("a"), 95_000, 16_000));
        assert_eq!(
            follower.follow(&state(Phase::Paused, Some("a"), 30_000, 40_000)),
            [wit::PlaybackEvent::Seeked(30_000)]
        );
    }

    #[test]
    fn the_next_track_and_a_repeated_one_each_start_again() {
        let mut follower = Follower::default();
        follower.follow(&state(Phase::Playing, Some("a"), 0, 0));
        let next = follower.follow(&state(Phase::Playing, Some("b"), 0, 200_000));
        assert_eq!(started_at(&next), [("b".to_string(), 0)]);
        let again = follower.follow(&state(Phase::Playing, Some("b"), 300, 400_100));
        assert_eq!(started_at(&again), [("b".to_string(), 300)]);
    }

    fn committed(mut state: PlayerState, token: u64) -> PlayerState {
        state.intent = Intent::Committed { token };
        state
    }

    #[test]
    fn a_state_carrying_the_last_tracks_position_starts_the_next_at_its_own() {
        let mut follower = Follower::default();
        follower.follow(&committed(state(Phase::Playing, Some("a"), 0, 0), 1));
        follower.position(1, 0, 0, true);
        // Gapless into "b": its state goes out still holding "a"'s position.
        let next = follower.follow(&committed(
            state(Phase::Playing, Some("b"), 199_000, 199_000),
            2,
        ));
        assert_eq!(started_at(&next), [("b".to_string(), 0)]);
        assert!(
            follower.position(2, 0, 199_050, true).is_empty(),
            "its own position is no seek"
        );
        assert_eq!(
            follower.position(2, 60_000, 200_000, true),
            [wit::PlaybackEvent::Seeked(60_000)]
        );
    }

    #[test]
    fn loading_the_next_track_is_no_stop() {
        let mut follower = Follower::default();
        follower.follow(&state(Phase::Playing, Some("a"), 0, 0));
        let mut loading = state(Phase::Idle, None, 0, 1_000);
        loading.intent = Intent::Loading {
            token: 2,
            from_token: Some(1),
        };
        assert!(follower.follow(&loading).is_empty());
        assert_eq!(
            started_at(&follower.follow(&state(Phase::Playing, Some("b"), 0, 2_000))),
            [("b".to_string(), 0)]
        );
    }

    #[test]
    fn a_paused_track_starts_paused_and_a_late_listener_catches_up() {
        let mut follower = Follower::default();
        let restored = follower.follow(&state(Phase::Paused, Some("a"), 42_000, 0));
        assert_eq!(started_at(&restored), [("a".to_string(), 42_000)]);
        assert_eq!(restored[1], wit::PlaybackEvent::Paused(42_000));
        assert_eq!(follower.current(), restored);
        follower.follow(&state(Phase::Idle, None, 0, 1_000));
        assert!(follower.current().is_empty());
    }
}
