//! Smart crossfades: where a fade belongs between two tracks, and where it does not.

use std::time::Duration;

use player::engine::{DeckMix, MixCurves, MixError, MixFilters};
use reader::Track;
use server::audiomuse::{Curves, TransitionPlan};

/// Silence only ends a track early this close to its end. Further out it is a
/// quiet intro or a pause, and the music resumes.
const TAIL_WINDOW: Duration = Duration::from_secs(30);

const BEATS_PER_BAR: f64 = 4.0;
const SECONDS_PER_MINUTE: f64 = 60.0;
const MILLIS_PER_SECOND: f64 = 1000.0;
/// Tempos further apart than this (faster / slower - 1) clash, so they overlap for half as long.
const TEMPO_CLASH: f64 = 0.12;

/// Before a planned mix the next track opens: room for a stream to resolve and probe.
pub(super) const MIX_PRELOAD: Duration = Duration::from_secs(10);
/// A plan armed closer than this to its start may find the decoder already past it.
const MIX_MARGIN: Duration = Duration::from_secs(3);

/// Whether `position` is inside the last `TAIL_WINDOW` of a track of `duration`.
pub(super) fn in_tail(duration: Duration, position: Duration) -> bool {
    !duration.is_zero() && position < duration && duration - position <= TAIL_WINDOW
}

/// The next track picks up where this one stops on the same album, so a fade would cut
/// into a live set or a mixed record. `1 → 2` continues; `1 → 3`, another disc or
/// another album does not.
pub(super) fn continues_album(current: &Track, next: &Track) -> bool {
    if current.album_id.is_empty() || current.album_id != next.album_id {
        return false;
    }
    if current.disc_number.unwrap_or(1) != next.disc_number.unwrap_or(1) {
        return false;
    }
    match (current.track_number, next.track_number) {
        (Some(this), Some(following)) => following == this + 1,
        _ => false,
    }
}

/// The crossfade length that ends on a bar line of the slower of two tempos (BPM).
/// 125 BPM is 1.92 s a bar, so 6 s fit 3 bars = 5.76 s. Never longer than
/// `configured`; tempos that clash get half of it before snapping.
pub(super) fn bar_fade(configured: Duration, tempos: Option<(f32, f32)>) -> Duration {
    let Some((from, to)) = tempos else {
        return configured;
    };
    let heard = |tempo: f32| tempo.is_finite() && tempo > 0.0;
    if !(heard(from) && heard(to)) {
        return configured;
    }
    let (slower, faster) = (f64::from(from.min(to)), f64::from(from.max(to)));

    let bar = BEATS_PER_BAR * SECONDS_PER_MINUTE / slower;
    let mut target = configured.as_secs_f64();
    if faster / slower - 1.0 > TEMPO_CLASH {
        target /= 2.0;
    }
    let bars = (target / bar).floor().max(1.0);
    Duration::from_millis((bars * bar * MILLIS_PER_SECOND).round() as u64).min(configured)
}

/// What to do with a planned mix at a playback position of the outgoing track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MixArming {
    /// Too early to open the next track.
    Wait,
    Arm,
    /// Too close to the start for the plan to be installed in time.
    Drop,
}

/// Arm `MIX_PRELOAD` before the plan's `start`, or at once when that has passed
/// but `MIX_MARGIN` is still left.
pub(super) fn plan_arming(position: Duration, start: Duration) -> MixArming {
    if position + MIX_MARGIN > start {
        return MixArming::Drop;
    }
    if position + MIX_PRELOAD < start {
        return MixArming::Wait;
    }
    MixArming::Arm
}

/// An AudioMuse-AI transition in the engine's terms.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlannedMix {
    /// Where in the outgoing track the mix begins.
    pub start: Duration,
    pub length: Duration,
    /// Where in the incoming track it enters.
    pub entry: Duration,
    pub outgoing: DeckMix,
    pub incoming: DeckMix,
}

