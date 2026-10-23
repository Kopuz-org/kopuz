//! Home's "For you" row: the active source's mix, asked for once the row is on screen.

use api::{ApiError, ErrorCode, ForYou, TrackInfo};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use link::Link;

use crate::invalidation::{Invalidation, TablesChanged};

/// What the row shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ForYouState {
    /// Nobody has asked since the last resync or source switch.
    #[default]
    Idle,
    Loading,
    Mix(Vec<TrackInfo>),
    /// The source has no likes or no track radio, so the row is hidden.
    Unavailable,
    Failed(ApiError),
    /// The daemon went away before the mix came, so there is nothing to wait for.
    Disconnected,
}

impl ForYouState {
    /// Whether Home draws the row at all.
    pub fn is_shown(&self) -> bool {
        match self {
            Self::Unavailable | Self::Disconnected => false,
            Self::Mix(tracks) => !tracks.is_empty(),
            Self::Idle | Self::Loading | Self::Failed(_) => true,
        }
    }
}

/// The transitions, apart from GPUI so they can be tested.
#[derive(Debug, Default)]
struct Machine {
    state: ForYouState,
    generation: u64,
    // The last mix shown, kept on screen through a resync or a lost daemon rather than collapsing the row.
    held: Vec<TrackInfo>,
}

impl Machine {
    fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.state = ForYouState::Loading;
        self.generation
    }

    /// Take an answer, unless a newer ask or a reset has overtaken it.
    fn settle(&mut self, generation: u64, result: Result<ForYou, ApiError>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.state = match result {
            Ok(ForYou::Mix(tracks)) => {
                self.held = tracks.clone();
                ForYouState::Mix(tracks)
            }
            Ok(ForYou::Unavailable(_)) => ForYouState::Unavailable,
            // A daemon from before For you answers every ask this way.
            Err(error) if error.code == ErrorCode::Unsupported => ForYouState::Unavailable,
            Err(error) => ForYouState::Failed(error),
        };
        true
    }

    /// The daemon dropped: a mix still on its way never arrives, so stop waiting for it.
    fn disconnect(&mut self) -> bool {
        if !matches!(self.state, ForYouState::Idle | ForYouState::Loading) {
            return false;
        }
        self.generation += 1;
        self.state = ForYouState::Disconnected;
        true
    }

    /// The mix to draw: the one in, or the last one while another is on its way or the daemon is gone.
    fn shown_mix(&self) -> Option<&[TrackInfo]> {
        match &self.state {
            ForYouState::Mix(tracks) => Some(tracks),
            ForYouState::Idle | ForYouState::Loading | ForYouState::Disconnected
                if !self.held.is_empty() =>
            {
                Some(&self.held)
            }
            _ => None,
        }
    }

    /// A refresh or another source: the held mix is no answer to either.
    fn forget(&mut self) {
        self.held.clear();
    }

    fn reset(&mut self) {
        self.generation += 1;
        self.state = ForYouState::Idle;
    }
}

pub struct ForYouModel {
    machine: Machine,
    // The source the held mix came from, so a switch drops it.
    source: Option<String>,
    _fetch: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ForYouModel {
    pub fn global(cx: &App) -> Entity<Self> {
        crate::handle(cx)
    }

    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let invalidation = Invalidation::global(cx);
        let connection = Link::connection(cx);
        let subscriptions = vec![
            cx.subscribe(&invalidation, |this, _, event: &TablesChanged, cx| {
                if event.is_all() {
                    this.reset(cx);
                }
            }),
            cx.observe(&connection, |this, connection, cx| {
                let connected = connection.read(cx).is_connected();
                if !connected && this.machine.disconnect() {
                    this._fetch = None;
                    crate::notify(cx);
                } else if connected && this.machine.state == ForYouState::Disconnected {
                    this.reset(cx);
                }
            }),
        ];
        Self {
            machine: Machine::default(),
            source: None,
            _fetch: None,
            _subscriptions: subscriptions,
        }
    }

