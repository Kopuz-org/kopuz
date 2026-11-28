//! Smart crossfades: where a fade belongs between two tracks, and where it does not.

use std::time::Duration;

use reader::Track;

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
}