pub(super) fn planned_mix(plan: &TransitionPlan) -> Result<PlannedMix, MixError> {
    let seconds = |value: f64| Duration::try_from_secs_f64(value).map_err(|_| MixError::Value);
    let (step, length) = (seconds(plan.step)?, seconds(plan.duration)?);
    let filters = MixFilters {
        high_hz: plan.filters.high_hz as f32,
        mid_hz: plan.filters.mid_hz as f32,
        mid_q: plan.filters.mid_q as f32,
        bass_hz: plan.filters.bass_hz as f32,
    };
    let highpass = plan.to.highpass_hz.as_deref().map(narrow);

    Ok(PlannedMix {
        start: seconds(plan.start)?,
        length,
        entry: seconds(plan.to.start)?,
        outgoing: DeckMix::new(mix_curves(&plan.from.curves, None), filters, step, length)?,
        incoming: DeckMix::new(mix_curves(&plan.to.curves, highpass), filters, step, length)?,
    })
}

fn mix_curves(curves: &Curves, highpass_hz: Option<Vec<f32>>) -> MixCurves {
    MixCurves {
        speed: narrow(&curves.speed),
        volume: narrow(&curves.volume),
        high_db: narrow(&curves.high_db),
        mid_db: narrow(&curves.mid_db),
        bass_db: narrow(&curves.bass_db),
        highpass_hz,
    }
}