    /// The mix the row draws, held over a reload; `None` while there is none to show.
    pub fn shown_mix(&self) -> Option<&[TrackInfo]> {
        self.machine.shown_mix()
    }

    /// Whether Home draws the row at all.
    pub fn is_shown(&self) -> bool {
        self.machine.state.is_shown() || self.shown_mix().is_some()
    }

    pub fn state(&self) -> &ForYouState {
        &self.machine.state
    }

    /// Ask for the mix unless it is held or on its way; the daemon caches it, so this is cheap.
    pub fn ensure(&mut self, cx: &mut Context<Self>) {
        if self.machine.state == ForYouState::Idle && Link::connection(cx).read(cx).is_connected() {
            self.fetch(false, cx);
        }
    }

    /// Throw the mix away and have the daemon build another.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.machine.forget();
        self.fetch(true, cx);
    }

    fn fetch(&mut self, fresh: bool, cx: &mut Context<Self>) {
        self.source = crate::SourcesModel::global(cx)
            .read(cx)
            .active()
            .map(|source| source.id.clone());
        let generation = self.machine.begin();
        let call = Link::global(cx).call(move |api| async move {
            match fresh {
                true => api.refresh_for_you().await,
                false => api.for_you().await,
            }
        });
        let first = crate::FirstRead::begin(cx);
        crate::notify(cx);
        self._fetch = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            let _first = first;
            this.update(cx, |this, cx| {
                if let Err(error) = &result {
                    tracing::debug!(%error, "for you failed");
                }
                if this.machine.settle(generation, result) {
                    crate::notify(cx);
                }
            })
            .ok();
        }));
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        let active = crate::SourcesModel::global(cx)
            .read(cx)
            .active()
            .map(|source| source.id.clone());
        if active != self.source {
            self.source = active;
            self.machine.forget();
        }
        self.machine.reset();
        // Nobody can ask while the daemon is gone, so an idle row would pulse forever.
        if !Link::connection(cx).read(cx).is_connected() {
            self.machine.disconnect();
        }
        self._fetch = None;
        crate::notify(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::ForYouUnavailable;

    fn track(key: &str) -> TrackInfo {
        TrackInfo {
            key: key.into(),
            ..Default::default()
        }
    }

    fn error() -> ApiError {
        ApiError::new(ErrorCode::SourceUnreachable, "radio down")
    }

    #[test]
    fn a_fresh_row_waits_to_be_asked() {
        let machine = Machine::default();
        assert_eq!(machine.state, ForYouState::Idle);
        assert!(machine.state.is_shown());
    }

    #[test]
    fn loading_fills_with_the_mix() {
        let mut machine = Machine::default();
        let generation = machine.begin();
        assert_eq!(machine.state, ForYouState::Loading);
        assert!(machine.settle(generation, Ok(ForYou::Mix(vec![track("a"), track("b")]))));
        assert_eq!(
            machine.state,
            ForYouState::Mix(vec![track("a"), track("b")])
        );
        assert!(machine.state.is_shown());
    }

    #[test]
    fn nothing_to_mix_from_hides_the_row() {
        for reason in [ForYouUnavailable::NoLikes, ForYouUnavailable::NoTrackRadio] {
            let mut machine = Machine::default();
            let generation = machine.begin();
            machine.settle(generation, Ok(ForYou::Unavailable(reason)));
            assert_eq!(machine.state, ForYouState::Unavailable);
            assert!(!machine.state.is_shown());
        }
    }

    #[test]
    fn an_empty_mix_hides_the_row() {
        let mut machine = Machine::default();
        let generation = machine.begin();
        machine.settle(generation, Ok(ForYou::Mix(Vec::new())));
        assert!(!machine.state.is_shown());
    }

    #[test]
    fn a_failure_keeps_the_row_to_say_so() {
        let mut machine = Machine::default();
        let generation = machine.begin();
        machine.settle(generation, Err(error()));
        assert_eq!(machine.state, ForYouState::Failed(error()));
        assert!(machine.state.is_shown());
    }

    #[test]
    fn a_daemon_without_for_you_hides_the_row() {
        for after_a_mix in [false, true] {
            let mut machine = Machine::default();
            if after_a_mix {
                let first = machine.begin();
                machine.settle(first, Ok(ForYou::Mix(vec![track("a")])));
            }
            let generation = machine.begin();
            machine.settle(generation, Err(ApiError::new(ErrorCode::Unsupported, "")));
            assert_eq!(machine.state, ForYouState::Unavailable);
            assert!(!machine.state.is_shown());
        }
    }

    #[test]
    fn a_refresh_shows_the_placeholder_and_drops_the_older_answer() {
        let mut machine = Machine::default();
        let first = machine.begin();
        machine.settle(first, Ok(ForYou::Mix(vec![track("a")])));
        let second = machine.begin();
        assert_eq!(machine.state, ForYouState::Loading);
        assert!(!machine.settle(first, Ok(ForYou::Mix(vec![track("old")]))));
        assert_eq!(machine.state, ForYouState::Loading);
        assert!(machine.settle(second, Ok(ForYou::Mix(vec![track("b")]))));
        assert_eq!(machine.state, ForYouState::Mix(vec![track("b")]));
    }

    #[test]
    fn a_refresh_after_a_failure_can_still_fill_the_row() {
        let mut machine = Machine::default();
        let first = machine.begin();
        machine.settle(first, Err(error()));
        let second = machine.begin();
        machine.settle(second, Ok(ForYou::Mix(vec![track("a")])));
        assert_eq!(machine.state, ForYouState::Mix(vec![track("a")]));
    }

    #[test]
    fn a_lost_daemon_stops_the_placeholder_and_drops_the_answer_in_flight() {
        let mut machine = Machine::default();
        let generation = machine.begin();
        assert!(machine.disconnect());
        assert_eq!(machine.state, ForYouState::Disconnected);
        assert!(!machine.state.is_shown());
        assert!(!machine.settle(generation, Ok(ForYou::Mix(vec![track("a")]))));
    }

    #[test]
    fn a_lost_daemon_keeps_a_mix_already_shown() {
        let mut machine = Machine::default();
        let generation = machine.begin();
        machine.settle(generation, Ok(ForYou::Mix(vec![track("a")])));
        assert!(!machine.disconnect());
        assert_eq!(machine.state, ForYouState::Mix(vec![track("a")]));
    }

    #[test]
    fn a_resync_forgets_the_mix_and_ignores_the_answer_in_flight() {
        let mut machine = Machine::default();
        let generation = machine.begin();
        machine.reset();
        assert_eq!(machine.state, ForYouState::Idle);
        assert!(!machine.settle(generation, Ok(ForYou::Mix(vec![track("a")]))));
        assert_eq!(machine.state, ForYouState::Idle);
    }

    #[test]
    fn a_reload_or_a_lost_daemon_keeps_the_last_mix_on_screen() {
        let mut machine = Machine::default();
        let generation = machine.begin();
        machine.settle(generation, Ok(ForYou::Mix(vec![track("a")])));
        machine.reset();
        assert_eq!(machine.shown_mix(), Some(&[track("a")][..]));
        machine.begin();
        assert_eq!(machine.shown_mix(), Some(&[track("a")][..]));
        assert!(machine.disconnect());
        assert_eq!(machine.shown_mix(), Some(&[track("a")][..]));
    }

    #[test]
    fn a_refresh_or_a_new_source_lets_the_mix_go() {
        let mut machine = Machine::default();
        let generation = machine.begin();
        machine.settle(generation, Ok(ForYou::Mix(vec![track("a")])));
        machine.forget();
        machine.begin();
        assert_eq!(machine.shown_mix(), None);
    }

    #[test]
    fn nothing_held_until_a_mix_has_come() {
        let mut machine = Machine::default();
        machine.begin();
        assert_eq!(machine.shown_mix(), None);
    }
}
