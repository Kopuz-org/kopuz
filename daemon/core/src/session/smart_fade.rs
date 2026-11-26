//! Smart crossfades: where a fade belongs between two tracks, and where it does not.

use std::time::Duration;

use player::engine::{DeckMix, MixCurves, MixError, MixFilters};
use reader::Track;

use crate::transition::{ASK_BEFORE, DeckCurves, LATEST_BEFORE, TransitionPlan};

/// Silence only ends a track early this close to its end. Further out it is a
/// quiet intro or a pause, and the music resumes.
const TAIL_WINDOW: Duration = Duration::from_secs(30);

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

/// What to do with a planned mix at a playback position of the outgoing track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MixArming {
    /// Too early to open the next track.
    Wait,
    Arm,
    /// Too close to the start for the plan to be installed in time.
    Drop,
}

/// Arm `ASK_BEFORE` the plan's `start`, which leaves the next track time to
/// open, or at once when that has passed but `LATEST_BEFORE` is still left:
/// closer in, the decoder may already be past the start.
pub(super) fn plan_arming(position: Duration, start: Duration) -> MixArming {
    if position + LATEST_BEFORE > start {
        return MixArming::Drop;
    }
    if position + ASK_BEFORE < start {
        return MixArming::Wait;
    }
    MixArming::Arm
}

/// A plugin's transition in the engine's terms.
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
    let (step, length) = (seconds(plan.step)?, seconds(plan.length)?);
    let bands = plan.bands;
    let filters = MixFilters {
        high_hz: bands.high_hz,
        mid_hz: bands.mid_hz,
        mid_q: bands.mid_q,
        bass_hz: bands.bass_hz,
    };

    Ok(PlannedMix {
        start: seconds(plan.start)?,
        length,
        entry: seconds(plan.entry)?,
        outgoing: DeckMix::new(mix_curves(&plan.outgoing), filters, step, length)?,
        incoming: DeckMix::new(mix_curves(&plan.incoming), filters, step, length)?,
    })
}

fn mix_curves(curves: &DeckCurves) -> MixCurves {
    MixCurves {
        speed: curves.speed.clone(),
        volume: curves.volume.clone(),
        high_db: curves.high_db.clone(),
        mid_db: curves.mid_db.clone(),
        bass_db: curves.bass_db.clone(),
        highpass_hz: curves.highpass_hz.clone(),
    }
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

    fn flat(points: usize) -> DeckCurves {
        DeckCurves {
            volume: vec![1.0; points],
            speed: vec![1.0; points],
            high_db: vec![0.0; points],
            mid_db: vec![0.0; points],
            bass_db: vec![0.0; points],
            highpass_hz: None,
        }
    }

    /// Four seconds at a quarter second a point, from 180 s into the outgoing track.
    fn plugin_plan() -> TransitionPlan {
        TransitionPlan {
            start: 180.0,
            length: 4.0,
            entry: 2.5,
            step: 0.25,
            bands: crate::transition::Bands {
                high_hz: 4_000.0,
                mid_hz: 1_000.0,
                mid_q: 0.7,
                bass_hz: 120.0,
            },
            outgoing: flat(17),
            incoming: flat(17),
        }
    }

    #[test]
    fn a_plugin_plan_becomes_two_engine_decks() {
        let mix = planned_mix(&plugin_plan()).expect("the plan converts");

        assert_eq!(mix.start, Duration::from_secs(180));
        assert_eq!(mix.length, Duration::from_secs(4));
        assert_eq!(mix.entry, Duration::from_millis(2_500));
        assert_eq!(mix.outgoing.length(), mix.length);
        assert_eq!(mix.incoming.length(), mix.length);
    }

    #[test]
    fn a_plan_the_engine_cannot_play_converts_to_an_error() {
        let mut plan = plugin_plan();
        plan.outgoing.speed[0] = 0.0;
        assert_eq!(planned_mix(&plan), Err(MixError::Value), "a stopped deck");

        let mut plan = plugin_plan();
        plan.bands.bass_hz = 0.0;
        assert_eq!(planned_mix(&plan), Err(MixError::Value), "no bass band");
    }
}