fn narrow(values: &[f64]) -> Vec<f32> {
    values.iter().map(|value| *value as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(album: &str, disc: Option<u32>, number: Option<u32>) -> Track {
        Track {
            id: reader::TrackId::Local(std::path::PathBuf::from("/t")),
            cover: None,
            album_id: album.into(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration: 0,
            khz: 0,
            bitrate: 0,
            bit_depth: 0,
            track_number: number,
            disc_number: disc,
            musicbrainz_release_id: None,
            musicbrainz_recording_id: None,
            musicbrainz_track_id: None,
            playlist_item_id: None,
            credits: Vec::new(),
            artists: Vec::new(),
            replay_gain: config::ReplayGainInfo::default(),
            explicit: false,
            plays: None,
            counterpart: None,
        }
    }

    #[test]
    fn silence_ends_a_track_only_inside_its_tail() {
        let duration = Duration::from_secs(240);

        assert!(in_tail(duration, Duration::from_secs(215)), "tail");
        assert!(in_tail(duration, Duration::from_secs(210)), "edge");
        assert!(
            !in_tail(duration, Duration::from_secs(209)),
            "a pause mid-track"
        );
        assert!(!in_tail(duration, Duration::from_secs(3)), "a quiet intro");
        assert!(!in_tail(duration, duration), "past the end");
        assert!(!in_tail(Duration::ZERO, Duration::ZERO), "unknown length");
    }

    #[test]
    fn only_the_following_track_of_the_same_disc_continues_an_album() {
        let first = track("a", Some(1), Some(1));

        assert!(continues_album(&first, &track("a", Some(1), Some(2))));
        assert!(
            continues_album(&first, &track("a", None, Some(2))),
            "no disc is disc 1"
        );
        assert!(
            !continues_album(&first, &track("a", Some(1), Some(3))),
            "a skipped track"
        );
        assert!(
            !continues_album(&first, &track("a", Some(2), Some(2))),
            "another disc"
        );
        assert!(
            !continues_album(&first, &track("b", Some(1), Some(2))),
            "another album"
        );
        assert!(
            !continues_album(&first, &track("a", Some(1), None)),
            "no number"
        );
        assert!(
            !continues_album(&track("", None, Some(1)), &track("", None, Some(2))),
            "no album"
        );
    }

    const TEMPO: f32 = 125.0;
    /// One bar at `TEMPO`: 4 beats of 0.48 s.
    const BAR: Duration = Duration::from_millis(1920);

    #[test]
    fn a_crossfade_ends_on_a_bar_of_the_slower_tempo() {
        let six = Duration::from_secs(6);

        assert_eq!(
            bar_fade(six, Some((TEMPO, 130.0))),
            BAR * 3,
            "3 bars fit in 6 s"
        );
        assert_eq!(bar_fade(six, Some((130.0, TEMPO))), BAR * 3, "either order");
    }

    #[test]
    fn clashing_tempos_fade_for_half_as_long() {
        let six = Duration::from_secs(6);

        assert_eq!(
            bar_fade(six, Some((TEMPO, 150.0))),
            BAR,
            "1 bar fits in 3 s"
        );
    }

    #[test]
    fn without_a_tempo_the_configured_length_stands() {
        let six = Duration::from_secs(6);

        assert_eq!(bar_fade(six, None), six);
        assert_eq!(bar_fade(six, Some((0.0, TEMPO))), six, "no tempo heard");
        assert_eq!(bar_fade(six, Some((f32::NAN, TEMPO))), six);
    }

    #[test]
    fn a_short_crossfade_keeps_one_bar_but_never_grows() {
        let one = Duration::from_secs(1);
        let three = Duration::from_secs(3);

        assert_eq!(bar_fade(one, Some((TEMPO, TEMPO))), one, "capped");
        assert_eq!(bar_fade(three, Some((TEMPO, TEMPO))), BAR, "one bar");
    }

    #[test]
    fn a_plan_arms_its_preload_ahead_of_the_mix() {
        let start = Duration::from_secs(240);
        let at = |seconds| plan_arming(Duration::from_secs(seconds), start);

        assert_eq!(at(200), MixArming::Wait);
        assert_eq!(at(229), MixArming::Wait);
        assert_eq!(at(230), MixArming::Arm, "preload reached");
        assert_eq!(at(236), MixArming::Arm, "late, still in the margin");
        assert_eq!(at(237), MixArming::Arm, "the margin's edge");
        assert_eq!(at(238), MixArming::Drop, "too late to install");
        assert_eq!(at(250), MixArming::Drop, "past the start");
    }

    /// AudioMuse-AI's answer for two real songs.
    const FIXTURE: &str = include_str!("../../../server/tests/fixtures/audiomuse-transition.json");

    #[derive(serde::Deserialize)]
    struct Fixture {
        start: f64,
        duration: f64,
        middle: f64,
        step: f64,
        filters: server::audiomuse::BandFilters,
        from: server::audiomuse::OutgoingDeck,
        to: server::audiomuse::IncomingDeck,
    }

    fn fixture_plan() -> TransitionPlan {
        let raw: Fixture = serde_json::from_str(FIXTURE).expect("fixture parses");
        TransitionPlan {
            kind: server::audiomuse::TransitionKind::Crossfade,
            start: raw.start,
            duration: raw.duration,
            middle: raw.middle,
            step: raw.step,
            filters: raw.filters,
            from: raw.from,
            to: raw.to,
        }
    }

    #[test]
    fn a_server_plan_becomes_two_engine_decks() {
        let mix = planned_mix(&fixture_plan()).expect("the plan converts");

        assert_eq!(mix.start, Duration::from_secs_f64(244.2765));
        assert_eq!(mix.length, Duration::from_secs_f64(10.2136));
        assert_eq!(mix.entry, Duration::from_secs_f64(8.8373));
        assert_eq!(mix.outgoing.length(), mix.length);
        assert_eq!(mix.incoming.length(), mix.length);
    }

    #[test]
    fn a_plan_the_engine_cannot_play_converts_to_an_error() {
        let mut plan = fixture_plan();
        plan.from.curves.speed[0] = 0.0;
        assert_eq!(planned_mix(&plan), Err(MixError::Value), "a stopped deck");

        let mut plan = fixture_plan();
        plan.start = -1.0;
        assert_eq!(planned_mix(&plan), Err(MixError::Value), "before the song");
    }
}
